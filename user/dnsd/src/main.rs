//! dnsd: systemovy preklad jmen pres sifrovane DNS s overenim DNSSEC.
//!
//! - Dotazy dostava od `netd` (kanal od init, `netproto::resolver`):
//!   jmeno a rodina adres, odpoved adresy + TTL, nebo NXDOMAIN, NODATA,
//!   UNAVAILABLE (zadny resolver neodpovedel - netd rozhodne o zaloze
//!   podle profilu site) a BOGUS (podpis DNSSEC neprosel).
//! - Ptá se resolveru pres DoH nebo DoT (`transport.rs`) s bity DO a CD
//!   a odpovedi overuje sam (`dns::dnssec`, retez od korenove kotvy IANA).
//!   Resolveru se neveri: lhat muze jen tak, ze zonu oznaci za
//!   nepodepsanou - a to dokazem z nadrazene zony neprojde.
//! - Mezipamet podle TTL (kladne odpovedi i neexistence), nejvys 1 den.
//!
//! Handly: 1. relace site (jen TCP na adresy resolveru), 2. kanal k netd,
//! 3. (volitelne) relace FS s ulozistem certifikatu `/certs`. Kontrola
//! odvolani certifikatu resolveru je jen OCSP stapling: revocd preklada
//! jmena pres dnsd (zavislost do kruhu).
//!
//! Konfigurace na bootstrap kanalu (TOML, `dnsd.toml`), odpoved `ok`;
//! potom `status`, `flush` (vyprazdnit mezipamet), `prefer <n>` (zacit
//! resolverem n).

#![no_std]
#![no_main]

extern crate alloc;

mod cache;
mod transport;

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use core::time::Duration;

use dns::dnssec::Ds;
use dns::dnssec::chain::{Fetch, Security, Validator};
use dns::message::rcode;
use dns::{Message, Name, rtype};
use fsproto::FsClient;
use libos::{Channel, Error, Signals, Startup, println, time};
use netproto::Net;
use netproto::resolver::{Answer, Query, status};

use cache::Cache;
use transport::{Server, Transport};

libos::entry!(main);

struct Config {
    servers: Vec<Server>,
    anchors: Vec<Ds>,
    dnssec: bool,
}

fn hex(s: &str) -> Option<Vec<u8>> {
    (s.len() % 2 == 0).then_some(())?;
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok()).collect()
}

/// `"tag alg typ otisk"` (jako zaznam DS v textu).
fn parse_anchor(s: &str) -> Option<Ds> {
    let mut p = s.split_whitespace();
    let key_tag = p.next()?.parse().ok()?;
    let algorithm = p.next()?.parse().ok()?;
    let digest_type = p.next()?.parse().ok()?;
    let digest = hex(p.next()?)?;
    p.next().is_none().then_some(Ds { key_tag, algorithm, digest_type, digest })
}

fn config(text: &str) -> Result<Config, String> {
    let t = conf::parse(text).map_err(|e| format!("{e}"))?;
    let mut servers = Vec::new();
    for s in t.get_strs("servers").ok_or("chybi servers")? {
        servers.push(Server::parse(s).ok_or_else(|| format!("neplatny server {s}"))?);
    }
    let mut anchors = Vec::new();
    for a in t.get_strs("trust_anchors").unwrap_or_default() {
        anchors.push(parse_anchor(a).ok_or_else(|| format!("neplatna kotva {a}"))?);
    }
    let dnssec = t.get_bool("dnssec").unwrap_or(true);
    if dnssec && anchors.is_empty() {
        return Err(String::from("DNSSEC bez korenove kotvy"));
    }
    Ok(Config { servers, anchors, dnssec })
}

struct Resolver<'a> {
    net: &'a Net,
    tls: &'a tls::Client,
    transports: Vec<Transport>,
    /// Posledni funkcni resolver (zkousi se prvni).
    preferred: usize,
    next_id: u16,
    stats: Stats,
}

