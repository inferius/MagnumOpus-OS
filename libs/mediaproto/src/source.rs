//! Protokol zdroje bajtu: dekoder cte vstup po kusech od `mediad`,
//! `mediad` cte od aplikace (kanal zdroje) nebo z VMO. Dekoder tak nikdy
//! nedostane handle souboru ani site, jen bajty, o ktere si rekl.
//!
//! Pozadavek: op u32 + parametry. Odpoved: status i32 + data (bez znacky
//! [`crate::TAG_REPLY`] - kanal zdroje nema udalosti).

use alloc::vec::Vec;

use crate::{Error, Result};
use wire::{Reader, Writer};

/// Nejvic bajtu v jednom cteni: odpoved (status + data) se musi vejit do
/// zpravy kanalu (`abi::syscall::CHANNEL_MAX_DATA` = 64 KiB).
pub const MAX_READ: u32 = 60 * 1024;

pub mod op {
    /// Posun u64, delka u32 -> data (kratsi jen na konci zdroje).
    pub const READ_AT: u32 = 1;
    /// -> velikost u64 (`u64::MAX` = nezname, proud).
    pub const SIZE: u32 = 2;
}

/// Velikost neznameho zdroje (zive vysilani).
pub const UNKNOWN_SIZE: u64 = u64::MAX;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceRequest {
    ReadAt { offset: u64, len: u32 },
    Size,
}

impl SourceRequest {
    pub fn encode(&self) -> Vec<u8> {
        let mut w;
        match *self {
            SourceRequest::ReadAt { offset, len } => {
                w = Writer::op(op::READ_AT);
                w.u64(offset).u32(len);
            }
            SourceRequest::Size => w = Writer::op(op::SIZE),
        }
        w.finish()
    }

    pub fn decode(m: &[u8]) -> Result<Self> {
        let mut r = Reader::new(m);
        let req = match r.u32()? {
            op::READ_AT => {
                let offset = r.u64()?;
                let len = r.u32()?;
                if len == 0 || len > MAX_READ || offset.checked_add(len as u64).is_none() {
                    return Err(Error::InvalidArgs);
                }
                SourceRequest::ReadAt { offset, len }
            }
            op::SIZE => SourceRequest::Size,
            _ => return Err(Error::NotSupported),
        };
        if !r.is_empty() {
            return Err(Error::InvalidArgs);
        }
        Ok(req)
    }
}

/// Odpoved zdroje.
pub fn encode_reply(r: Result<&[u8]>) -> Vec<u8> {
    let mut w = Writer::new();
    match r {
        Ok(d) => {
            w.i32(0).bytes(d);
        }
        Err(e) => {
            w.i32(e.code());
        }
    }
    w.finish()
}

pub fn decode_reply(m: &[u8]) -> Result<&[u8]> {
    let mut r = Reader::new(m);
    let s = r.i32()?;
    if s != 0 {
        return Err(Error::from_code(s as i64).unwrap_or(Error::Corrupted));
    }
    Ok(r.rest())
}

/// Vyrez `[offset, offset + len)` z dat velikosti `size`, oriznuty na
/// konec (prazdny za koncem). Pro zdroj nad VMO nebo pameti.
pub fn clamp(offset: u64, len: u32, size: u64) -> (u64, usize) {
    if offset >= size {
        return (offset, 0);
    }
    (offset, (size - offset).min(len as u64) as usize)
}
