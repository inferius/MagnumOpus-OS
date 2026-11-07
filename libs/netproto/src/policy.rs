//! Opravneni k siti: prava relace a omezeni cilu.
//!
//! Program dostane od init (pozdeji od spravce sezeni) kanal relace u sluzby
//! `netd`. Relace nese:
//!
//! - **prava** ([`rights`]) - co vubec smi (odchozi TCP, naslouchani, UDP,
//!   DNS, ping, ...),
//! - **vrstvy omezeni** ([`Policy`]) - kam smi. Kazde zuzeni relace
//!   (`Net::restrict`) prida vrstvu; cil musi projit **vsemi** vrstvami, takze
//!   odvozena relace nikdy nesmi vic nez ta, ze ktere vznikla.
//!
//! Pravidlo cile ([`Rule`]) urcuje protokoly, adresy (sit s prefixem nebo
//! jmeno hostitele) a rozsah portu. Pravidlo se jmenem (`api.example.com`,
//! `*.example.com`) plati pro adresy, ktere relace pro to jmeno dostala od
//! **systemoveho** resolveru ([`Auth::learn`]). Odpoved od serveru DNS, ktery
//! si program zvolil sam, se nepocita - jinak by si program mohl "prelozit"
//! povolene jmeno na libovolnou adresu.

use alloc::string::String;
use alloc::vec::Vec;
use core::net::{IpAddr, SocketAddr};

use abi::{Error, Result};

use crate::codec::{Reader, Writer};

/// Prava relace (bity).
pub mod rights {
    /// Stav rozhrani: adresa, brana, servery DNS (`Net::info`).
    pub const INFO: u32 = 1 << 0;
    /// Odchozi spojeni TCP.
    pub const TCP_CONNECT: u32 = 1 << 1;
    /// Naslouchani TCP (server).
    pub const TCP_LISTEN: u32 = 1 << 2;
    /// Sockety UDP (odesilani i prijem).
    pub const UDP: u32 = 1 << 3;
    /// Preklad jmen (DNS).
    pub const RESOLVE: u32 = 1 << 4;
    /// ICMP echo (ping).
    pub const PING: u32 = 1 << 5;
    /// Smi vytvaret uzsi relace pro jine programy (`Net::restrict`).
    pub const RESTRICT: u32 = 1 << 6;
    /// Zmena nastaveni rozhrani (zatim nepouzito; pozdeji staticka adresa,
    /// servery DNS).
    pub const CONFIGURE: u32 = 1 << 7;
    pub const ALL: u32 = (1 << 8) - 1;
    /// Bezny klient: odchozi TCP a UDP, DNS a stav rozhrani.
    pub const CLIENT: u32 = INFO | TCP_CONNECT | UDP | RESOLVE;
}

/// Protokoly v pravidle cile (bity).
pub mod proto {
    pub const TCP: u8 = 1 << 0;
    pub const UDP: u8 = 1 << 1;
    pub const ICMP: u8 = 1 << 2;
    pub const ALL: u8 = TCP | UDP | ICMP;
}

/// Nejvic pravidel v jedne vrstve a vrstev v relaci (ochrana sluzby pred
/// prilis velkym popisem).
pub const MAX_RULES: usize = 256;
pub const MAX_LAYERS: usize = 16;
/// Nejvic zapamatovanych prekladu jmen jedne relace.
pub const MAX_LEARNED: usize = 256;

/// Cil pravidla.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    /// Jakakoli adresa.
    Any,
    /// Sit: adresa a delka prefixu (`10.0.2.0/24`, `::/0`).
    Net { addr: IpAddr, prefix: u8 },
    /// Jmeno hostitele: presne (`api.example.com`) nebo vsechny poddomeny
    /// (`*.example.com`, bez samotneho `example.com`).
    Host(String),
}

/// Jedno pravidlo cile: protokoly, cil a porty `lo..=hi`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rule {
    pub protos: u8,
    pub target: Target,
    pub ports: (u16, u16),
}

impl Rule {
    /// Pravidlo pro vsechny porty.
    pub fn any_port(protos: u8, target: Target) -> Self {
        Self {
            protos,
            target,
            ports: (0, u16::MAX),
        }
    }

    /// Pravidlo pro jeden port.
    pub fn port(protos: u8, target: Target, port: u16) -> Self {
        Self {
            protos,
            target,
            ports: (port, port),
        }
    }

    fn matches(&self, proto: u8, dest: SocketAddr, learned: &[(String, IpAddr)]) -> bool {
        if self.protos & proto == 0 {
            return false;
        }
        // ICMP porty nema, plati jen adresa.
        if proto != proto::ICMP && !(self.ports.0..=self.ports.1).contains(&dest.port()) {
            return false;
        }
        match &self.target {
            Target::Any => true,
            Target::Net { addr, prefix } => in_network(dest.ip(), *addr, *prefix),
            Target::Host(pattern) => learned
                .iter()
                .any(|(name, ip)| *ip == dest.ip() && host_matches(pattern, name)),
        }
    }

