//! Paketovy filtr sluzby `netd` (prichozi provoz).
//!
//! `netd` je jediny sitovy stack v systemu, takze filtr nejde obejit: kazdy
//! prijaty ramec projde [`Filter::inbound`] driv, nez ho dostane stack.
//! Odchozi provoz se kontroluje uz na rozhrani socketu (relace, blokovane
//! site), filtr k tomu nabizi [`Filter::blocked`].
//!
//! Pravidla podle **profilu site** (domaci / verejna / ...):
//! - **nova prichozi spojeni TCP** (SYN bez ACK) a datagramy UDP mimo
//!   sledovane toky: rezim `deny` (jen vyjimky `allow_in`) nebo `listeners`
//!   (i na porty, kde nekdo nasloucha),
//! - **ping** (ICMP echo) povolen / zakazan, ostatni ICMP (chyby, odpovedi)
//!   prochazi vzdy,
//! - **blokovane site**: nic dovnitr ani ven,
//! - **omezeni rychlosti** SYN a ICMP echo na zdroj i celkem (token bucket),
//! - **stealth**: filtrovany paket se zahodi bez odpovedi (jinak by stack
//!   na zavreny port odpovedel RST).
//!
//! Sledovani toku UDP: odpoved na datagram, ktery jsme poslali (mistni port
//! + vzdalena adresa a port), prochazi bez pravidla - tak funguje DNS, NTP
//! a dalsi klienti i ve verejne siti.

#![no_std]

extern crate alloc;

mod packet;
mod rate;

#[cfg(test)]
mod tests;

use alloc::collections::BTreeMap;
use alloc::vec::Vec;
use core::net::{IpAddr, Ipv4Addr};

pub use packet::{Packet, Proto, parse_frame};
pub use rate::Bucket;

/// Jak dlouho plati tok UDP po nasem poslednim datagramu.
pub const UDP_FLOW_NS: u64 = 120_000_000_000;
/// Nejvic sledovanych toku UDP (nejstarsi se zahodi).
pub const MAX_FLOWS: usize = 65_536;
/// Nejvic zdroju se samostatnym omezenim rychlosti.
pub const MAX_SOURCES: usize = 4096;

/// Rezim prichozich spojeni profilu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Inbound {
    /// Jen vyjimky `allow_in`.
    Deny,
    /// I porty, na kterych nekdo nasloucha (TCP) / prijima (UDP).
    Listeners,
}

/// Vyjimka pro prichozi provoz.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InRule {
    /// Bity [`Proto`] (TCP = 1, UDP = 2).
    pub protos: u8,
    /// Zdrojova sit (adresa, prefix).
    pub from: (IpAddr, u8),
    pub ports: (u16, u16),
}

/// Omezeni rychlosti: pakety za sekundu a davka.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rate {
    pub per_sec: u32,
    pub burst: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    pub inbound: Inbound,
    pub ping: bool,
    pub stealth: bool,
    pub allow_in: Vec<InRule>,
    pub blocked: Vec<(IpAddr, u8)>,
    pub syn: Rate,
    pub icmp: Rate,
}

impl Config {
    /// Bezpecne vychozi nastaveni (verejna sit), dokud fwd neposle profil.
    pub fn public() -> Self {
        Self {
            inbound: Inbound::Deny,
            ping: false,
            stealth: true,
            allow_in: Vec::new(),
            blocked: Vec::new(),
            syn: Rate {
                per_sec: 100,
                burst: 200,
            },
            icmp: Rate {
                per_sec: 10,
                burst: 20,
            },
        }
    }
}

/// Proc byl paket zahozen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Reason {
    /// Zdroj v blokovane siti.
    Blocked,
    /// Nove prichozi spojeni / datagram bez pravidla.
    Inbound,
    /// Ping zakazany profilem.
    Ping,
    /// Prilis mnoho SYN.
    RateSyn,
    /// Prilis mnoho ICMP echo.
    RateIcmp,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    Pass,
    /// Zahodit (zapocitano v [`Filter::dropped`]).
    Drop { reason: Reason },
}

