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

use crate::codec::{ReadAddr, Reader, WriteAddr, Writer};

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
    /// Zmena nastaveni rozhrani (servery DNS, pozdeji staticka adresa).
    pub const CONFIGURE: u32 = 1 << 7;
    /// Cteni auditu firewallu a provozu aplikaci (`Net::audit`, `stats`).
    pub const AUDIT: u32 = 1 << 8;
    pub const ALL: u32 = (1 << 9) - 1;
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

/// Nejvic pravidel v jedne vrstve: popis vrstvy jde v jedne zprave kanalu
/// (64 KiB; pravidlo se jmenem az ~260 B). Na vic cilu jsou site a vzory
/// jmen (`*.example.com`).
pub const MAX_RULES: usize = 256;
/// Nejvic vrstev relace (kazde zuzeni jednu; hloubka delegace, ne pocet
/// pravidel).
pub const MAX_LAYERS: usize = 16;

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
    pub fn put(&self, w: &mut Writer) {
        w.u8(self.protos);
        match &self.target {
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
        w.u16(self.ports.0).u16(self.ports.1);
    }

    /// Precte pravidlo (bez kontroly, viz [`Rule::validate`]).
    pub fn get(r: &mut Reader) -> Result<Self> {
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
        Ok(Rule {
            protos,
            target,
            ports: (lo, r.u16()?),
        })
    }

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

    /// Odpovida cil pravidlu? `host(vzor, adresa)` = patri adresa jmenu
    /// podle vzoru (preklady ze systemoveho resolveru).
    pub fn matches_with(&self, proto: u8, dest: SocketAddr, host: impl Fn(&str, IpAddr) -> bool) -> bool {
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
            Target::Host(pattern) => host(pattern, dest.ip()),
        }
    }

    pub fn validate(&self) -> Result<()> {
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

/// Odkud smi relace prijimat spojeni (naslouchani TCP, UDP na zvolenem
/// portu).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(u8)]
pub enum BindScope {
    /// Sit i loopback.
    #[default]
    Any = 0,
    /// Jen loopback (127.0.0.0/8, `::1`); navazani na sit (`0.0.0.0`,
    /// adresa rozhrani) = `ACCESS_DENIED`, nikdy tiche zuzeni.
    Loopback = 1,
}

impl BindScope {
    pub fn from_u8(v: u8) -> Result<Self> {
        match v {
            0 => Ok(Self::Any),
            1 => Ok(Self::Loopback),
            _ => Err(Error::InvalidArgs),
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
    /// Odkud smi prijimat (sit, nebo jen loopback).
    pub bind_scope: BindScope,
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
            bind_scope: BindScope::Any,
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
                    r.put(w);
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
        w.u8(self.bind_scope as u8);
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
                    rules.push(Rule::get(r)?);
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
        let bind_scope = BindScope::from_u8(r.u8()?)?;
        let p = Self { connect, bind, bind_scope };
        p.validate()?;
        Ok(p)
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
        (AUDIT, "audit"),
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
