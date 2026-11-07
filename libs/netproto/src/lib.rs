//! Protokol site: zpravy po kanalech mezi programy a sluzbou `netd`
//! a mezi `netd` a ovladacem sitove karty ([`link`]).
//!
//! **Kanaly jsou capability.** Program na sit nemuze, dokud mu nekdo (init,
//! pozdeji spravce sezeni) nepreda kanal **relace**. Relace ma prava
//! a omezeni cilu ([`policy`]); uzsi relaci pro jiny program vytvori
//! [`op::RESTRICT`]. Kazdy socket je **vlastni kanal**: lze ho predat jinemu
//! procesu a zavreni kanalu socket zavre.
//!
//! Pozadavek: `op` u32 a parametry. Odpoved: `status` i32 (0 nebo kod
//! chyby), rezerva u32 a vysledek; novy socket nebo relace jde v handlech
//! odpovedi. Na jednom kanalu je vzdy nejvys jeden pozadavek: sluzba dalsi
//! zpravu precte az po odpovedi. Cisla little-endian, adresa IP = rodina u8
//! (4/6) + bajty, retezec = delka u16 + UTF-8, data = zbytek zpravy.
//!
//! Cekani: parametr `timeout` v ms, [`NO_WAIT`] = jen zkusit,
//! [`FOREVER`] = bez limitu. Vyprseni = `TIMED_OUT`.
//!
//! Adresy jsou `core::net::IpAddr` (IPv4 i IPv6). Sluzba zatim umi jen IPv4;
//! IPv6 vraci `UNREACHABLE`, ale protokol se jeho pridanim nezmeni.
//!
//! Popis: `docs/cs/sit.md`, `docs/en/network.md`.

#![no_std]

extern crate alloc;
#[cfg(any(test, feature = "std"))]
extern crate std;

pub mod auth;
mod codec;
pub mod dns;
pub mod fw;
pub mod link;
pub mod ntp;
pub mod policy;

#[cfg(feature = "client")]
mod client;
pub mod io;

#[cfg(test)]
mod tests;

use alloc::vec::Vec;
use core::net::IpAddr;
use core::time::Duration;

use abi::{Error, Result};

pub use codec::{ReadAddr, Reader, WriteAddr, Writer};
pub use auth::{Auth, Quota, Restriction, UserRules, Verdict, layer_flags};
pub use policy::{Policy, Rule, Target, proto, rights, rights_names};

#[cfg(feature = "client")]
pub use client::{Net, NtpSync, TcpListener, TcpStream, UdpSocket};

/// Nejvic bajtu dat v jedne zprave proudu (kousek; klient vetsi data deli,
/// zprava kanalu ma nejvys 64 KiB).
pub const MAX_IO: usize = 60 * 1024;
/// Nejvetsi datagram UDP: nejvetsi obsah datagramu IPv4 (65 535 - 20 B
/// hlavicky IP - 8 B hlavicky UDP). Vejde se do jedne zpravy kanalu.
pub const MAX_DATAGRAM: usize = 65_507;

/// Velikost bufferu socketu (prijem, odesilani) v bajtech; 0 = vychozi
/// (64 KiB). Sluzba hodnotu omezi na [`Buffers::MIN`]..=[`Buffers::MAX`]
/// a strhne ji z rozpoctu relace.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Buffers {
    pub rx: u32,
    pub tx: u32,
}

impl Buffers {
    pub const DEFAULT: Self = Self { rx: 0, tx: 0 };
    pub const DEFAULT_SIZE: u32 = 64 * 1024;
    pub const MIN: u32 = 4 * 1024;
    pub const MAX: u32 = 16 * 1024 * 1024;

    pub fn put(&self, w: &mut Writer) {
        w.u32(self.rx).u32(self.tx);
    }

    pub fn get(r: &mut Reader) -> Result<Self> {
        Ok(Self {
            rx: r.u32()?,
            tx: r.u32()?,
        })
    }

    /// Skutecne velikosti (vychozi a meze).
    pub fn resolved(&self) -> (usize, usize) {
        let f = |v: u32| {
            if v == 0 {
                Self::DEFAULT_SIZE
            } else {
                v.clamp(Self::MIN, Self::MAX)
            }
        };
        (f(self.rx) as usize, f(self.tx) as usize)
    }
}

/// Timeout "necekat".
pub const NO_WAIT: u64 = 0;
/// Timeout "cekat bez limitu".
pub const FOREVER: u64 = u64::MAX;

/// Prevod timeoutu na drat (ms, zaokrouhleno nahoru).
pub fn timeout_ms(t: Option<Duration>) -> u64 {
    match t {
        None => FOREVER,
        Some(d) => {
            let ms = d.as_nanos().div_ceil(1_000_000);
            ms.min(FOREVER as u128 - 1) as u64
        }
    }
}