    fn validate(&self) -> Result<()> {
        if self.protos == 0 || self.protos & !proto::ALL != 0 || self.ports.0 > self.ports.1 {
            return Err(Error::InvalidArgs);
        }
        match &self.target {
            Target::Any => Ok(()),
            Target::Net { addr, prefix } => {
                let max = if addr.is_ipv4() { 32 } else { 128 };
                if *prefix > max {
                    Err(Error::InvalidArgs)
                } else {
                    Ok(())
                }
            }
            Target::Host(p) => {
                let name = p.strip_prefix("*.").unwrap_or(p);
                if crate::dns::valid_name(name) {
                    Ok(())
                } else {
                    Err(Error::InvalidArgs)
                }
            }
        }
    }
}

/// Jedna vrstva omezeni. `None` = bez omezeni v teto vrstve.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Policy {
    /// Kam se smi pripojit / posilat (TCP, UDP, ICMP).
    pub connect: Option<Vec<Rule>>,
    /// Na kterych mistnich portech smi naslouchat (TCP) nebo prijimat na
    /// zvolenem portu (UDP): rozsahy `lo..=hi`. Port 0 (prideli sluzba) je
    /// povolen vzdy.
    pub bind: Option<Vec<(u16, u16)>>,
}

impl Policy {
    /// Vrstva bez omezeni.
    pub fn unrestricted() -> Self {
        Self::default()
    }

    /// Jen zadana pravidla cilu, naslouchani na zvolenem portu zakazano.
    pub fn only(rules: Vec<Rule>) -> Self {
        Self {
            connect: Some(rules),
            bind: Some(Vec::new()),
        }
    }

    pub fn validate(&self) -> Result<()> {
        if let Some(rules) = &self.connect {
            if rules.len() > MAX_RULES {
                return Err(Error::InvalidArgs);
            }
            rules.iter().try_for_each(Rule::validate)?;
        }
        if let Some(b) = &self.bind
            && (b.len() > MAX_RULES || b.iter().any(|(lo, hi)| lo > hi))
        {
            return Err(Error::InvalidArgs);
        }
        Ok(())
    }

    pub fn put(&self, w: &mut Writer) {
        match &self.connect {
            None => {
                w.u8(0);
            }
            Some(rules) => {
                w.u8(1).u32(rules.len() as u32);
                for r in rules {
                    w.u8(r.protos);
                    match &r.target {
                        Target::Any => {
                            w.u8(0);
                        }
                        Target::Net { addr, prefix } => {
                            w.u8(1).ip(*addr).u8(*prefix);
                        }
                        Target::Host(h) => {
                            w.u8(2).str(h);
                        }
                    }
                    w.u16(r.ports.0).u16(r.ports.1);
                }
            }
        }
        match &self.bind {
            None => {
                w.u8(0);
            }
            Some(ranges) => {
                w.u8(1).u32(ranges.len() as u32);
                for (lo, hi) in ranges {
                    w.u16(*lo).u16(*hi);
                }
            }
        }
    }

    pub fn get(r: &mut Reader) -> Result<Self> {
        let connect = match r.u8()? {
            0 => None,
            1 => {
                let n = r.u32()? as usize;
                if n > MAX_RULES {
                    return Err(Error::InvalidArgs);
                }
                let mut rules = Vec::with_capacity(n);
                for _ in 0..n {
                    let protos = r.u8()?;
                    let target = match r.u8()? {
                        0 => Target::Any,
                        1 => {
                            let addr = r.ip()?;
                            Target::Net {
                                addr,
                                prefix: r.u8()?,
                            }
                        }
                        2 => Target::Host(r.str()?),
                        _ => return Err(Error::InvalidArgs),
                    };
                    let lo = r.u16()?;
                    rules.push(Rule {
                        protos,
                        target,
                        ports: (lo, r.u16()?),
                    });
                }
                Some(rules)
            }
            _ => return Err(Error::InvalidArgs),
        };
        let bind = match r.u8()? {
            0 => None,
            1 => {
                let n = r.u32()? as usize;
                if n > MAX_RULES {
                    return Err(Error::InvalidArgs);
                }
                let mut v = Vec::with_capacity(n);
                for _ in 0..n {
                    let lo = r.u16()?;
                    v.push((lo, r.u16()?));
                }
                Some(v)
            }
            _ => return Err(Error::InvalidArgs),
        };
        let p = Self { connect, bind };
        p.validate()?;
        Ok(p)
    }
}

