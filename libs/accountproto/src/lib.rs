//! Protokol sluzby uzivatelskych uctu `accountd` (verze 1) a CTAP2 pro
//! bezpecnostni klice FIDO2 (modul [`ctap`]).
//!
//! Popis: `docs/cs/ucty.md`. Kanaly jsou ruzne capability:
//!
//! - **login** (prihlasovaci obrazovka): vypis uctu, overeni, odemceni,
//! - **self** (relace uzivatele): vlastni ucet,
//! - **setup** (instalator): zalozeni prvniho uctu, jen dokud zadny neni,
//! - **init** (soukromy kanal accountd <-> init): relace, domovy, klice.
//!
//! Zprava = kod operace `u32` + pole (`libs/wire`, little-endian).
//! Odpoved = stav `i32` (0 = ok, jinak `abi::Error`) + telo. Cteni
//! neplatne zpravy vrati `INVALID_ARGS`, nikdy paniku. Tajemstvi (heslo,
//! PIN) jsou v typu [`Secret`], ktery se pri zahozeni vynuluje a do logu
//! se nevypise.

#![cfg_attr(not(test), no_std)]

extern crate alloc;

pub mod ctap;
pub mod elevate;
pub mod perm;
mod msg;
#[cfg(feature = "client")]
pub mod client;

#[cfg(test)]
mod tests;
#[cfg(test)]
mod perm_tests;

use alloc::string::String;
use alloc::vec::Vec;
use core::fmt;

pub use abi::{Error, Result};
pub use msg::*;
use wire::{Reader, Writer};
use zeroize::Zeroize;

pub const VERSION: u32 = 1;

/// Nejdelsi tajemstvi ve zprave (heslo v UTF-8, PIN, klic domova).
pub const MAX_SECRET: usize = 1024;
/// Nejdelsi blob ve zprave (parametry ukonu, id klice).
pub const MAX_BLOB: usize = 16 * 1024;

/// Ucty sluzeb a lidi (viz "Cisla identit" v `docs/cs/ucty.md`).
pub mod ids {
    /// Servisni ucet accountd na FS.
    pub const ACCOUNTD: u64 = 902;
    /// Prvni cislo cloveka.
    pub const FIRST_USER: u64 = 1000;
    /// Posledni cislo cloveka.
    pub const LAST_USER: u64 = 999_999;
    /// Skupina vsech lidi.
    pub const GROUP_USERS: u64 = 1_000_000;
    /// Prvni cislo dalsi skupiny.
    pub const FIRST_GROUP: u64 = 1_000_001;
}

/// Kody operaci.
pub mod op {
    // login
    pub const LIST: u32 = 1;
    pub const PHRASE: u32 = 2;
    pub const AUTH: u32 = 3;
    pub const UNLOCK: u32 = 4;
    /// Zamknout relaci (zamykaci obrazovka; odemknout jde jen overenim).
    pub const LOGIN_LOCK: u32 = 5;
    // self
    pub const WHOAMI: u32 = 10;
    pub const SET_PASSWORD: u32 = 11;
    pub const SET_PIN: u32 = 12;
    pub const ENROLL_KEY: u32 = 13;
    pub const REMOVE_KEY: u32 = 14;
    pub const SET_PHRASE: u32 = 15;
    pub const ENCRYPT_HOME: u32 = 16;
    pub const LOCK: u32 = 17;
    pub const LOGOUT: u32 = 18;
    pub const ELEVATE: u32 = 19;
    pub const SET_KEY_PIN: u32 = 20;
    /// Vlastni rozhodnuti o povolenich aplikaci (`perm.rs`).
    pub const PERM_LIST: u32 = 21;
    pub const PERM_REVOKE: u32 = 22;
    /// Rezim vyvojare (`Request::DevMode`, jen feature `dev-mode`); cislo
    /// rezervovane i ve vydani, kde ho accountd odmitne `NOT_SUPPORTED`.
    pub const DEV_MODE: u32 = 23;
    // setup
    pub const CREATE_FIRST: u32 = 30;
    // grants (sluzby ukonu)
    pub const CHECK_GRANT: u32 = 40;
    // names (jen cteni: cislo -> zobrazovane jmeno; spravce souboru)
    pub const NAMES: u32 = 41;
    // init (accountd -> init)
    pub const OPEN_SESSION: u32 = 50;
    pub const CLOSE_SESSION: u32 = 51;
    pub const LOCK_SESSION: u32 = 52;
    pub const UNLOCK_SESSION: u32 = 53;
    pub const CREATE_HOME: u32 = 54;
    pub const DELETE_HOME: u32 = 55;
    pub const LIST_SESSIONS: u32 = 56;
    pub const UNLOCK_HOME: u32 = 57;
    // init -> accountd
    pub const SESSION_CLOSED: u32 = 60;
}

/// Metoda overeni.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    Password,
    Pin,
    /// Bezpecnostni klic FIDO2 (secret = PIN klice, prazdny = bez PINu).
    Key,
    /// Automaticke prihlaseni jen pro vyvoj (`cargo xtask run`).
    Dev,
    /// Obnovovaci klic (zapomenute heslo u sifrovaneho domova).
    Recovery,
}