/// Kody operaci.
pub mod op {
    // Kanal relace.
    /// () -> Info
    pub const INFO: u32 = 1;
    /// timeout -> Info (az bude rozhrani nastavene, napr. po DHCP)
    pub const WAIT_CONFIGURED: u32 = 2;
    /// cil socket_addr, timeout, Buffers -> mistni socket_addr + kanal spojeni
    pub const TCP_CONNECT: u32 = 3;
    /// port u16, fronta u16, Buffers -> mistni socket_addr + kanal naslouchani
    pub const TCP_LISTEN: u32 = 4;
    /// port u16, Buffers -> mistni socket_addr + kanal socketu UDP
    pub const UDP_BIND: u32 = 5;
    /// jmeno str, rodina u8 (0 = obe, 4, 6), ma server u8 [socket_addr],
    /// timeout -> pocet u16, adresy, TTL u32
    pub const RESOLVE: u32 = 6;
    /// adresa, poradi u16, delka dat u16, timeout -> doba odezvy ns u64
    pub const PING: u32 = 7;
    /// Restriction (prava u32, Policy, priznaky u8, aplikace str, sockety
    /// u32, bajty u64) -> kanal nove (uzsi) relace
    pub const RESTRICT: u32 = 8;
    /// pocet u8, servery socket_addr -> () - systemove servery DNS
    /// (pravo `CONFIGURE`); prazdny seznam = zpet servery z DHCP
    pub const SET_DNS: u32 = 9;
    /// od seq u64, nejvic u16 -> dalsi seq u64, pocet u16, zaznamy
    /// (`fw::AuditRecord`); pravo `AUDIT`
    pub const AUDIT: u32 = 10;
    /// () -> profil str, pocet u16, `fw::AppStats`; pravo `AUDIT`
    pub const STATS: u32 = 11;

    // Kanal spojeni TCP.
    /// timeout, data -> prijato u32 (vse, nebo cast pri vyprseni)
    pub const SEND: u32 = 20;
    /// nejvic u32, timeout -> data (prazdna = druha strana ukoncila zapis)
    pub const RECV: u32 = 21;
    /// () -> () - ukonci zapis (FIN), cteni pokracuje
    pub const SHUTDOWN: u32 = 22;
    /// () -> mistni socket_addr, vzdalena socket_addr
    pub const ADDRS: u32 = 23;

    // Kanal naslouchani TCP.
    /// timeout -> vzdalena socket_addr, mistni socket_addr + kanal spojeni
    pub const ACCEPT: u32 = 30;

    // Kanal naslouchani i socketu UDP.
    /// () -> mistni socket_addr (podle aktualni adresy rozhrani)
    pub const LOCAL_ADDR: u32 = 31;

    // Kanal socketu UDP.
    /// cil socket_addr, data -> ()
    pub const SEND_TO: u32 = 40;
    /// nejvic u32, timeout -> odesilatel socket_addr, data
    pub const RECV_FROM: u32 = 41;
}

/// Stav rozhrani.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Info {
    pub mac: [u8; 6],
    pub mtu: u16,
    pub link_up: bool,
    /// Ma rozhrani adresu (DHCP nebo staticky)?
    pub configured: bool,
    /// Adresa a delka prefixu.
    pub addr: Option<(IpAddr, u8)>,
    pub gateway: Option<IpAddr>,
    pub dns: Vec<IpAddr>,
    /// Prava relace, ktera se ptala.
    pub rights: u32,
}

impl Info {
    pub fn put(&self, w: &mut Writer) {
        w.bytes(&self.mac).u16(self.mtu);
        w.u8(self.link_up as u8 | (self.configured as u8) << 1);
        match self.addr {
            Some((a, p)) => {
                w.u8(1).ip(a).u8(p);
            }
            None => {
                w.u8(0);
            }
        }
        match self.gateway {
            Some(g) => {
                w.u8(1).ip(g);
            }
            None => {
                w.u8(0);
            }
        }
        w.u8(self.dns.len() as u8);
        for d in &self.dns {
            w.ip(*d);
        }
        w.u32(self.rights);
    }

    pub fn get(r: &mut Reader) -> Result<Self> {
        let mut mac = [0u8; 6];
        for b in &mut mac {
            *b = r.u8()?;
        }
        let mtu = r.u16()?;
        let flags = r.u8()?;
        let addr = match r.u8()? {
            0 => None,
            _ => {
                let a = r.ip()?;
                Some((a, r.u8()?))
            }
        };
        let gateway = match r.u8()? {
            0 => None,
            _ => Some(r.ip()?),
        };
        let n = r.u8()?;
        let mut dns = Vec::with_capacity(n as usize);
        for _ in 0..n {
            dns.push(r.ip()?);
        }
        Ok(Self {
            mac,
            mtu,
            link_up: flags & 1 != 0,
            configured: flags & 2 != 0,
            addr,
            gateway,
            dns,
            rights: r.u32()?,
        })
    }
}

/// Telo odpovedi: status, rezerva a vysledek.
pub fn reply(result: Result<Writer>) -> Vec<u8> {
    let mut w = Writer::new();
    match result {
        Ok(body) => {
            w.i32(0).u32(0);
            w.bytes(&body.finish());
        }
        Err(e) => {
            w.i32(e.code()).u32(0);
        }
    }
    w.finish()
}

/// Rozebere odpoved: chyba sluzby, nebo ctecka vysledku.
pub fn parse_reply(data: &[u8]) -> Result<Reader<'_>> {
    let mut r = Reader::new(data);
    let status = r.i32()?;
    r.u32()?;
    if status != 0 {
        return Err(Error::from_code(status as i64).unwrap_or(Error::IoError));
    }
    Ok(r)
}