/// Co filtr potrebuje vedet o stacku.
pub trait Ports {
    /// Nasloucha nekdo na mistnim portu TCP?
    fn tcp_listening(&self, port: u16) -> bool;
    /// Prijima nekdo na mistnim portu UDP?
    fn udp_bound(&self, port: u16) -> bool;
}

pub struct Filter {
    cfg: Config,
    /// (mistni port, vzdalena adresa, vzdaleny port) -> konec platnosti.
    flows: BTreeMap<(u16, Ipv4Addr, u16), u64>,
    sources: Vec<(Ipv4Addr, Bucket, Bucket)>,
    global_syn: Bucket,
    global_icmp: Bucket,
    /// Pocty zahozenych paketu podle duvodu (pro audit).
    pub dropped: [u64; 5],
}

impl Filter {
    pub fn new(cfg: Config) -> Self {
        let (gs, gi) = global_buckets(&cfg);
        Self {
            cfg,
            flows: BTreeMap::new(),
            sources: Vec::new(),
            global_syn: gs,
            global_icmp: gi,
            dropped: [0; 5],
        }
    }

    pub fn config(&self) -> &Config {
        &self.cfg
    }

    /// Nova konfigurace (zmena profilu); sledovane toky zustavaji.
    pub fn set_config(&mut self, cfg: Config) {
        let (gs, gi) = global_buckets(&cfg);
        self.global_syn = gs;
        self.global_icmp = gi;
        self.sources.clear();
        self.cfg = cfg;
    }

    /// Je adresa v blokovane siti?
    pub fn blocked(&self, ip: IpAddr) -> bool {
        self.cfg
            .blocked
            .iter()
            .any(|(net, p)| in_network(ip, *net, *p))
    }

    /// Zaznamena odchozi datagram UDP (odpovedi pak projdou).
    pub fn note_udp_out(&mut self, local_port: u16, remote: Ipv4Addr, remote_port: u16, now: u64) {
        if self.flows.len() >= MAX_FLOWS {
            self.flows.retain(|_, exp| *exp > now);
        }
        if self.flows.len() >= MAX_FLOWS
            && let Some(k) = self
                .flows
                .iter()
                .min_by_key(|(_, e)| **e)
                .map(|(k, _)| *k)
        {
            self.flows.remove(&k);
        }
        self.flows
            .insert((local_port, remote, remote_port), now.saturating_add(UDP_FLOW_NS));
    }

    /// Zapomene toky mistniho portu (socket zavren).
    pub fn forget_udp_port(&mut self, local_port: u16) {
        self.flows.retain(|(p, _, _), _| *p != local_port);
    }

    fn drop(&mut self, reason: Reason) -> Decision {
        self.dropped[reason as usize] += 1;
        Decision::Drop { reason }
    }

    fn allowed_in(&self, proto: u8, src: Ipv4Addr, port: u16) -> bool {
        self.cfg.allow_in.iter().any(|r| {
            r.protos & proto != 0
                && (r.ports.0..=r.ports.1).contains(&port)
                && in_network(IpAddr::V4(src), r.from.0, r.from.1)
        })
    }