/// Opravneni jedne relace (drzi ho sluzba `netd`, klient ho nevidi).
#[derive(Debug, Clone, Default)]
pub struct Auth {
    pub rights: u32,
    layers: Vec<Policy>,
    /// Preklady jmen ze systemoveho resolveru: (jmeno, adresa).
    learned: Vec<(String, IpAddr)>,
}

impl Auth {
    /// Korenova relace (ma ji init): vsechna prava, bez omezeni.
    pub fn root() -> Self {
        Self {
            rights: rights::ALL,
            layers: Vec::new(),
            learned: Vec::new(),
        }
    }

    /// Uzsi relace: prava jen podmnozina, k vrstvam pribude `policy`.
    /// Zapamatovane preklady se zdedi.
    pub fn restrict(&self, rights: u32, policy: Policy) -> Result<Self> {
        policy.validate()?;
        if self.layers.len() >= MAX_LAYERS {
            return Err(Error::NoResources);
        }
        let mut layers = self.layers.clone();
        layers.push(policy);
        Ok(Self {
            rights: self.rights & rights,
            layers,
            learned: self.learned.clone(),
        })
    }

    pub fn has(&self, right: u32) -> bool {
        self.rights & right == right
    }

    /// Ma pravo `right`? Jinak `ACCESS_DENIED`.
    pub fn require(&self, right: u32) -> Result<()> {
        if self.has(right) {
            Ok(())
        } else {
            Err(Error::AccessDenied)
        }
    }

    /// Smi relace komunikovat s `dest` protokolem `proto` (jeden bit
    /// z [`proto`])?
    pub fn allows(&self, proto: u8, dest: SocketAddr) -> bool {
        self.layers.iter().all(|l| match &l.connect {
            None => true,
            Some(rules) => rules.iter().any(|r| r.matches(proto, dest, &self.learned)),
        })
    }

    /// Smi relace naslouchat / prijimat na mistnim portu `port`?
    pub fn allows_bind(&self, port: u16) -> bool {
        port == 0
            || self.layers.iter().all(|l| match &l.bind {
                None => true,
                Some(ranges) => ranges.iter().any(|(lo, hi)| (*lo..=*hi).contains(&port)),
            })
    }

    /// Zapamatuje si preklad jmena od systemoveho resolveru (pro pravidla
    /// se jmenem hostitele). Nejstarsi zaznamy nad limit se zahodi.
    pub fn learn(&mut self, name: &str, addrs: &[IpAddr]) {
        let name = normalize_name(name);
        for a in addrs {
            if self.learned.iter().any(|(n, ip)| *n == name && ip == a) {
                continue;
            }
            if self.learned.len() >= MAX_LEARNED {
                self.learned.remove(0);
            }
            self.learned.push((name.clone(), *a));
        }
    }

    /// Ma nektera vrstva pravidlo se jmenem hostitele? (Jen pak ma smysl
    /// preklady zapamatovat.)
    pub fn has_host_rules(&self) -> bool {
        self.layers.iter().any(|l| {
            l.connect
                .as_ref()
                .is_some_and(|r| r.iter().any(|r| matches!(r.target, Target::Host(_))))
        })
    }
}

/// Jmeno pro porovnani: mala pismena ASCII, bez tecky na konci.
pub fn normalize_name(name: &str) -> String {
    name.trim_end_matches('.').to_ascii_lowercase()
}

/// Odpovida jmeno vzoru? `*.example.com` = libovolna poddomena (ne samotne
/// `example.com`), jinak presna shoda. Bez ohledu na velikost pismen.
pub fn host_matches(pattern: &str, name: &str) -> bool {
    let pattern = normalize_name(pattern);
    let name = normalize_name(name);
    match pattern.strip_prefix("*.") {
        Some(suffix) => {
            name.len() > suffix.len() + 1
                && name.ends_with(suffix)
                && name.as_bytes()[name.len() - suffix.len() - 1] == b'.'
        }
        None => pattern == name,
    }
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

/// Textovy popis prav relace (pro logy), napr. `info,tcp,udp,dns`.
pub fn rights_names(r: u32) -> String {
    use rights::*;
    let names = [
        (INFO, "info"),
        (TCP_CONNECT, "tcp"),
        (TCP_LISTEN, "listen"),
        (UDP, "udp"),
        (RESOLVE, "dns"),
        (PING, "ping"),
        (RESTRICT, "restrict"),
        (CONFIGURE, "configure"),
    ];
    let mut s = String::new();
    for (bit, name) in names {
        if r & bit != 0 {
            if !s.is_empty() {
                s.push(',');
            }
            s.push_str(name);
        }
    }
    s
}
