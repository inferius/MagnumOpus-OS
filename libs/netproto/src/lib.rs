//! Protokol site: zpravy po kanalech mezi programy a sluzbou `netd`
//! a mezi `netd` a ovladacem sitove karty ([`link`]).
//!
//! **Kanaly jsou capability.** Program na sit nemuze, dokud mu nekdo (init,
//! pozdeji spravce sezeni) nepreda kanal **relace**. Relace ma prava
//! a omezeni cilu ([`policy`]); uzsi relaci pro jiny program vytvori
//! [`op::RESTRICT`]. Kazdy socket je **vlastni kanal**: lze ho predat jinemu
//! procesu a zavreni kanalu socket zavre.
//!
//! Dotaz: hlavicka (`op` u32, `id` u32, termin u64) a parametry. Odpoved:
//! `id` u32, `status` i32 (0 nebo kod chyby) a vysledek; novy socket nebo
//! relace jde v handlech odpovedi. Na jednom kanalu muze byt vic dotazu
//! najednou; odpovedi se paruji podle `id` ([`msg`]). Cisla little-endian,
//! adresa IP = rodina u8 (4/6) + bajty, retezec = delka u16 + UTF-8, data =
//! zbytek zpravy.
//!
//! Cekani: absolutni termin v hlavicce ([`msg::NO_DEADLINE`] = bez limitu,
//! [`msg::TRY`] = jen zkusit). Vyprseni = `TIMED_OUT`, zruseni
//! ([`op::CANCEL`]) = `CANCELED`.
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
pub mod msg;
pub mod fw;
pub mod link;
pub mod ntp;
#[cfg(feature = "nts")]
pub mod nts;
pub mod policy;
pub mod resolver;

#[cfg(feature = "client")]
mod client;
pub mod io;

#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_msg;
#[cfg(all(test, feature = "nts"))]
mod tests_nts;

use alloc::vec::Vec;
use core::net::IpAddr;


use abi::Result;

pub use codec::{ReadAddr, Reader, WriteAddr, Writer};
pub use msg::{Header, NO_DEADLINE, TRY, parse_reply, reply, reply_id};
pub use auth::{Auth, Quota, Restriction, UserRules, Verdict, layer_flags};
pub use policy::{Policy, Rule, Target, proto, rights, rights_names};

#[cfg(feature = "client")]
pub use client::{CancelToken, Deadline, Net, NtpSync, Readiness, TcpListener, TcpStream, UdpSocket};

/// Nejvic bajtu dat v jedne zprave proudu (kousek; klient vetsi data deli,
/// zprava kanalu ma nejvys 64 KiB).
pub const MAX_IO: usize = 60 * 1024;
/// Nejvic rozpracovanych dotazu stejneho smeru na jednom socketu (RECV,
/// SEND, ACCEPT, RECV_FROM, SEND_TO): sluzba je vyrizuje postupne ve
/// fronte FIFO, dalsi dostane `NO_RESOURCES`.
pub const DIRECTION_QUEUE: usize = 8;
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

/// Kody operaci.
pub mod op {
    // Kazdy kanal.
    /// Zrusi rozpracovany dotaz s `id` z hlavicky (viz [`crate::msg`]);
    /// vlastni odpoved nema.
    pub const CANCEL: u32 = 0xFFFF_FFFF;

    // Kanal relace.
    /// () -> Info
    pub const INFO: u32 = 1;
    /// () -> Info (az bude rozhrani nastavene, napr. po DHCP; do terminu)
    pub const WAIT_CONFIGURED: u32 = 2;
    /// cil socket_addr, Buffers -> mistni socket_addr + kanal spojeni
    pub const TCP_CONNECT: u32 = 3;
    /// mistni socket_addr (0.0.0.0 nebo adresa rozhrani), fronta u16,
    /// Buffers -> mistni socket_addr + kanal naslouchani
    pub const TCP_LISTEN: u32 = 4;
    /// mistni socket_addr, Buffers -> mistni socket_addr + kanal socketu UDP
    pub const UDP_BIND: u32 = 5;
    /// jmeno str, rodina u8 (0 = obe, 4, 6), ma server u8 [socket_addr]
    /// -> pocet u16, adresy, TTL u32
    pub const RESOLVE: u32 = 6;
    /// adresa, poradi u16, delka dat u16 -> doba odezvy ns u64
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
    /// data -> prijato u32 (vse, nebo cast pri vyprseni terminu)
    pub const SEND: u32 = 20;
    /// nejvic u32 -> data (prazdna = druha strana ukoncila zapis)
    pub const RECV: u32 = 21;
    /// () -> () - ukonci zapis (FIN), cteni pokracuje
    pub const SHUTDOWN: u32 = 22;
    /// () -> mistni socket_addr, vzdalena socket_addr
    pub const ADDRS: u32 = 23;
    // Plan (kody rezervovane): RECV s priznakem PEEK, SHUTDOWN_READ (24),
    // SET_OPTION (25), GET_OPTION (26) - nodelay, ttl, keepalive, linger,
    // broadcast.

    // Kanal naslouchani TCP.
    /// () -> vzdalena socket_addr, mistni socket_addr + kanal spojeni
    pub const ACCEPT: u32 = 30;

    // Kanal naslouchani i socketu UDP.
    /// () -> mistni socket_addr (podle aktualni adresy rozhrani)
    pub const LOCAL_ADDR: u32 = 31;

    // Kanal kazdeho socketu (spojeni, naslouchani, UDP).
    /// () -> () + event pripravenosti v handlech (jen pravo `WAIT`), viz
    /// [`crate::ready`]
    pub const READINESS: u32 = 50;

    // Kanal socketu UDP.
    /// cil socket_addr, data -> ()
    pub const SEND_TO: u32 = 40;
    /// nejvic u32 -> odesilatel socket_addr, data
    pub const RECV_FROM: u32 = 41;
}

/// Bity pripravenosti socketu na eventu od [`op::READINESS`] (uzivatelske
/// bity eventu). Stav je urovnovy: bit plati, dokud plati podminka.
pub mod ready {
    use abi::Signals;

    /// Cteni (`RECV`, `RECV_FROM`, `ACCEPT`) neblokuje: data, datagram,
    /// spojeni k prijeti, konec proudu nebo chyba.
    pub const READABLE: Signals = Signals::USER_0;
    /// Zapis neblokuje: misto ve fronte odesilani (nebo chyba).
    pub const WRITABLE: Signals = Signals::USER_1;
    /// Druha strana ukoncila zapis (FIN) nebo je spojeni zavrene.
    pub const HUP: Signals = Signals::USER_2;
    /// Spojeni preruseno (RST, druha strana prestala odpovidat).
    pub const ERROR: Signals = Signals::USER_3;
    /// Vsechny bity pripravenosti.
    pub const ALL: Signals = Signals(READABLE.0 | WRITABLE.0 | HUP.0 | ERROR.0);
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
    /// Profil site dovoluje neoverene DNS a NTP jako zalohu (firewall).
    pub unauthenticated_fallback: bool,
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
        w.u8(self.unauthenticated_fallback as u8);
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
            unauthenticated_fallback: r.u8()? != 0,
        })
    }
}