#[derive(Default)]
struct Stats {
    queries: u64,
    upstream: u64,
    cache_hits: u64,
    secure: u64,
    insecure: u64,
    bogus: u64,
    unavailable: u64,
    last_server: String,
    last_error: String,
}

impl Resolver<'_> {
    /// Dotaz na resolvery (od posledniho funkcniho); `None` = nikdo.
    fn ask(&mut self, name: &Name, qtype: u16, dnssec: bool) -> Option<Message> {
        let n = self.transports.len();
        for k in 0..n {
            let i = (self.preferred + k) % n;
            self.next_id = self.next_id.wrapping_add(1);
            let id = if matches!(self.transports[i].server, Server::Doh { .. }) { 0 } else { self.next_id };
            let q = Message::query(id, name, qtype, dnssec);
            self.stats.upstream += 1;
            match self.transports[i].exchange(self.net, self.tls, &q.to_wire()) {
                Ok(bytes) => match Message::parse(&bytes) {
                    Ok(m) if m.id == id && m.questions.first().is_some_and(|x| x.name == *name && x.qtype == qtype) => {
                        if m.rcode() == rcode::SERVFAIL {
                            self.stats.last_error = format!("{}: SERVFAIL", self.transports[i].server.label());
                            continue;
                        }
                        self.preferred = i;
                        self.stats.last_server = self.transports[i].server.label();
                        return Some(m);
                    }
                    _ => self.stats.last_error = format!("{}: neplatna odpoved", self.transports[i].server.label()),
                },
                Err(e) => {
                    if self.stats.last_error != e {
                        println!("dnsd: {}: {e}", self.transports[i].server.label());
                    }
                    self.stats.last_error = e;
                }
            }
        }
        None
    }
}

impl Fetch for Resolver<'_> {
    fn query(&mut self, name: &Name, qtype: u16) -> Option<Message> {
        self.ask(name, qtype, true)
    }
}

/// Adresy typu `qtype` pro `qname` z odpovedi (po retezu CNAME).
fn addresses(m: &Message, qname: &Name, qtype: u16) -> (Vec<IpAddr>, u32) {
    let mut name = qname.clone();
    let mut ttl = u32::MAX;
    for _ in 0..16 {
        let Some(c) = m.answers.iter().find(|r| r.rtype == rtype::CNAME && r.name == name) else {
            break;
        };
        ttl = ttl.min(c.ttl);
        match c.target() {
            Some(t) => name = t,
            None => break,
        }
    }
    let mut out = Vec::new();
    for r in m.answers.iter().filter(|r| r.rtype == qtype && r.name == name) {
        ttl = ttl.min(r.ttl);
        match (qtype, r.rdata.len()) {
            (rtype::A, 4) => out.push(IpAddr::V4(Ipv4Addr::new(r.rdata[0], r.rdata[1], r.rdata[2], r.rdata[3]))),
            (rtype::AAAA, 16) => out.push(IpAddr::V6(Ipv6Addr::from(<[u8; 16]>::try_from(r.rdata.as_slice()).unwrap()))),
            _ => {}
        }
    }
    (out, if ttl == u32::MAX { 0 } else { ttl })
}

/// TTL zaporne odpovedi: minimum ze SOA (RFC 2308), nejvys 1 h.
fn negative_ttl(m: &Message) -> u32 {
    m.authority
        .iter()
        .find(|r| r.rtype == rtype::SOA)
        .map(|r| {
            let min = r.rdata.len().checked_sub(4).map_or(0, |i| u32::from_be_bytes(r.rdata[i..].try_into().unwrap()));
            min.min(r.ttl)
        })
        .unwrap_or(60)
        .min(3600)
}

fn now_secs() -> u32 {
    time::wall().map_or(0, |t| t.secs as u32)
}

struct Service<'a> {
    cfg: Config,
    resolver: Resolver<'a>,
    validator: Validator,
    cache: Cache,
}

