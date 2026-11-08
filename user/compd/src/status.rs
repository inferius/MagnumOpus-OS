//! Stav systemu v liste: sit (`netd`, relace jen s pravem `INFO`)
//! a hlasitost (`audiod`). Kompozitor se nesmi zablokovat na cizi sluzbe:
//! dotaz se posle, odpoved se precte, az prijde (cekani spolu s ostatnimi
//! kanaly), dalsi dotaz za [`PERIOD`].

use alloc::vec::Vec;

use libos::{Channel, Error};

use crate::compd_now;

/// Jak casto se ptat na stav.
pub const PERIOD: u64 = 2_000_000_000;
/// Bez odpovedi tak dlouho: zkusit znovu (sluzba mohla zpravu zahodit).
const RETRY: u64 = 10_000_000_000;

/// Jeden zdroj stavu: kanal a pozadavek, ktery se opakuje.
pub struct Source {
    pub ch: Channel,
    request: Vec<u8>,
    /// Kdy se zeptat znovu; `pending` = dotaz je na ceste od `sent`.
    next: u64,
    pending: Option<u64>,
}

impl Source {
    pub fn new(ch: Channel, request: Vec<u8>) -> Self {
        Self { ch, request, next: 0, pending: None }
    }

    /// Posle dotaz, kdyz je cas; vraci, kdy se probudit.
    pub fn poll(&mut self, now: u64) -> Option<u64> {
        if let Some(sent) = self.pending {
            if now < sent + RETRY {
                return Some(sent + RETRY);
            }
            self.pending = None;
        }
        if now >= self.next {
            if self.ch.write(&self.request, Vec::new()).is_ok() {
                self.pending = Some(now);
            }
            self.next = now + PERIOD;
        }
        Some(self.next)
    }

    /// Precte odpoved (bez cekani): `Ok(Some)` = novy stav, `Ok(None)` =
    /// nic neprislo, `Err` = sluzba skoncila.
    pub fn read<T>(&mut self, parse: fn(&[u8]) -> Option<T>) -> Result<Option<T>, ()> {
        let mut out = None;
        loop {
            match self.ch.try_read() {
                Ok(m) => {
                    self.pending = None;
                    self.next = compd_now() + PERIOD;
                    if let Some(v) = parse(&m.data) {
                        out = Some(v);
                    }
                }
                Err(Error::ShouldWait) => return Ok(out),
                Err(_) => return Err(()),
            }
        }
    }
}

pub fn net_request() -> Vec<u8> {
    netproto::Writer::op(netproto::op::INFO).finish()
}

pub fn audio_request() -> Vec<u8> {
    audioproto::client::Request::Status.encode()
}

pub fn net_state(data: &[u8]) -> Option<comp::render::NetState> {
    let mut r = netproto::parse_reply(data).ok()?;
    let i = netproto::Info::get(&mut r).ok()?;
    Some(if !i.link_up {
        comp::render::NetState::Offline
    } else if i.configured {
        comp::render::NetState::Online
    } else {
        comp::render::NetState::Connecting
    })
}

pub fn volume(data: &[u8]) -> Option<(u8, bool)> {
    let body = audioproto::decode_reply(data).ok()?;
    let s = audioproto::client::Status::decode(body).ok()?;
    Some((s.master, s.muted))
}
