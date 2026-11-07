//! Firewall: zpravy mezi `netd` a sluzbou firewallu `fwd`, mezi `fwd`
//! a agentem uzivatele (UI) a zaznamy auditu.
//!
//! **netd <-> fwd** (kanal od init, bez odpovedi - kazda zprava je
//! samostatna):
//! - netd -> fwd: [`op::ASK`] (aplikace chce cil mimo sva pravidla),
//!   [`op::NETWORK`] (pripojena sit: brana a jeji MAC),
//! - fwd -> netd: [`op::ANSWER`] (rozhodnuti dotazu), [`op::CONFIG`]
//!   (cela konfigurace: profil, filtr, blokovane domeny, rozhodnuti).
//!
//! **fwd <-> agent** (UI, pozdeji spravce sezeni): fwd posila dotazy
//! [`op::QUESTION`] a [`op::NETWORK_QUESTION`], agent odpovida
//! [`op::DECIDE`] a [`op::CLASSIFY`]. Agent muze poslat pozadavky
//! [`op::CONFIG_GET`], [`op::CONFIG_SET`], [`op::PROFILE_SET`]; fwd na ne
//! odpovi formatem [`crate::reply`]. Odpoved zacina statusem <= 0,
//! zprava fwd kodem operace > 0 - agent je tak rozlisi.
//!
//! Audit: `netd` drzi kruhovy seznam [`AuditRecord`] (`AUDIT` na relaci
//! s pravem `rights::AUDIT`) a pocty bajtu podle aplikaci (`STATS`).

use alloc::string::String;
use alloc::vec::Vec;
use core::net::{IpAddr, SocketAddr};

use abi::{Error, Result};

use crate::codec::{ReadAddr, Reader, WriteAddr, Writer};
use crate::policy::{MAX_RULES, Rule};

/// Kody zprav firewallu.
pub mod op {
    /// netd -> fwd: id u32, aplikace str, protokol u8, cil socket_addr,
    /// jmeno hostitele str (prazdne = nezname)
    pub const ASK: u32 = 100;
    /// netd -> fwd: brana ip, MAC brany [6]
    pub const NETWORK: u32 = 101;
    /// fwd -> netd: id u32, povolit u8 (0/1, jen tento pozadavek)
    pub const ANSWER: u32 = 110;
    /// fwd -> netd: [`super::FwConfig`]
    pub const CONFIG: u32 = 111;

    /// fwd -> agent: id u32, aplikace str, jmeno aplikace str, protokol u8,
    /// cil socket_addr, jmeno hostitele str
    pub const QUESTION: u32 = 120;
    /// agent -> fwd: id u32, volba u8 ([`super::choice`])
    pub const DECIDE: u32 = 121;
    /// fwd -> agent: id u32, id site str, brana str
    pub const NETWORK_QUESTION: u32 = 122;
    /// agent -> fwd: id u32, profil str
    pub const CLASSIFY: u32 = 123;
    /// agent -> fwd: () -> konfigurace TOML str (bez limitu delky: data)
    pub const CONFIG_GET: u32 = 130;
    /// agent -> fwd: konfigurace TOML (data) -> ()
    pub const CONFIG_SET: u32 = 131;
    /// agent -> fwd: profil str -> () (profil aktualni site)
    pub const PROFILE_SET: u32 = 132;
}

/// Volba uzivatele v dotazu.
pub mod choice {
    pub const DENY_ONCE: u8 = 0;
    pub const ALLOW_ONCE: u8 = 1;
    /// Povolit vzdy (fwd ulozi pravidlo pro aplikaci).
    pub const ALLOW_ALWAYS: u8 = 2;
    /// Zakazat vzdy (pristi pozadavky bez dotazu).
    pub const DENY_ALWAYS: u8 = 3;
}

/// Rezim prichozich spojeni profilu.
pub mod inbound {
    pub const DENY: u8 = 0;
    pub const LISTENERS: u8 = 1;
}

/// Vyjimka pro prichozi provoz.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InRule {
    /// `proto::TCP | proto::UDP`
    pub protos: u8,
    pub from: (IpAddr, u8),
    pub ports: (u16, u16),
}