impl Service<'_> {
    /// Jeden typ adres: (stav, adresy, TTL, overeno).
    fn lookup(&mut self, name: &Name, qtype: u16) -> (u8, Vec<IpAddr>, u32, bool) {
        let now = now_secs();
        if let Some(c) = self.cache.get(name, qtype, now) {
            self.resolver.stats.cache_hits += 1;
            return c;
        }
        let Some(m) = self.resolver.ask(name, qtype, self.cfg.dnssec) else {
            self.resolver.stats.unavailable += 1;
            return (status::UNAVAILABLE, Vec::new(), 0, false);
        };
        let secure = if self.cfg.dnssec {
            match self.validator.validate(&mut self.resolver, name, qtype, &m, now) {
                Security::Secure => {
                    self.resolver.stats.secure += 1;
                    true
                }
                Security::Insecure => {
                    self.resolver.stats.insecure += 1;
                    false
                }
                Security::Bogus(why) => {
                    self.resolver.stats.bogus += 1;
                    println!("dnsd: {}: DNSSEC neprosel ({why})", name.to_text());
                    // Podvrzena odpoved se do mezipameti nedava (dalsi
                    // dotaz to zkusi znovu, treba pres jiny resolver).
                    return (status::BOGUS, Vec::new(), 0, false);
                }
                Security::Indeterminate => {
                    self.resolver.stats.unavailable += 1;
                    return (status::UNAVAILABLE, Vec::new(), 0, false);
                }
            }
        } else {
            false
        };
        let result = if m.rcode() == rcode::NXDOMAIN {
            (status::NXDOMAIN, Vec::new(), negative_ttl(&m), secure)
        } else {
            let (addrs, ttl) = addresses(&m, name, qtype);
            if addrs.is_empty() {
                (status::NODATA, Vec::new(), negative_ttl(&m), secure)
            } else {
                (status::OK, addrs, ttl, secure)
            }
        };
        self.cache.put(name, qtype, &result, now);
        result
    }

    fn handle(&mut self, q: &Query) -> Answer {
        self.resolver.stats.queries += 1;
        let mut answer = Answer {
            id: q.id,
            status: status::NXDOMAIN,
            ttl: u32::MAX,
            addrs: Vec::new(),
            secure: true,
        };
        let Ok(name) = Name::parse(&q.name) else {
            answer.status = status::NXDOMAIN;
            answer.ttl = 0;
            return answer;
        };
        let types: &[u16] = match q.family {
            4 => &[rtype::A],
            6 => &[rtype::AAAA],
            _ => &[rtype::A, rtype::AAAA],
        };
        let mut statuses = Vec::new();
        for &t in types {
            let (st, addrs, ttl, secure) = self.lookup(&name, t);
            statuses.push(st);
            if st == status::OK {
                answer.addrs.extend(addrs);
                answer.ttl = answer.ttl.min(ttl);
                answer.secure &= secure;
            }
        }
        // Poradi: podvrzene > adresy > nedostupne > NODATA > NXDOMAIN.
        answer.status = if statuses.contains(&status::BOGUS) {
            status::BOGUS
        } else if !answer.addrs.is_empty() {
            status::OK
        } else if statuses.contains(&status::UNAVAILABLE) {
            status::UNAVAILABLE
        } else if statuses.contains(&status::NODATA) {
            status::NODATA
        } else {
            status::NXDOMAIN
        };
        if answer.status != status::OK {
            answer.addrs.clear();
            answer.secure = false;
        }
        if answer.ttl == u32::MAX {
            answer.ttl = 0;
        }
        answer
    }

    fn status(&self) -> String {
        let s = &self.resolver.stats;
        format!(
            "ok:queries={} upstream={} cache_hits={} secure={} insecure={} bogus={} unavailable={} dnssec={} server={} error={}",
            s.queries,
            s.upstream,
            s.cache_hits,
            s.secure,
            s.insecure,
            s.bogus,
            s.unavailable,
            self.cfg.dnssec as u8,
            if s.last_server.is_empty() { "-" } else { &s.last_server },
            if s.last_error.is_empty() { "-" } else { &s.last_error }
        )
    }
}

