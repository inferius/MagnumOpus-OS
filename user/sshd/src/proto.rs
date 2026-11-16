//! Zpravy proces spojeni (`sshd-conn`) <-> sshd na kanalu rodice.
//!
//! Proces spojeni cte data ze site a muze byt napadeny: sshd mu nevi
//! listek relace a overeni preda sluzbe uctu beze zmeny. Pozadavek =
//! `u32` op + telo (`wire`), odpoved podle op.

// Sdileno obema programy (sshd kodovani nepotrebuje, sshd-conn cteni).
#![allow(dead_code)]

use alloc::string::String;
use alloc::vec::Vec;

use libos::{Error, Result};
use wire::{Reader, Writer};

/// Prijme server klic? Telo: jmeno, blob klice. Odpoved 1 B (1 = ano).
pub const PROBE: u32 = 1;
/// Overeni podpisu. Telo: jmeno, klic, podepsana data, podpis. Odpoved
/// 1 B: [`AUTH_OK`], [`AUTH_DENIED`], [`AUTH_LOCKED`].
pub const AUTH: u32 = 2;
/// Vyzvednuti relace po uspesnem overeni. Odpoved `ok <relace>`
/// s handly [kanal okna terminalu, relace FS uctu] nebo `err KOD`.
pub const CLAIM: u32 = 3;

pub const AUTH_OK: u8 = 0;
pub const AUTH_DENIED: u8 = 1;
pub const AUTH_LOCKED: u8 = 2;

/// Nejvetsi blob v pozadavku (klic, data, podpis).
const MAX_BLOB: usize = 8 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Req {
    Probe { user: String, key: Vec<u8> },
    Auth { user: String, key: Vec<u8>, data: Vec<u8>, sig: Vec<u8> },
    Claim,
}

fn put(w: &mut Writer, b: &[u8]) {
    w.u32(b.len() as u32);
    w.bytes(b);
}

fn get(r: &mut Reader) -> Result<Vec<u8>> {
    let n = r.u32()? as usize;
    if n > MAX_BLOB {
        return Err(Error::InvalidArgs);
    }
    Ok(r.take(n)?.to_vec())
}

impl Req {
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::new();
        match self {
            Req::Probe { user, key } => {
                w.u32(PROBE).str(user);
                put(&mut w, key);
            }
            Req::Auth { user, key, data, sig } => {
                w.u32(AUTH).str(user);
                put(&mut w, key);
                put(&mut w, data);
                put(&mut w, sig);
            }
            Req::Claim => {
                w.u32(CLAIM);
            }
        }
        w.finish()
    }

    pub fn decode(b: &[u8]) -> Result<Self> {
        let mut r = Reader::new(b);
        let m = match r.u32()? {
            PROBE => Req::Probe { user: r.str()?, key: get(&mut r)? },
            AUTH => Req::Auth { user: r.str()?, key: get(&mut r)?, data: get(&mut r)?, sig: get(&mut r)? },
            CLAIM => Req::Claim,
            _ => return Err(Error::InvalidArgs),
        };
        if !r.is_empty() {
            return Err(Error::InvalidArgs);
        }
        Ok(m)
    }
}

/// Hex -> bajty (klic hostitele v konfiguraci).
pub fn unhex(s: &str) -> Option<Vec<u8>> {
    let b = s.as_bytes();
    if b.len() % 2 != 0 {
        return None;
    }
    let d = |c: u8| (c as char).to_digit(16);
    b.chunks(2).map(|p| Some((d(p[0])? * 16 + d(p[1])?) as u8)).collect()
}
