//! Systemovy preklad pres sluzbu `dnsd` (sifrovane DNS, DNSSEC).
//!
//! Kanal k dnsd dostane netd pri startu (4. handle; init druhy konec preda
//! dnsd, az ji spusti - do te doby dotazy cekaji v kanalu). Dotaz relace se
//! systemovym resolverem jde do dnsd; odpoved se paruje podle `id`.
//!
//! Kdyz dnsd odpovi `UNAVAILABLE`, nebezi nebo nestihne termin, rozhodne
//! profil site: s `unauthenticated = "fallback"` se pouzije puvodni
//! neovereny resolver (UDP/TCP na servery z DHCP), jinak preklad selze
//! (`UNREACHABLE`). Podvrzena odpoved (`BOGUS`) zalohu nespousti - je to
//! utok, ne vypadek.

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;

use libos::{Channel, Error, println};
use netproto::resolver::{Answer, Query, status};

/// Nejvic nevyzvednutych odpovedi (zruseny dotaz = odpoved nikdo nechce).
const MAX_REPLIES: usize = 256;

pub struct Forwarder {
    ch: Option<Channel>,
    /// Byl kanal predan pri startu (jinak se dnsd nepouziva vubec).
    pub configured: bool,
    next_id: u32,
    replies: BTreeMap<u32, Answer>,
}

impl Forwarder {
    pub fn new(ch: Option<Channel>) -> Self {
        Self {
            configured: ch.is_some(),
            ch,
            next_id: 1,
            replies: BTreeMap::new(),
        }
    }

    pub fn channel(&self) -> Option<&Channel> {
        self.ch.as_ref()
    }

    pub fn alive(&self) -> bool {
        self.ch.is_some()
    }

    /// Posle dotaz; `None` = dnsd nebezi.
    pub fn send(&mut self, name: &str, family: u8) -> Option<u32> {
        let ch = self.ch.as_ref()?;
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1).max(1);
        let q = Query {
            id,
            family,
            name: String::from(name),
        };
        match ch.write(&q.encode(), Vec::new()) {
            Ok(()) => Some(id),
            Err(e) => {
                println!("netd: dnsd neprijima dotazy ({e}), preklad bez nej");
                self.ch = None;
                None
            }
        }
    }

    /// Precte odpovedi. Zavreny kanal = dal bez dnsd.
    pub fn pump(&mut self) {
        let Some(ch) = &self.ch else {
            return;
        };
        loop {
            match ch.try_read() {
                Ok(m) => {
                    if let Ok(a) = Answer::decode(&m.data) {
                        if self.replies.len() >= MAX_REPLIES {
                            self.replies.pop_first();
                        }
                        self.replies.insert(a.id, a);
                    }
                }
                Err(Error::ShouldWait) => return,
                Err(e) => {
                    println!("netd: dnsd skoncil ({e}), preklad bez nej");
                    self.ch = None;
                    return;
                }
            }
        }
    }

    pub fn take(&mut self, id: u32) -> Option<Answer> {
        self.replies.remove(&id)
    }

    /// Zrusi cekani na odpoved (zahodi ji, az prijde).
    pub fn forget(&mut self, id: u32) {
        self.replies.remove(&id);
    }
}

/// Rozpracovany dotaz pres dnsd.
pub struct Forward {
    pub id: u32,
    pub name: String,
    pub family: u8,
    pub deadline: u64,
    pub timeout: u64,
}

/// Vysledek pro relaci: adresy a TTL, chyba, nebo zaloha.
pub enum Outcome {
    Addrs(Vec<core::net::IpAddr>, u32),
    Fail(Error),
    Fallback,
}

pub fn outcome(a: &Answer) -> Outcome {
    match a.status {
        status::OK if !a.addrs.is_empty() => Outcome::Addrs(a.addrs.clone(), a.ttl),
        status::OK | status::NXDOMAIN | status::NODATA => Outcome::Fail(Error::NotFound),
        status::BOGUS => Outcome::Fail(Error::Corrupted),
        _ => Outcome::Fallback,
    }
}