fn main(startup: Startup) -> i64 {
    let ctl = startup.bootstrap;
    let mut handles = startup.handles.into_iter();
    let (Some(net), Some(netd)) = (handles.next(), handles.next()) else {
        let _ = ctl.write(b"err:chybi handly", Vec::new());
        return 1;
    };
    let net = Net::new(Channel::from_handle(net));
    let netd = Channel::from_handle(netd);
    let fs = handles.next().map(|h| FsClient::new(Channel::from_handle(h)));
    let cfg = match ctl
        .read(Some(time::deadline_after(Duration::from_secs(10))))
        .map_err(|e| format!("{e}"))
        .and_then(|m| String::from_utf8(m.data).map_err(|_| String::from("neni UTF-8")))
        .and_then(|t| config(&t))
    {
        Ok(c) => c,
        Err(e) => {
            println!("dnsd: konfigurace: {e}");
            let _ = ctl.write(format!("err:{e}").as_bytes(), Vec::new());
            return 2;
        }
    };
    let roots = web::load_roots(fs.as_ref());
    let tls = match tls::Client::new(&tls::LIBOS, &roots, httpc::ALPN) {
        Ok(c) => c,
        Err(e) => {
            println!("dnsd: TLS: {e}");
            return 3;
        }
    };
    let _ = ctl.write(b"ok", Vec::new());
    println!(
        "dnsd: bezim, resolvery {:?}, DNSSEC {}, kotvy {}, CA {}",
        cfg.servers.iter().map(Server::label).collect::<Vec<_>>(),
        if cfg.dnssec { "ano" } else { "ne" },
        cfg.anchors.len(),
        roots.len()
    );
    let mut svc = Service {
        validator: Validator::new(cfg.anchors.clone()),
        resolver: Resolver {
            net: &net,
            tls: &tls,
            transports: cfg.servers.iter().cloned().map(Transport::new).collect(),
            preferred: 0,
            next_id: 0,
            stats: Stats::default(),
        },
        cache: Cache::default(),
        cfg,
    };
    loop {
        let items = [
            (ctl.handle(), Signals::READABLE | Signals::PEER_CLOSED),
            (netd.handle(), Signals::READABLE | Signals::PEER_CLOSED),
        ];
        let observed = match libos::wait_many(&items, None) {
            Ok((_, o)) => o,
            Err(e) => {
                println!("dnsd: cekani: {e}");
                return 4;
            }
        };
        if observed[1].contains(Signals::READABLE) {
            while let Ok(m) = netd.try_read() {
                let Ok(q) = Query::decode(&m.data) else {
                    continue;
                };
                let a = svc.handle(&q);
                if netd.write(&a.encode(), Vec::new()).is_err() {
                    return 0;
                }
            }
        } else if observed[1].contains(Signals::PEER_CLOSED) {
            println!("dnsd: netd skoncil");
            return 0;
        }
        if observed[0].contains(Signals::READABLE) {
            match ctl.try_read() {
                Ok(m) => {
                    let reply = if m.data == b"status" {
                        svc.status()
                    } else if m.data == b"flush" {
                        svc.cache = Cache::default();
                        String::from("ok")
                    } else if let Some(i) = m.data.strip_prefix(b"prefer ").and_then(|v| core::str::from_utf8(v).ok()?.parse::<usize>().ok()) {
                        // Zacit resolverem `i` (sprava, testy); mezipamet pryc.
                        if i < svc.resolver.transports.len() {
                            svc.resolver.preferred = i;
                            svc.cache = Cache::default();
                            String::from("ok")
                        } else {
                            String::from("err:neni takovy resolver")
                        }
                    } else {
                        String::from("err:neznamy prikaz")
                    };
                    let _ = ctl.write(reply.as_bytes(), Vec::new());
                }
                Err(Error::PeerClosed) => return 0,
                Err(_) => {}
            }
        } else if observed[0].contains(Signals::PEER_CLOSED) {
            return 0;
        }
    }
}
