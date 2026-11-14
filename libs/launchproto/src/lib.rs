//! Protokol spousteci sluzby `launchd` a registr prikazu.
//!
//! Program, ktery chce spustit jiny program (`std::process::Command`), ma
//! kanal k launchd (stitek `abi::params::tag::LAUNCH`). Spusteni nikdy
//! neda vic, nez ma volajici: vsechny schopnosti dodava v pozadavku sam
//! (svuj job, relaci FS, relaci site, stdio). launchd:
//!
//! 1. najde program ([`Registry`] prikazu z manifestu, `/system/bin`,
//!    initfs; absolutni cesta = soubor s pravem spusteni v relaci FS
//!    volajiciho),
//! 2. zalozi podrizeny job pod jobem volajiciho s limity z manifestu,
//! 3. relaci site zuzi manifestem (bez `[network]` = zadna sit), program
//!    bez manifestu ji dostane, jak ji volajici predal,
//! 4. spusti program s blokem parametru a stdio a vrati handle procesu.
//!
//! Zprava = `op u32`, `id u32` a parametry; odpoved = `id u32`, `status
//! i32` (0 nebo kod `abi::Error`), `zprava str` (popis chyby pro
//! uzivatele, napr. ktery svazek pripojit) a vysledek. Cisla
//! little-endian (`wire`). Popis: `docs/cs/rust-std.md`, kapitola Procesy.

#![cfg_attr(not(test), no_std)]

extern crate alloc;

#[cfg(test)]
mod tests;

use alloc::string::String;
use alloc::vec::Vec;

use abi::{Error, Result};
pub use wire::{Reader, Writer};

/// Operace.
pub mod op {
    /// `program str, priznaky u32, pocet roli u8, role u8..., blok
    /// parametru (zbytek)`; handly ve zprave v poradi roli. Odpoved:
    /// handly `[proces, kanal k potomkovi]`.
    pub const LAUNCH: u32 = 1;
    /// `program str` -> `zdroj u8, cesta str, manifest u8` (kde by se
    /// program vzal; pro `which` a testy).
    pub const LOOKUP: u32 = 2;
    /// Novy kanal k launchd (pro potomka); odpoved: handle kanalu.
    pub const CONNECT: u32 = 3;
}

/// Priznaky `LAUNCH`.
pub mod flags {
    /// Povolit spusteni z ciziho FS (FAT, NTFS bez nasich prav). Bez nej
    /// launchd takovy soubor odmitne. Zatim se cizi FS nerozpoznava
    /// (docs/cs/rust-std.md).
    pub const ALLOW_FOREIGN: u32 = 1 << 0;
    pub const ALL: u32 = ALLOW_FOREIGN;
}

/// Co znamena handle v `LAUNCH`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Role {
    /// Job volajiciho (potomek dostane podrizeny job). Povinny.
    Job = 1,
    /// Relace FS pro potomka (typicky nova relace volajiciho, `CONNECT`).
    Fs = 2,
    /// Relace site pro potomka (jen kdyz ji volajici vyslovne preda).
    Net = 3,
    Stdin = 4,
    Stdout = 5,
    Stderr = 6,
}

impl Role {
    pub fn from_u8(v: u8) -> Option<Self> {
        Some(match v {
            1 => Self::Job,
            2 => Self::Fs,
            3 => Self::Net,
            4 => Self::Stdin,
            5 => Self::Stdout,
            6 => Self::Stderr,
            _ => return None,
        })
    }
}

/// Nejvic roli (handlu) v jednom `LAUNCH`.
pub const MAX_ROLES: usize = 8;

/// Pozadavek `LAUNCH` (bez handlu; ty jdou ve zprave v poradi `roles`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Launch {
    pub program: String,
    pub flags: u32,
    pub roles: Vec<Role>,
    /// Blok parametru (`abi::params`) pro potomka.
    pub params: Vec<u8>,
}

impl Launch {
    pub fn encode(&self, id: u32) -> Result<Vec<u8>> {
        if self.roles.len() > MAX_ROLES || self.flags & !flags::ALL != 0 {
            return Err(Error::InvalidArgs);
        }
        let mut w = Writer::op(op::LAUNCH);
        w.u32(id).str(&self.program).u32(self.flags).u8(self.roles.len() as u8);
        for r in &self.roles {
            w.u8(*r as u8);
        }
        w.bytes(&self.params);
        Ok(w.finish())
    }