/// Konfigurace, kterou fwd posila netd.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FwConfig {
    /// Jmeno aktivniho profilu (pro audit a INFO).
    pub profile: String,
    /// [`inbound`]
    pub inbound: u8,
    pub ping: bool,
    pub stealth: bool,
    pub allow_in: Vec<InRule>,
    pub blocked: Vec<(IpAddr, u8)>,
    /// (za sekundu, davka)
    pub syn_rate: (u32, u32),
    pub icmp_rate: (u32, u32),
    /// Vzory blokovanych jmen (`ads.example`, `*.tracker.example`).
    pub dns_block: Vec<String>,
    /// Trvala rozhodnuti uzivatele: (aplikace, pravidlo, povolit).
    pub user_rules: Vec<(String, Rule, bool)>,
    /// Kapacita auditu (zaznamy).
    pub audit_capacity: u32,
    /// Smi se v teto siti pouzit neoverene DNS a NTP, kdyz overene
    /// (DoH/DoT, NTS) nefunguje? Verejna sit: ne.
    pub unauthenticated_fallback: bool,
}

impl Default for FwConfig {
    /// Verejna sit: nic dovnitr, bez pingu.
    fn default() -> Self {
        Self {
            profile: String::from("public"),
            inbound: inbound::DENY,
            ping: false,
            stealth: true,
            allow_in: Vec::new(),
            blocked: Vec::new(),
            syn_rate: (100, 200),
            icmp_rate: (10, 20),
            dns_block: Vec::new(),
            user_rules: Vec::new(),
            audit_capacity: 4096,
            unauthenticated_fallback: false,
        }
    }
}

/// Nejvic polozek v kazdem seznamu konfigurace (zprava kanalu = 64 KiB).
pub const MAX_ITEMS: usize = 1024;

fn len(n: usize) -> Result<usize> {
    if n > MAX_ITEMS {
        Err(Error::InvalidArgs)
    } else {
        Ok(n)
    }
}

impl FwConfig {
    pub fn put(&self, w: &mut Writer) {
        w.str(&self.profile)
            .u8(self.inbound)
            .u8(self.ping as u8)
            .u8(self.stealth as u8);
        w.u32(self.allow_in.len() as u32);
        for r in &self.allow_in {
            w.u8(r.protos).ip(r.from.0).u8(r.from.1).u16(r.ports.0).u16(r.ports.1);
        }
        w.u32(self.blocked.len() as u32);
        for (ip, p) in &self.blocked {
            w.ip(*ip).u8(*p);
        }
        w.u32(self.syn_rate.0).u32(self.syn_rate.1);
        w.u32(self.icmp_rate.0).u32(self.icmp_rate.1);
        w.u32(self.dns_block.len() as u32);
        for d in &self.dns_block {
            w.str(d);
        }
        w.u32(self.user_rules.len() as u32);
        for (app, rule, allow) in &self.user_rules {
            w.str(app).u8(*allow as u8);
            rule.put(w);
        }
        w.u32(self.audit_capacity);
        w.u8(self.unauthenticated_fallback as u8);
    }

    pub fn get(r: &mut Reader) -> Result<Self> {
        let profile = r.str()?;
        let inbound = r.u8()?;
        let ping = r.u8()? != 0;
        let stealth = r.u8()? != 0;
        let mut allow_in = Vec::new();
        for _ in 0..len(r.u32()? as usize)? {
            let protos = r.u8()?;
            let ip = r.ip()?;
            let prefix = r.u8()?;
            let lo = r.u16()?;
            allow_in.push(InRule {
                protos,
                from: (ip, prefix),
                ports: (lo, r.u16()?),
            });
        }
        let mut blocked = Vec::new();
        for _ in 0..len(r.u32()? as usize)? {
            let ip = r.ip()?;
            blocked.push((ip, r.u8()?));
        }
        let syn_rate = (r.u32()?, r.u32()?);
        let icmp_rate = (r.u32()?, r.u32()?);
        let mut dns_block = Vec::new();
        for _ in 0..len(r.u32()? as usize)? {
            dns_block.push(r.str()?);
        }
        let n = r.u32()? as usize;
        if n > MAX_RULES * 16 {
            return Err(Error::InvalidArgs);
        }
        let mut user_rules = Vec::with_capacity(n);
        for _ in 0..n {
            let app = r.str()?;
            let allow = r.u8()? != 0;
            let rule = Rule::get(r)?;
            rule.validate()?;
            user_rules.push((app, rule, allow));
        }
        let audit_capacity = r.u32()?;
        let unauthenticated_fallback = r.u8()? != 0;
        if inbound > inbound::LISTENERS {
            return Err(Error::InvalidArgs);
        }
        Ok(Self {
            profile,
            inbound,
            ping,
            stealth,
            allow_in,
            blocked,
            syn_rate,
            icmp_rate,
            dns_block,
            user_rules,
            audit_capacity,
            unauthenticated_fallback,
        })
    }
}

