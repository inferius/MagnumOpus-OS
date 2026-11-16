//! Protokol mezi `fsd` a procesem media (`volfs`, docs/cs/media.md).
//!
//! Proces media dostane jen kanal k oddilu vymenneho media (`blockproto`)
//! a tento kanal k fsd. Cizi souborovy system (FAT, NTFS; exFAT pozdeji)
//! rozebira jen on: chyba nebo pad v parseru nedoveryhodnych dat z USB
//! nezasahne fsd, medium jen zmizi.
//!
//! Protokol je **podle cest** a bez stavu: otevrene soubory drzi fsd
//! (handle -> medium + cesta), proces media dostava operace nad cestami.
//! Kazda zprava nese cislo pozadavku (`id`); fsd smi mit rozpracovanych
//! vic pozadavku (z ruznych relaci) a odpovedi paruje podle cisla. Proces
//! media odpovida v poradi prijeti.
//!
//! Cesta je relativni ke koreni media, s `/` na zacatku (`/slozka/soubor`),
//! bez `.` a `..` (normalizuje fsd). Zprava = `id u64`, `op u32`, pole
//! (`libs/wire`, little-endian); odpoved = `id u64`, stav `i32` (0 nebo kod
//! chyby `abi::Error`), pole.
//!
//! Novy souborovy system (exFAT) = jen dalsi implementace v procesu media;
//! protokol ani fsd se nemeni ([`Info::fs`] je jen popis).

#![no_std]

extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;

pub use abi::{Error, Result};
pub use wire::{Reader, Writer};

/// Verze protokolu (`INFO` ji vraci).
pub const VERSION: u32 = 1;

/// Nejvic bajtu dat jednoho `READ`/`WRITE` (vejde se do zpravy kanalu).
pub const MAX_IO: usize = 32 * 1024;

/// Kody operaci.
pub mod op {
    /// `-> Info`.
    pub const INFO: u32 = 1;
    /// cesta -> `Stat`.
    pub const STAT: u32 = 2;
    /// cesta -> pocet u32 + (jmeno, `Stat`)...
    pub const LIST: u32 = 3;
    /// cesta, offset u64, delka u32 -> data (kratsi = konec souboru).
    pub const READ: u32 = 4;
    /// cesta, offset u64, data (zbytek zpravy) -> zapsano u32. Soubor se
    /// pripadne zvetsi (dira = nuly).
    pub const WRITE: u32 = 5;
    /// cesta, druh u8 (`kind`), exkluzivne u8 -> (). Soubor: existujici
    /// a ne exkluzivne = nic (`EXISTS` jen s exkluzivne).
    pub const CREATE: u32 = 6;
    /// cesta -> (). Soubor, nebo prazdna slozka (`NOT_EMPTY`).
    pub const REMOVE: u32 = 7;
    /// z, do, nahradit u8 -> (). V ramci media.
    pub const RENAME: u32 = 8;
    /// cesta, delka u64 -> ().
    pub const SET_LEN: u32 = 9;
    /// -> (). Vse zapsat na medium (vyrovnavaci pamet, FSInfo).
    pub const SYNC: u32 = 10;
    /// -> (). Zapsat, oznacit FS jako ciste a skoncit (bezpecne odpojeni).
    pub const UNMOUNT: u32 = 11;
    /// cesta, priznaky u8 (`open_flags`) -> `Stat`. Otevreni pro fsd:
    /// pripadne vytvoreni (exkluzivne), zkraceni na 0, pak stav.
    pub const OPEN: u32 = 12;
}

/// Priznaky `OPEN`.
pub mod open_flags {
    pub const CREATE: u8 = 1;
    pub const EXCL: u8 = 2;
    pub const TRUNCATE: u8 = 4;
    /// Vytvorit slozku (s `CREATE`).
    pub const DIR: u8 = 8;
}

/// Druh polozky (stejne hodnoty jako `fsproto::kind`).
pub mod kind {
    pub const FILE: u8 = 1;
    pub const DIR: u8 = 2;
}