    /// Precte telo `LAUNCH` (za `op` a `id`).
    pub fn decode(r: &mut Reader) -> Result<Self> {
        let program = r.str()?;
        let flags = r.u32()?;
        if flags & !flags::ALL != 0 {
            return Err(Error::InvalidArgs);
        }
        let n = r.u8()? as usize;
        if n > MAX_ROLES {
            return Err(Error::InvalidArgs);
        }
        let mut roles = Vec::with_capacity(n);
        for _ in 0..n {
            let role = Role::from_u8(r.u8()?).ok_or(Error::InvalidArgs)?;
            if roles.contains(&role) {
                return Err(Error::InvalidArgs);
            }
            roles.push(role);
        }
        Ok(Self { program, flags, roles, params: r.rest().to_vec() })
    }
}

/// Hlavicka pozadavku: `(op, id)`.
pub fn request_head(r: &mut Reader) -> Result<(u32, u32)> {
    Ok((r.u32()?, r.u32()?))
}

/// Jednoduchy pozadavek (`LOOKUP` s programem, `CONNECT` bez parametru).
pub fn simple(op: u32, id: u32, program: Option<&str>) -> Vec<u8> {
    let mut w = Writer::op(op);
    w.u32(id);
    if let Some(p) = program {
        w.str(p);
    }
    w.finish()
}

/// Odpoved: `id`, vysledek (chyba s popisem) a data.
pub fn reply(id: u32, result: core::result::Result<&[u8], (Error, &str)>) -> Vec<u8> {
    let mut w = Writer::new();
    w.u32(id);
    match result {
        Ok(data) => {
            w.i32(0).str("").bytes(data);
        }
        Err((e, msg)) => {
            w.i32(e.code()).str(msg);
        }
    }
    w.finish()
}

/// Precte odpoved: `(id, Ok(data) | Err((chyba, popis)))`.
pub fn parse_reply(b: &[u8]) -> Result<(u32, core::result::Result<Vec<u8>, (Error, String)>)> {
    let mut r = Reader::new(b);
    let id = r.u32()?;
    let status = r.i32()?;
    let msg = r.str()?;
    if status == 0 {
        return Ok((id, Ok(r.rest().to_vec())));
    }
    let e = Error::from_code(i64::from(status)).ok_or(Error::Corrupted)?;
    Ok((id, Err((e, msg))))
}

/// Odkud se program vezme.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    /// Program v initfs (jmeno souboru).
    Initfs(String),
    /// Soubor v relaci FS volajiciho (absolutni cesta).
    Path(String),
    /// Aplikace nainstalovana na svazku (`/svazky/<svazek>/...`):
    /// `path` je absolutni cesta, `volume` jmeno svazku pro chybu
    /// `NOT_MOUNTED`. **[navrzeno]**: instalace aplikaci zatim neni.
    Volume { volume: String, path: String },
}

impl Source {
    fn kind(&self) -> u8 {
        match self {
            Self::Initfs(_) => 1,
            Self::Path(_) => 2,
            Self::Volume { .. } => 3,
        }
    }

    pub fn encode(&self, w: &mut Writer) {
        w.u8(self.kind());
        match self {
            Self::Initfs(p) | Self::Path(p) => {
                w.str(p);
            }
            Self::Volume { volume, path } => {
                w.str(volume).str(path);
            }
        }
    }

    pub fn decode(r: &mut Reader) -> Result<Self> {
        Ok(match r.u8()? {
            1 => Self::Initfs(r.str()?),
            2 => Self::Path(r.str()?),
            3 => Self::Volume { volume: r.str()?, path: r.str()? },
            _ => return Err(Error::Corrupted),
        })
    }
}

/// Prikaz v registru.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Command {
    pub name: String,
    pub source: Source,
    /// Id aplikace z manifestu (kvuli chybam a prehledu uloh).
    pub app: String,
}

/// Registr prikazu terminalu (`[[command]]` v manifestech).
///
/// Dnes ho launchd plni z manifestu v initfs. Registr nainstalovanych
/// aplikaci (balicky na svazcich, `docs/cs/architektura.md`, Aplikace) ho
/// doplni pres stejne rozhrani [`Registry::add`] **[navrzeno]**.
#[derive(Debug, Default, Clone)]
pub struct Registry {
    commands: Vec<Command>,
}

impl Registry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Prida prikaz; prvni registrace jmena vyhrava (`false` = jmeno uz
    /// existuje, prikaz se neprida).
    pub fn add(&mut self, c: Command) -> bool {
        if self.commands.iter().any(|x| x.name == c.name) {
            return false;
        }
        self.commands.push(c);
        true
    }

    pub fn lookup(&self, name: &str) -> Option<&Command> {
        self.commands.iter().find(|c| c.name == name)
    }

    pub fn commands(&self) -> &[Command] {
        &self.commands
    }
}

/// Je `program` hole jmeno prikazu (ne cesta)?
pub fn is_bare_name(program: &str) -> bool {
    !program.is_empty() && !program.contains('/')
}

/// Kde hledat hole jmeno po registru: adresar systemovych programu.
pub const SYSTEM_BIN: &str = "/system/bin";