    /// Posoudi prijaty paket.
    pub fn inbound(&mut self, p: &Packet, now: u64, ports: &dyn Ports) -> Decision {
        let Some(src) = p.src else {
            // Neni IPv4 (ARP, ...): filtr se ho netyka.
            return Decision::Pass;
        };
        if self.blocked(IpAddr::V4(src)) {
            return self.drop(Reason::Blocked);
        }
        match p.proto {
            Proto::Tcp { syn: true, ack: false, .. } => {
                if !self.take_rate(src, now, true) {
                    return self.drop(Reason::RateSyn);
                }
                let listening = ports.tcp_listening(p.dport);
                let open = self.allowed_in(Proto::TCP_BIT, src, p.dport)
                    || (self.cfg.inbound == Inbound::Listeners && listening);
                if open {
                    Decision::Pass
                } else {
                    self.reject(listening)
                }
            }
            Proto::Tcp { .. } => Decision::Pass,
            Proto::Udp => {
                // Odpoved serveru DHCP (klient na 68).
                if p.dport == 68 && p.sport == 67 {
                    return Decision::Pass;
                }
                if self
                    .flows
                    .get(&(p.dport, src, p.sport))
                    .is_some_and(|exp| *exp > now)
                {
                    return Decision::Pass;
                }
                let bound = ports.udp_bound(p.dport);
                let open = self.allowed_in(Proto::UDP_BIT, src, p.dport)
                    || (self.cfg.inbound == Inbound::Listeners && bound);
                if open {
                    Decision::Pass
                } else {
                    self.reject(bound)
                }
            }
            Proto::IcmpEcho => {
                if !self.cfg.ping {
                    return self.drop(Reason::Ping);
                }
                if !self.take_rate(src, now, false) {
                    return self.drop(Reason::RateIcmp);
                }
                Decision::Pass
            }
            Proto::Icmp | Proto::Fragment | Proto::Other => Decision::Pass,
        }
    }

    /// Filtrovany paket se zapocita a zahodi. Bez stealth ho na port, kde
    /// nikdo neposloucha, dostane stack - ten odpovi jako na zavreny port
    /// (RST). Na port s posluchacem se nikdy nepusti (spojeni by vzniklo).
    fn reject(&mut self, has_socket: bool) -> Decision {
        let d = self.drop(Reason::Inbound);
        if self.cfg.stealth || has_socket { d } else { Decision::Pass }
    }

    fn take_rate(&mut self, src: Ipv4Addr, now: u64, syn: bool) -> bool {
        let global = if syn {
            &mut self.global_syn
        } else {
            &mut self.global_icmp
        };
        if !global.take(now) {
            return false;
        }
        let i = match self.sources.iter().position(|(ip, _, _)| *ip == src) {
            Some(i) => i,
            None => {
                if self.sources.len() >= MAX_SOURCES {
                    // Uvolnit zdroj, ktery nejdele nic neposlal.
                    if let Some(old) = (0..self.sources.len()).min_by_key(|&i| {
                        self.sources[i].1.last.max(self.sources[i].2.last)
                    }) {
                        self.sources.swap_remove(old);
                    }
                }
                self.sources.push((
                    src,
                    Bucket::new(self.cfg.syn, now),
                    Bucket::new(self.cfg.icmp, now),
                ));
                self.sources.len() - 1
            }
        };
        let s = &mut self.sources[i];
        if syn { s.1.take(now) } else { s.2.take(now) }
    }
}

/// Celkove limity: desetinasobek limitu na zdroj.
fn global_buckets(cfg: &Config) -> (Bucket, Bucket) {
    let scale = |r: Rate| Rate {
        per_sec: r.per_sec.saturating_mul(10),
        burst: r.burst.saturating_mul(10),
    };
    (Bucket::new(scale(cfg.syn), 0), Bucket::new(scale(cfg.icmp), 0))
}

/// Patri `ip` do site `net/prefix`? Ruzne rodiny adres nikdy.
pub fn in_network(ip: IpAddr, net: IpAddr, prefix: u8) -> bool {
    match (ip, net) {
        (IpAddr::V4(a), IpAddr::V4(n)) => {
            let p = prefix.min(32) as u32;
            let mask = if p == 0 { 0 } else { u32::MAX << (32 - p) };
            u32::from(a) & mask == u32::from(n) & mask
        }
        (IpAddr::V6(a), IpAddr::V6(n)) => {
            let p = prefix.min(128) as u32;
            let mask = if p == 0 { 0 } else { u128::MAX << (128 - p) };
            u128::from(a) & mask == u128::from(n) & mask
        }
        _ => false,
    }
}