/// Udaje o mediu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Info {
    pub version: u32,
    /// Souborovy system (`fat`, `ntfs`, ...).
    pub fs: String,
    /// Jmenovka svazku (muze byt prazdna).
    pub label: String,
    pub total_bytes: u64,
    pub free_bytes: u64,
    pub read_only: bool,
}

impl Info {
    pub fn put(&self, w: &mut Writer) {
        w.u32(self.version).str(&self.fs).str(&self.label).u64(self.total_bytes).u64(self.free_bytes).u8(self.read_only as u8);
    }

    pub fn get(r: &mut Reader) -> Result<Self> {
        Ok(Self {
            version: r.u32()?,
            fs: r.str()?,
            label: r.str()?,
            total_bytes: r.u64()?,
            free_bytes: r.u64()?,
            read_only: r.u8()? != 0,
        })
    }
}

/// Udaje o polozce (casy v sekundach od 1970 UTC; FAT ma rozliseni 2 s
/// a mistni cas bez pasma - bere se jako UTC).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Stat {
    pub kind: u8,
    pub size: u64,
    pub mtime: i64,
    pub btime: i64,
    /// Jen pro cteni (atribut FAT, NTFS, nebo cele medium).
    pub read_only: bool,
}

impl Stat {
    pub fn put(&self, w: &mut Writer) {
        w.u8(self.kind).u64(self.size).u64(self.mtime as u64).u64(self.btime as u64).u8(self.read_only as u8);
    }

    pub fn get(r: &mut Reader) -> Result<Self> {
        Ok(Self { kind: r.u8()?, size: r.u64()?, mtime: r.u64()? as i64, btime: r.u64()? as i64, read_only: r.u8()? != 0 })
    }
}

/// Zacatek pozadavku `op` s cislem `id`.
pub fn request(id: u64, op: u32) -> Writer {
    let mut w = Writer::new();
    w.u64(id).u32(op);
    w
}

/// Hlavicka pozadavku: (id, op, zbytek).
pub fn parse_request(m: &[u8]) -> Result<(u64, u32, Reader<'_>)> {
    let mut r = Reader::new(m);
    let id = r.u64()?;
    let op = r.u32()?;
    Ok((id, op, r))
}

/// Odpoved na pozadavek `id`.
pub fn reply(id: u64, res: Result<Writer>) -> Vec<u8> {
    let mut w = Writer::new();
    w.u64(id);
    match res {
        Ok(body) => {
            w.i32(0);
            w.bytes(&body.finish());
        }
        Err(e) => {
            w.i32(e.code() as i32);
        }
    }
    w.finish()
}

/// Rozebere odpoved: (id, telo nebo chyba).
pub fn parse_reply(m: &[u8]) -> Result<(u64, Result<&[u8]>)> {
    let mut r = Reader::new(m);
    let id = r.u64()?;
    let status = r.i32()?;
    if status == 0 {
        Ok((id, Ok(r.rest())))
    } else {
        Ok((id, Err(Error::from_code(status as i64).unwrap_or(Error::IoError))))
    }
}

/// Normalizovana cesta media (`/`, `/a/b`): bez `.`, `..`, prazdnych
/// slozek a znaku, ktere FAT ani NTFS nepovoli (`\ : * ? " < > |`, ridici
/// znaky). Jinak `INVALID_ARGS`.
pub fn normalize(path: &str) -> Result<String> {
    let mut out = String::new();
    for part in path.split('/') {
        if part.is_empty() || part == "." {
            continue;
        }
        if part == ".." || part.chars().any(|c| (c as u32) < 0x20 || "\\:*?\"<>|".contains(c)) {
            return Err(Error::InvalidArgs);
        }
        out.push('/');
        out.push_str(part);
    }
    if out.is_empty() {
        out.push('/');
    }
    Ok(out)
}

#[cfg(test)]
mod tests;
