//! Firewall ve sluzbe: kontrola cilu (relace + rozhodnuti uzivatele +
//! blokovane site), dotazy pres `fwd`, prichozi filtr, filtr DNS, audit
//! a provoz podle aplikaci.
//!
//! Konfiguraci posila `fwd` ([`netproto::fw::op::CONFIG`]). Bez `fwd` (nebo
//! nez prvni konfigurace prijde) plati bezpecne vychozi nastaveni: verejna
//! sit (nic dovnitr) a dotazy aplikaci se odmitnou.

use alloc::collections::{BTreeMap, VecDeque};
use alloc::string::String;
use alloc::vec::Vec;
use core::net::{IpAddr, Ipv4Addr, SocketAddr};

use libos::{Channel, Error, println};
use netfilter::{Config, Decision, Filter, InRule, Inbound, Ports, Rate, Reason};
use netproto::fw::{self, AppStats, Ask, AuditRecord, FwConfig, audit};
use netproto::{Auth, Reader, UserRules, Verdict, WriteAddr, Writer};

/// Jak dlouho netd ceka na odpoved `fwd` (fwd sam odpovi driv - po svem
/// casovem limitu dotazu - tohle je jen pojistka).
pub const ASK_WAIT_NS: u64 = 300_000_000_000;
/// Stejny zahozeny zdroj a duvod se do auditu zapise nejvys jednou za 10 s.
const DROP_LOG_NS: u64 = 10_000_000_000;
const DROP_LOG_MAX: usize = 256;

pub struct Firewall {
    pub filter: Filter,
    pub user: UserRules,
    dns_block: Vec<String>,
    pub profile: String,
    audit: VecDeque<AuditRecord>,
    audit_capacity: usize,
    next_seq: u64,
    stats: BTreeMap<String, AppStats>,
    ch: Option<Channel>,
    next_id: u32,
    answers: BTreeMap<u32, bool>,
    /// Zrusene dotazy, na ktere jeste muze prijit odpoved.
    cancelled: Vec<u32>,
    /// Brana a jeji MAC (z ARP), posledni ohlasena `fwd`.
    gateway_mac: Option<(Ipv4Addr, [u8; 6])>,
    announced: Option<(Ipv4Addr, [u8; 6])>,
    drop_log: BTreeMap<(Ipv4Addr, u8), u64>,
}

/// Jmeno aplikace pro audit a statistiky (relace bez identity = system).
pub fn app_name(auth: &Auth) -> String {
    match auth.app() {
        "" => String::from("-"),
        a => String::from(a),
    }
}

fn reason_text(r: Reason) -> &'static str {
    match r {
        Reason::Blocked => "blokovana sit",
        Reason::Inbound => "prichozi bez pravidla",
        Reason::Ping => "ping zakazan",
        Reason::RateSyn => "prilis mnoho SYN",
        Reason::RateIcmp => "prilis mnoho ICMP",
    }
}

/// Konfigurace filtru z konfigurace `fwd`.
fn filter_config(c: &FwConfig) -> Config {
    Config {
        inbound: if c.inbound == fw::inbound::LISTENERS {
            Inbound::Listeners
        } else {
            Inbound::Deny
        },
        ping: c.ping,
        stealth: c.stealth,
        allow_in: c
            .allow_in
            .iter()
            .map(|r| InRule {
                protos: r.protos,
                from: r.from,
                ports: r.ports,
            })
            .collect(),
        blocked: c.blocked.clone(),
        syn: Rate {
            per_sec: c.syn_rate.0,
            burst: c.syn_rate.1,
        },
        icmp: Rate {
            per_sec: c.icmp_rate.0,
            burst: c.icmp_rate.1,
        },
    }
}

impl Firewall {
    pub fn new(ch: Option<Channel>) -> Self {
        let d = FwConfig::default();
        Self {
            filter: Filter::new(filter_config(&d)),
            user: UserRules::default(),
            dns_block: Vec::new(),
            profile: d.profile,
            audit: VecDeque::new(),
            audit_capacity: d.audit_capacity as usize,
            next_seq: 1,
            stats: BTreeMap::new(),
            ch,
            next_id: 1,
            answers: BTreeMap::new(),
            cancelled: Vec::new(),
            gateway_mac: None,
            announced: None,
            drop_log: BTreeMap::new(),
        }
    }

    pub fn channel(&self) -> Option<&Channel> {
        self.ch.as_ref()
    }

    fn apply(&mut self, c: FwConfig) {
        if c.profile != self.profile {
            println!("netd: profil site {}", c.profile);
            self.record(audit::PROFILE, "-", 0, None, 0, &c.profile);
        }
        self.filter.set_config(filter_config(&c));
        self.user = UserRules {
            rules: c.user_rules,
        };
        self.dns_block = c.dns_block;
        self.profile = c.profile;
        self.audit_capacity = (c.audit_capacity as usize).max(16);
        while self.audit.len() > self.audit_capacity {
            self.audit.pop_front();
        }
    }