/// Dotaz netd -> fwd.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ask {
    pub id: u32,
    pub app: String,
    pub proto: u8,
    pub dest: SocketAddr,
    pub host: String,
}

impl Ask {
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::op(op::ASK);
        w.u32(self.id).str(&self.app).u8(self.proto).socket_addr(self.dest).str(&self.host);
        w.finish()
    }

    /// Precte telo (za kodem operace).
    pub fn get(r: &mut Reader) -> Result<Self> {
        Ok(Self {
            id: r.u32()?,
            app: r.str()?,
            proto: r.u8()?,
            dest: r.socket_addr()?,
            host: r.str()?,
        })
    }
}

/// Druh zaznamu auditu.
pub mod audit {
    /// Spojeni / datagram / ping povolen pravidly.
    pub const ALLOWED: u8 = 1;
    /// Odmitnuto pravidly relace nebo trvalym zakazem.
    pub const DENIED: u8 = 2;
    /// Uzivatel povolil (dotaz).
    pub const ASK_ALLOWED: u8 = 3;
    /// Uzivatel odmitl, nebo dotaz vyprsel.
    pub const ASK_DENIED: u8 = 4;
    /// Naslouchani na portu.
    pub const LISTEN: u8 = 5;
    /// Prichozi paket zahozen filtrem (`detail` = duvod).
    pub const INBOUND_DROP: u8 = 6;
    /// Preklad blokovaneho jmena.
    pub const DNS_BLOCKED: u8 = 7;
    /// Cil v blokovane siti.
    pub const BLOCKED_NET: u8 = 8;
    /// Zmena profilu site.
    pub const PROFILE: u8 = 9;
}

/// Zaznam auditu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditRecord {
    /// Poradove cislo (roste, i kdyz starsi zaznamy vypadnou).
    pub seq: u64,
    /// Cas (ns od startu).
    pub time: u64,
    /// [`audit`]
    pub kind: u8,
    pub app: String,
    pub proto: u8,
    pub remote: Option<SocketAddr>,
    pub local_port: u16,
    /// Jmeno hostitele, duvod zahozeni, profil, ...
    pub detail: String,
}

impl AuditRecord {
    pub fn put(&self, w: &mut Writer) {
        w.u64(self.seq).u64(self.time).u8(self.kind).str(&self.app).u8(self.proto);
        match self.remote {
            Some(a) => {
                w.u8(1).socket_addr(a);
            }
            None => {
                w.u8(0);
            }
        }
        w.u16(self.local_port).str(&self.detail);
    }

    pub fn get(r: &mut Reader) -> Result<Self> {
        let seq = r.u64()?;
        let time = r.u64()?;
        let kind = r.u8()?;
        let app = r.str()?;
        let proto = r.u8()?;
        let remote = match r.u8()? {
            0 => None,
            _ => Some(r.socket_addr()?),
        };
        Ok(Self {
            seq,
            time,
            kind,
            app,
            proto,
            remote,
            local_port: r.u16()?,
            detail: r.str()?,
        })
    }
}

/// Provoz jedne aplikace (`STATS`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AppStats {
    pub app: String,
    pub sent: u64,
    pub received: u64,
    /// Spojeni a sockety od startu.
    pub sockets: u64,
}

impl AppStats {
    pub fn put(&self, w: &mut Writer) {
        w.str(&self.app).u64(self.sent).u64(self.received).u64(self.sockets);
    }

    pub fn get(r: &mut Reader) -> Result<Self> {
        Ok(Self {
            app: r.str()?,
            sent: r.u64()?,
            received: r.u64()?,
            sockets: r.u64()?,
        })
    }
}