impl Method {
    pub const fn to_u8(self) -> u8 {
        match self {
            Method::Password => 1,
            Method::Pin => 2,
            Method::Key => 3,
            Method::Dev => 4,
            Method::Recovery => 5,
        }
    }

    pub fn from_u8(v: u8) -> Result<Self> {
        Ok(match v {
            1 => Method::Password,
            2 => Method::Pin,
            3 => Method::Key,
            4 => Method::Dev,
            5 => Method::Recovery,
            _ => return Err(Error::InvalidArgs),
        })
    }

    /// Bit v masce metod uctu ([`AccountSummary::methods`]).
    pub const fn bit(self) -> u8 {
        1 << (self.to_u8() - 1)
    }

    pub const fn name(self) -> &'static str {
        match self {
            Method::Password => "heslo",
            Method::Pin => "pin",
            Method::Key => "klic",
            Method::Dev => "vyvoj",
            Method::Recovery => "obnova",
        }
    }
}

/// Priznaky uctu.
pub mod flags {
    /// Role "smi spravovat tento pocitac".
    pub const ADMIN: u32 = 1;
    /// Ucet vypnuty (prihlaseni odmitnuto).
    pub const DISABLED: u32 = 2;
    /// Vyvojovy ucet (automaticke prihlaseni, jen ladici build).
    pub const DEV: u32 = 4;
    /// Domov sifrovany.
    pub const ENCRYPTED: u32 = 8;
    /// PIN bez hardwaroveho klice zarizeni (TPM) - nastaveni varuje.
    pub const PIN_WEAK: u32 = 16;
}

/// Tajemstvi (heslo, PIN, klic). Pri zahozeni se vynuluje; `Debug`
/// a `Display` vypisi `***`.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct Secret(Vec<u8>);

impl Secret {
    pub fn new(b: Vec<u8>) -> Self {
        Secret(b)
    }

    pub fn from_slice(b: &[u8]) -> Self {
        Secret(b.to_vec())
    }

    pub fn expose(&self) -> &[u8] {
        &self.0
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl From<&str> for Secret {
    fn from(s: &str) -> Self {
        Secret(s.as_bytes().to_vec())
    }
}

impl Drop for Secret {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("***")
    }
}

impl fmt::Display for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("***")
    }
}

/// Zapis delky `u32` a dat.
pub fn put_blob(w: &mut Writer, b: &[u8]) {
    w.u32(b.len() as u32).bytes(b);
}

/// Cteni dat s delkou `u32` (nejvys [`MAX_BLOB`]).
pub fn get_blob(r: &mut Reader) -> Result<Vec<u8>> {
    let n = r.u32()? as usize;
    if n > MAX_BLOB {
        return Err(Error::InvalidArgs);
    }
    Ok(r.take(n)?.to_vec())
}

pub fn put_secret(w: &mut Writer, s: &Secret) {
    put_blob(w, s.expose());
}

pub fn get_secret(r: &mut Reader) -> Result<Secret> {
    let n = r.u32()? as usize;
    if n > MAX_SECRET {
        return Err(Error::InvalidArgs);
    }
    Ok(Secret::from_slice(r.take(n)?))
}

pub fn put_u64s(w: &mut Writer, v: &[u64]) {
    w.u32(v.len() as u32);
    for x in v {
        w.u64(*x);
    }
}

pub fn get_u64s(r: &mut Reader) -> Result<Vec<u64>> {
    let n = r.u32()? as usize;
    if n > 1024 {
        return Err(Error::InvalidArgs);
    }
    (0..n).map(|_| r.u64()).collect()
}

/// Odpoved: stav + telo.
pub fn reply_ok(body: &[u8]) -> Vec<u8> {
    let mut w = Writer::new();
    w.i32(0).bytes(body);
    w.finish()
}

pub fn reply_err(e: Error) -> Vec<u8> {
    let mut w = Writer::new();
    w.i32(e.code());
    w.finish()
}

/// Rozebere odpoved: `Ok(telo)` nebo chyba ze stavu.
pub fn parse_reply(b: &[u8]) -> Result<&[u8]> {
    let mut r = Reader::new(b);
    let code = r.i32()?;
    if code == 0 {
        Ok(r.rest())
    } else {
        Err(Error::from_code(code as i64).unwrap_or(Error::IoError))
    }
}

/// Je jmeno pro prihlaseni platne? `a-z`, `0-9`, `-`, `_`, 1-32 znaku,
/// zacina pismenem (je to i jmeno domova `/home/<jmeno>`).
pub fn valid_name(name: &str) -> bool {
    let b = name.as_bytes();
    !b.is_empty()
        && b.len() <= 32
        && b[0].is_ascii_lowercase()
        && b.iter().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == b'-' || *c == b'_')
}

/// Zobrazovane jmeno: 1-64 znaku, bez ridicich znaku.
pub fn valid_display(s: &str) -> bool {
    let n = s.chars().count();
    (1..=64).contains(&n) && !s.chars().any(char::is_control)
}

/// Bezpecnostni fraze: 0-64 znaku, bez ridicich znaku.
pub fn valid_phrase(s: &str) -> bool {
    s.chars().count() <= 64 && !s.chars().any(char::is_control)
}

/// Retezec pro zpravu (prevod chyby UTF-8 na `INVALID_ARGS`).
pub fn get_str(r: &mut Reader) -> Result<String> {
    r.str()
}
