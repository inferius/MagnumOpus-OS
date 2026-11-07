//! Protokol listku: klient <-> supervisor.
//!
//! Zprava = pole v kodovani `libs/wire` (little-endian). Pozadavek:
//! `u32 op`, `u32 id`, dal podle operace. Odpoved: `u32 op`, `u32 id`,
//! `i32 stav` (0 = ok, jinak kod chyby), `u64 generace`, `u8 stav sluzby`.
//! Na jednom listku je najednou nejvys jeden pozadavek; odpoved s jinym
//! `id` je pozdni odpoved (klient ji zahodi, nebo prevezme relaci z ni).
//!
//! Cteni neplatne zpravy vrati `INVALID_ARGS`, nikdy paniku.

use alloc::vec::Vec;

use abi::{Error, Result};
use wire::{Reader, Writer};

/// Kody operaci.
pub mod op {
    /// Nova relace podle receptu listku na instanci novejsi, nez na ktere
    /// byla vydana ta dosavadni. Pole: `u32 cekat_ms`. Handle odpovedi:
    /// kanal relace.
    pub const RECONNECT: u32 = 1;
    /// Zuzeny recept: zbytek zpravy = zuzeni (format sluzby). Handly
    /// odpovedi: kanal zuzene relace, novy listek.
    pub const NARROW: u32 = 2;
    /// Stav sluzby a generace (bez handlu).
    pub const STATUS: u32 = 3;
}

/// Nejdel, kolik smi klient chtit cekat na novou instanci (10 min).
pub const MAX_WAIT_MS: u32 = 600_000;

/// Stav sluzby, jak ho vidi klient.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    /// Instance bezi a potvrdila start.
    Running,
    /// Po padu: prodleva, cekani na zavislosti nebo start nove instance.
    Restarting,
    /// Supervisor to vzdal (prilis mnoho padu, nebo to vzdala zavislost).
    GaveUp,
    /// Sluzba se jen hlida (bez restartu) a skoncila.
    Stopped,
}

impl State {
    pub const fn to_u8(self) -> u8 {
        match self {
            State::Running => 0,
            State::Restarting => 1,
            State::GaveUp => 2,
            State::Stopped => 3,
        }
    }

    pub fn from_u8(v: u8) -> Result<Self> {
        Ok(match v {
            0 => State::Running,
            1 => State::Restarting,
            2 => State::GaveUp,
            3 => State::Stopped,
            _ => return Err(Error::InvalidArgs),
        })
    }

    pub const fn name(self) -> &'static str {
        match self {
            State::Running => "bezi",
            State::Restarting => "restartuje se",
            State::GaveUp => "vzdano",
            State::Stopped => "zastavena",
        }
    }
}

/// Pozadavek na listku.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Request {
    Reconnect { id: u32, wait_ms: u32 },
    Narrow { id: u32, narrowing: Vec<u8> },
    Status { id: u32 },
}

impl Request {
    pub fn id(&self) -> u32 {
        match self {
            Request::Reconnect { id, .. } | Request::Narrow { id, .. } | Request::Status { id } => *id,
        }
    }

    pub fn op(&self) -> u32 {
        match self {
            Request::Reconnect { .. } => op::RECONNECT,
            Request::Narrow { .. } => op::NARROW,
            Request::Status { .. } => op::STATUS,
        }
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::op(self.op());
        w.u32(self.id());
        match self {
            Request::Reconnect { wait_ms, .. } => {
                w.u32(*wait_ms);
            }
            Request::Narrow { narrowing, .. } => {
                w.bytes(narrowing);
            }
            Request::Status { .. } => {}
        }
        w.finish()
    }

    pub fn decode(data: &[u8]) -> Result<Self> {
        let mut r = Reader::new(data);
        let op = r.u32()?;
        let id = r.u32()?;
        let req = match op {
            op::RECONNECT => Request::Reconnect {
                id,
                wait_ms: r.u32()?.min(MAX_WAIT_MS),
            },
            op::NARROW => Request::Narrow {
                id,
                narrowing: r.rest().to_vec(),
            },
            op::STATUS => Request::Status { id },
            _ => return Err(Error::InvalidArgs),
        };
        if !r.is_empty() {
            return Err(Error::InvalidArgs);
        }
        Ok(req)
    }
}

/// Odpoved supervisoru (handly jdou zvlast ve zprave kanalu).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Reply {
    pub op: u32,
    pub id: u32,
    /// `Ok(())` nebo chyba (`SERVICE_UNAVAILABLE`, `PEER_CLOSED`, ...).
    pub status: core::result::Result<(), Error>,
    /// Generace instance, na ktere je vydana relace (nebo bezici instance).
    pub generation: u64,
    pub state: State,
}

impl Reply {
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::op(self.op);
        w.u32(self.id)
            .i32(match self.status {
                Ok(()) => 0,
                Err(e) => e.code(),
            })
            .u64(self.generation)
            .u8(self.state.to_u8());
        w.finish()
    }

    pub fn decode(data: &[u8]) -> Result<Self> {
        let mut r = Reader::new(data);
        let op = r.u32()?;
        let id = r.u32()?;
        let code = r.i32()?;
        let status = match code {
            0 => Ok(()),
            c => Err(Error::from_code(c as i64).ok_or(Error::InvalidArgs)?),
        };
        let generation = r.u64()?;
        let state = State::from_u8(r.u8()?)?;
        if !r.is_empty() {
            return Err(Error::InvalidArgs);
        }
        Ok(Reply {
            op,
            id,
            status,
            generation,
            state,
        })
    }

    /// Kolik handlu musi mit uspesna odpoved na operaci `op`.
    pub fn handles_for(op: u32) -> usize {
        match op {
            op::RECONNECT => 1,
            op::NARROW => 2,
            _ => 0,
        }
    }
}
