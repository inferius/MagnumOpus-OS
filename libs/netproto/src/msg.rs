//! Hlavicka zprav (verze 2): kontext operace v kazdem dotazu i odpovedi.
//!
//! | Zprava | Hlavicka | Telo |
//! |---|---|---|
//! | dotaz | `op` u32, `id` u32, `deadline` u64 (16 B) | parametry operace |
//! | odpoved | `id` u32, `status` i32 (8 B) | vysledek (jen pri `status` 0) |
//!
//! - `id` vybira klient: jedinecne mezi rozpracovanymi dotazy na kanalu,
//!   nikdy 0. Sluzba odpoved oznaci stejnym `id`; poradi odpovedi neni
//!   dane (rozpracovany RECV nebrzdi SEND, dlouhy connect nebrzdi resolve).
//! - `deadline` je absolutni cas monotonnich hodin jadra v ns (stejne hodiny
//!   ve vsech procesech, `libos::time::now`). [`NO_DEADLINE`] = bez limitu,
//!   [`TRY`] (nebo cokoli v minulosti) = jen zkusit. Vyprseni = `TIMED_OUT`.
//! - [`op::CANCEL`](crate::op::CANCEL): `id` v hlavicce je cislo ruseneho
//!   dotazu, telo prazdne, vlastni odpoved nema. Ruseny dotaz odpovi
//!   `CANCELED`; nezname nebo hotove `id` sluzba ignoruje.
//!
//! Hlavicka je zaklad "kontextu operace" budouciho IDL (cislo, termin,
//! ruseni), viz `docs/cs/architektura.md`.

use alloc::vec::Vec;
use core::time::Duration;

use abi::{Error, Result};

use crate::codec::{Reader, Writer};

/// Velikost hlavicky dotazu.
pub const REQUEST_HEADER: usize = 16;
/// Velikost hlavicky odpovedi.
pub const REPLY_HEADER: usize = 8;
/// Termin "bez limitu".
pub const NO_DEADLINE: u64 = u64::MAX;
/// Termin "jen zkusit" (necekat).
pub const TRY: u64 = 0;

/// Hlavicka dotazu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Header {
    pub op: u32,
    pub id: u32,
    pub deadline: u64,
}

/// Termin `t` od okamziku `now` (ns): `None` = bez limitu.
pub fn deadline_after(now: u64, t: Option<Duration>) -> u64 {
    match t {
        None => NO_DEADLINE,
        Some(d) => {
            let ns = u64::try_from(d.as_nanos()).unwrap_or(u64::MAX);
            // Konecny termin nesmi splynout s "bez limitu".
            now.saturating_add(ns).min(NO_DEADLINE - 1)
        }
    }
}

/// Zprava dotazu: hlavicka a telo.
pub fn request(h: Header, body: &[u8]) -> Vec<u8> {
    let mut w = Writer::new();
    w.u32(h.op).u32(h.id).u64(h.deadline).bytes(body);
    w.finish()
}

/// Rozebere dotaz: hlavicka a ctecka tela.
pub fn parse_request(data: &[u8]) -> Result<(Header, Reader<'_>)> {
    let mut r = Reader::new(data);
    let h = Header {
        op: r.u32()?,
        id: r.u32()?,
        deadline: r.u64()?,
    };
    if h.id == 0 {
        return Err(Error::InvalidArgs);
    }
    Ok((h, r))
}

/// Zprava odpovedi s cislem dotazu `id`.
pub fn reply(id: u32, result: Result<Writer>) -> Vec<u8> {
    let mut w = Writer::new();
    match result {
        Ok(body) => {
            w.u32(id).i32(0);
            w.bytes(&body.finish());
        }
        Err(e) => {
            w.u32(id).i32(e.code());
        }
    }
    w.finish()
}

/// Cislo dotazu, ke kteremu odpoved patri (`None` = zprava kratsi nez
/// hlavicka).
pub fn reply_id(data: &[u8]) -> Option<u32> {
    data.get(..4).map(|b| u32::from_le_bytes(b.try_into().unwrap()))
}

/// Rozebere odpoved: chyba sluzby, nebo ctecka vysledku.
pub fn parse_reply(data: &[u8]) -> Result<Reader<'_>> {
    let mut r = Reader::new(data);
    r.u32()?;
    let status = r.i32()?;
    if status != 0 {
        return Err(Error::from_code(status as i64).unwrap_or(Error::IoError));
    }
    Ok(r)
}
