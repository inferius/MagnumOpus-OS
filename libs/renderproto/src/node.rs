//! Kanal poskytovatele render uzlu: `gpud` <-> ovladac GPU, a stejne
//! zpravy init <-> `gpud`.
//!
//! Ovladac vyda poskytovatele jednou (`OPEN_PROVIDER` na kanalu displeje,
//! dalsi pokus `BAD_STATE`); init drzi jeho konec a `gpud` dostava
//! duplikat (pad `gpud` poskytovatele nezavre). `OPEN` vrati v handlech
//! novy kanal render uzlu ([`super::Request`]).

use alloc::vec::Vec;

use super::{Error, Result, finish};
use wire::{Reader, Writer};

pub mod op {
    pub const OPEN: u32 = 1;
    pub const OPENED: u32 = 101;
    pub const ERROR: u32 = 199;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Request {
    /// Novy render uzel (odpoved [`Reply::Opened`] s kanalem v handlech).
    Open,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reply {
    Opened,
    Error { code: Error },
}

impl Request {
    pub fn encode(&self) -> Vec<u8> {
        match self {
            Request::Open => Writer::op(op::OPEN).finish(),
        }
    }

    pub fn decode(data: &[u8]) -> Result<Self> {
        let mut r = Reader::new(data);
        let m = match r.u32()? {
            op::OPEN => Request::Open,
            _ => return Err(Error::NotSupported),
        };
        finish(r)?;
        Ok(m)
    }
}

impl Reply {
    pub fn encode(&self) -> Vec<u8> {
        match self {
            Reply::Opened => Writer::op(op::OPENED).finish(),
            Reply::Error { code } => {
                let mut w = Writer::op(op::ERROR);
                w.i32(code.code() as i32);
                w.finish()
            }
        }
    }

    pub fn decode(data: &[u8]) -> Result<Self> {
        let mut r = Reader::new(data);
        let m = match r.u32()? {
            op::OPENED => Reply::Opened,
            op::ERROR => Reply::Error {
                code: Error::from_code(r.i32()? as i64).ok_or(Error::InvalidArgs)?,
            },
            _ => return Err(Error::NotSupported),
        };
        finish(r)?;
        Ok(m)
    }
}
