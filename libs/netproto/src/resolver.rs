//! Zpravy mezi `netd` a sluzbou `dnsd` (systemovy preklad jmen pres
//! sifrovane DNS a DNSSEC).
//!
//! `netd` posila dotazy systemoveho resolveru (jmeno uz overene a mimo
//! blokovane domeny) a ceka na odpoved podle `id`; z odpovedi se dal uci
//! pravidla se jmenem hostitele. `dnsd` nevidi, ktera aplikace se pta.

use alloc::string::String;
use alloc::vec::Vec;
use core::net::IpAddr;

use crate::{ReadAddr, Reader, WriteAddr, Writer};
use abi::Result;

/// Vysledek prekladu.
pub mod status {
    /// Adresy (overene DNSSEC, nebo zona nepodepsana).
    pub const OK: u8 = 0;
    /// Jmeno neexistuje (overene, nebo zona nepodepsana).
    pub const NXDOMAIN: u8 = 1;
    /// Jmeno existuje, adresa daneho typu ne.
    pub const NODATA: u8 = 2;
    /// Zadny sifrovany resolver neodpovedel (netd pak podle profilu site
    /// pouzije neoverene DNS, nebo prelozit nejde).
    pub const UNAVAILABLE: u8 = 3;
    /// DNSSEC: podpis nebo dukaz neprosel - odpoved je podvrzena.
    pub const BOGUS: u8 = 4;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Query {
    pub id: u32,
    /// 0 = A i AAAA, 4, 6.
    pub family: u8,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Answer {
    pub id: u32,
    pub status: u8,
    pub ttl: u32,
    pub addrs: Vec<IpAddr>,
    /// Bylo overeno DNSSEC (ne jen nepodepsana zona).
    pub secure: bool,
}

impl Query {
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::new();
        w.u32(self.id).u8(self.family).str(&self.name);
        w.finish()
    }

    pub fn decode(b: &[u8]) -> Result<Self> {
        let mut r = Reader::new(b);
        Ok(Self {
            id: r.u32()?,
            family: r.u8()?,
            name: r.str()?,
        })
    }
}

impl Answer {
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::new();
        w.u32(self.id).u8(self.status).u32(self.ttl).u8(self.secure as u8);
        w.u16(self.addrs.len().min(64) as u16);
        for a in self.addrs.iter().take(64) {
            w.ip(*a);
        }
        w.finish()
    }

    pub fn decode(b: &[u8]) -> Result<Self> {
        let mut r = Reader::new(b);
        let id = r.u32()?;
        let status = r.u8()?;
        let ttl = r.u32()?;
        let secure = r.u8()? != 0;
        let n = r.u16()?;
        let mut addrs = Vec::with_capacity(n as usize);
        for _ in 0..n {
            addrs.push(r.ip()?);
        }
        Ok(Self { id, status, ttl, addrs, secure })
    }
}