    /// Precte zpravy od `fwd`. `false` = kanal zavren (dal bez `fwd`).
    pub fn pump(&mut self) -> bool {
        loop {
            let Some(ch) = &self.ch else {
                return false;
            };
            let msg = match ch.try_read() {
                Ok(m) => m,
                Err(Error::ShouldWait) => return true,
                Err(_) => {
                    println!("netd: sluzba firewallu skoncila, dotazy se odmitaji");
                    self.ch = None;
                    // Rozpracovane dotazy: odmitnout.
                    return false;
                }
            };
            let mut r = Reader::new(&msg.data);
            match r.u32() {
                Ok(fw::op::ANSWER) => {
                    if let (Ok(id), Ok(allow)) = (r.u32(), r.u8()) {
                        if let Some(i) = self.cancelled.iter().position(|&c| c == id) {
                            self.cancelled.swap_remove(i);
                        } else {
                            self.answers.insert(id, allow != 0);
                        }
                    }
                }
                Ok(fw::op::CONFIG) => match FwConfig::get(&mut r) {
                    Ok(c) => self.apply(c),
                    Err(e) => println!("netd: neplatna konfigurace firewallu: {e}"),
                },
                _ => {}
            }
        }
    }

    /// Kontrola cile: blokovane site, relace a rozhodnuti uzivatele.
    /// Zapise audit povoleni a zakazu.
    pub fn check(
        &mut self,
        auth: &Auth,
        proto: u8,
        dest: SocketAddr,
        now: u64,
        once: &[String],
    ) -> Verdict {
        let app = app_name(auth);
        if self.filter.blocked(dest.ip()) {
            self.record(audit::BLOCKED_NET, &app, proto, Some(dest), 0, "");
            return Verdict::Deny;
        }
        let v = auth.check(proto, dest, now, &self.user, once);
        match &v {
            Verdict::Allow => {
                let kind = if !once.is_empty() {
                    audit::ASK_ALLOWED
                } else {
                    audit::ALLOWED
                };
                self.record(kind, &app, proto, Some(dest), 0, "");
            }
            Verdict::Deny => self.record(audit::DENIED, &app, proto, Some(dest), 0, ""),
            Verdict::Ask { .. } => {}
        }
        v
    }

