//! Spousteni programu ze shellu: relace shellu (`shsess`) -> init
//! (`docs/cs/navrh-shell.md`, kapitola Programy).
//!
//! Shell nema sit ani pravo razit relace: posle initu jen jmeno programu,
//! argumenty, pracovni slozku a pripojene cesty (zuzeni prav uzivatele).
//! Init overi relaci, zalozi programu relaci FS s identitou uzivatele
//! relace, sit preda jen programu s `[network]` v manifestu (zuzi ji
//! launchd, firewall rozhoduje podle id aplikace) a spusti ho pres launchd
//! v jobu relace.
//!
//! Zprava na kanalu sluzby: `op u32 (EXEC), program str, cwd str, sit u8,
//! argc u32,
//! argumenty str..., pocet cest u32, (jmeno str, cil str, jen cteni u8)...`.
//! Handly: `[odpoved, stdin, stdout, stderr]` (stdio = konce kanalu pro
//! program, protokol proudu bajtu jako `std`). Odpoved na kanalu odpovedi:
//! `status i32 (0 nebo kod abi::Error), popis str` a pri uspechu handle
//! procesu (cekani a ukonceni).

use alloc::string::String;
use alloc::vec::Vec;

use abi::{Error, Result};

use crate::{Reader, Writer};

/// Operace kanalu sluzby.
pub const EXEC: u32 = 1;

/// Nejvic argumentu a pripojenych cest (pozadavek se vejde do zpravy).
pub const MAX_ARGS: usize = 256;
pub const MAX_MOUNTS: usize = 64;

/// Pripojena cesta programu: (jmeno v relaci, cesta ve FS, jen pro cteni).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mount {
    pub name: String,
    pub target: String,
    pub read_only: bool,
}

/// Pozadavek `EXEC` (bez handlu).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Exec {
    /// Hole jmeno prikazu (v1 jen programy z registru launchd: initfs,
    /// `/system/bin`).
    pub program: String,
    pub cwd: String,
    pub args: Vec<String>,
    /// Prazdne = cely FS s pravy uzivatele.
    pub mounts: Vec<Mount>,
    /// Smi program dostat sit (skript s `needs` bez `net` = ne). I pak ji
    /// dostane jen program s `[network]` v manifestu.
    pub net: bool,
}

impl Exec {
    pub fn encode(&self) -> Result<Vec<u8>> {
        if self.args.len() > MAX_ARGS || self.mounts.len() > MAX_MOUNTS {
            return Err(Error::InvalidArgs);
        }
        let mut w = Writer::op(EXEC);
        w.str(&self.program).str(&self.cwd).u8(self.net as u8).u32(self.args.len() as u32);
        for a in &self.args {
            w.str(a);
        }
        w.u32(self.mounts.len() as u32);
        for m in &self.mounts {
            w.str(&m.name).str(&m.target).u8(m.read_only as u8);
        }
        Ok(w.finish())
    }

    pub fn decode(b: &[u8]) -> Result<Self> {
        let mut r = Reader::new(b);
        if r.u32()? != EXEC {
            return Err(Error::NotSupported);
        }
        let program = r.str()?;
        let cwd = r.str()?;
        let net = match r.u8()? {
            0 => false,
            1 => true,
            _ => return Err(Error::InvalidArgs),
        };
        let n = r.u32()? as usize;
        if n > MAX_ARGS {
            return Err(Error::InvalidArgs);
        }
        let args = (0..n).map(|_| r.str()).collect::<Result<Vec<_>>>()?;
        let n = r.u32()? as usize;
        if n > MAX_MOUNTS {
            return Err(Error::InvalidArgs);
        }
        let mut mounts = Vec::with_capacity(n);
        for _ in 0..n {
            let name = r.str()?;
            let target = r.str()?;
            let read_only = match r.u8()? {
                0 => false,
                1 => true,
                _ => return Err(Error::InvalidArgs),
            };
            mounts.push(Mount { name, target, read_only });
        }
        if !r.is_empty() {
            return Err(Error::InvalidArgs);
        }
        Ok(Self { program, cwd, args, mounts, net })
    }
}

/// Odpoved: `Ok(())` (handle procesu jde ve zprave) nebo chyba s popisem.
pub fn reply(result: core::result::Result<(), (Error, &str)>) -> Vec<u8> {
    let mut w = Writer::new();
    match result {
        Ok(()) => {
            w.i32(0).str("");
        }
        Err((e, msg)) => {
            w.i32(e.code()).str(msg);
        }
    }
    w.finish()
}

pub fn parse_reply(b: &[u8]) -> Result<core::result::Result<(), (Error, String)>> {
    let mut r = Reader::new(b);
    let status = r.i32()?;
    let msg = r.str()?;
    if status == 0 {
        return Ok(Ok(()));
    }
    Ok(Err((Error::from_code(i64::from(status)).ok_or(Error::Corrupted)?, msg)))
}