    /// Posle dotaz `fwd`; `None` = neni komu (dotaz se odmitne).
    pub fn ask(&mut self, app: &str, proto: u8, dest: SocketAddr, host: &str) -> Option<u32> {
        let ch = self.ch.as_ref()?;
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1).max(1);
        let msg = Ask {
            id,
            app: String::from(app),
            proto,
            dest,
            host: String::from(host),
        };
        ch.write(&msg.encode(), Vec::new()).ok()?;
        Some(id)
    }

    /// Odpoved na dotaz `id`, pokud uz prisla. Bez `fwd` = zamitnuto.
    pub fn take_answer(&mut self, id: u32) -> Option<bool> {
        if self.ch.is_none() {
            return Some(self.answers.remove(&id).unwrap_or(false));
        }
        self.answers.remove(&id)
    }

    /// Dotaz zrusen (klient zavrel kanal): pozdni odpoved zahodit.
    pub fn forget_answer(&mut self, id: u32) {
        self.answers.remove(&id);
        self.cancelled.push(id);
        if self.cancelled.len() > 1024 {
            self.cancelled.remove(0);
        }
    }

    /// Zapise rozhodnuti dotazu do auditu.
    pub fn record_answer(&mut self, app: &str, proto: u8, dest: SocketAddr, allowed: bool) {
        if !allowed {
            self.record(audit::ASK_DENIED, app, proto, Some(dest), 0, "");
        }
    }

    /// Je jmeno na seznamu blokovanych domen? Vzor `ads.example` blokuje
    /// jmeno i jeho poddomeny, `*.ads.example` jen poddomeny.
    pub fn dns_blocked(&self, name: &str) -> bool {
        use netproto::policy::host_matches;
        self.dns_block.iter().any(|p| {
            host_matches(p, name)
                || (!p.starts_with("*.") && host_matches(&alloc::format!("*.{p}"), name))
        })
    }

    /// Prichozi ramec: projde filtrem? Zaroven se uci MAC brany (ARP).
    pub fn inbound(&mut self, frame: &[u8], ports: &dyn Ports, gateway: Option<Ipv4Addr>, now: u64) -> bool {
        if let Some(gw) = gateway {
            self.learn_gateway(frame, gw);
        }
        let p = netfilter::parse_frame(frame);
        match self.filter.inbound(&p, now, ports) {
            Decision::Pass => true,
            Decision::Drop { reason } => {
                if let Some(src) = p.src {
                    self.log_drop(src, p.dport, reason, now);
                }
                false
            }
        }
    }

    fn log_drop(&mut self, src: Ipv4Addr, port: u16, reason: Reason, now: u64) {
        let key = (src, reason as u8);
        if self
            .drop_log
            .get(&key)
            .is_some_and(|t| now < t.saturating_add(DROP_LOG_NS))
        {
            return;
        }
        if self.drop_log.len() >= DROP_LOG_MAX {
            self.drop_log.retain(|_, t| now < t.saturating_add(DROP_LOG_NS));
            if self.drop_log.len() >= DROP_LOG_MAX {
                self.drop_log.clear();
            }
        }
        self.drop_log.insert(key, now);
        self.record(
            audit::INBOUND_DROP,
            "-",
            0,
            Some(SocketAddr::new(IpAddr::V4(src), 0)),
            port,
            reason_text(reason),
        );
    }

    /// ARP odpoved od brany: jeji MAC identifikuje sit (profil).
    fn learn_gateway(&mut self, f: &[u8], gw: Ipv4Addr) {
        // Ethernet + ARP: typ 0x0806, operace 2 (odpoved), odesilatel MAC 22..28, IP 28..32.
        if f.len() < 42 || f[12..14] != [0x08, 0x06] || f[20..22] != [0, 2] {
            return;
        }
        if f[28..32] != gw.octets() {
            return;
        }
        let mut mac = [0u8; 6];
        mac.copy_from_slice(&f[22..28]);
        self.gateway_mac = Some((gw, mac));
    }

    /// Ohlasi `fwd` pripojenou sit (brana + MAC), kdyz se zmenila.
    pub fn announce_network(&mut self) {
        let (Some(cur), Some(ch)) = (self.gateway_mac, &self.ch) else {
            return;
        };
        if self.announced == Some(cur) {
            return;
        }
        let mut w = Writer::op(fw::op::NETWORK);
        w.ip(IpAddr::V4(cur.0)).bytes(&cur.1);
        if ch.write(&w.finish(), Vec::new()).is_ok() {
            self.announced = Some(cur);
        }
    }

    /// Sit se odpojila (DHCP): pri dalsim pripojeni ohlasit znovu.
    pub fn network_lost(&mut self) {
        self.gateway_mac = None;
        self.announced = None;
    }

    pub fn record(
        &mut self,
        kind: u8,
        app: &str,
        proto: u8,
        remote: Option<SocketAddr>,
        local_port: u16,
        detail: &str,
    ) {
        if self.audit.len() >= self.audit_capacity {
            self.audit.pop_front();
        }
        self.audit.push_back(AuditRecord {
            seq: self.next_seq,
            time: crate::stack::now_ns(),
            kind,
            app: String::from(app),
            proto,
            remote,
            local_port,
            detail: String::from(detail),
        });
        self.next_seq += 1;
    }

    /// Zaznamy od `from` (nejvys `max`) a dalsi poradove cislo.
    pub fn audit_read(&self, from: u64, max: usize, w: &mut Writer) {
        let recs: Vec<&AuditRecord> = self
            .audit
            .iter()
            .filter(|r| r.seq >= from)
            .take(max)
            .collect();
        let next = recs.last().map_or(from.max(self.oldest_seq()), |r| r.seq + 1);
        w.u64(next).u16(recs.len() as u16);
        for r in recs {
            r.put(w);
        }
    }

    fn oldest_seq(&self) -> u64 {
        self.audit.front().map_or(self.next_seq, |r| r.seq)
    }

    fn app_stats(&mut self, app: &str) -> &mut AppStats {
        if !self.stats.contains_key(app) {
            self.stats.insert(
                String::from(app),
                AppStats {
                    app: String::from(app),
                    ..AppStats::default()
                },
            );
        }
        self.stats.get_mut(app).unwrap()
    }

    pub fn socket_opened(&mut self, app: &str) {
        self.app_stats(app).sockets += 1;
    }

    pub fn traffic(&mut self, app: &str, sent: usize, received: usize) {
        if sent == 0 && received == 0 {
            return;
        }
        let s = self.app_stats(app);
        s.sent += sent as u64;
        s.received += received as u64;
    }

    pub fn stats(&self, w: &mut Writer) {
        w.str(&self.profile).u16(self.stats.len().min(u16::MAX as usize) as u16);
        for s in self.stats.values().take(u16::MAX as usize) {
            s.put(w);
        }
    }
}
