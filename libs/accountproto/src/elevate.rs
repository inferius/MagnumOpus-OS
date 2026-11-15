//! Ukony spravy pro cely pocitac (`ELEVATE`) a jejich parametry.
//!
//! Ukon potvrdi clovek s roli spravy v duveryhodnem dialogu kompozitoru.
//! Ukony uctu (1-4) provede rovnou sluzba uctu; ostatni (ovladace,
//! aktualizace, disky, firewall, cas) dostanou **grant**: token jen na tento
//! ukon s temito parametry, plati [`GRANT_TTL_SECS`] a jen jednou. Sluzba
//! ukonu ho overi a spotrebuje operaci `CHECK_GRANT` na kanalu `grants`.

use alloc::string::String;
use alloc::vec::Vec;

use wire::{Reader, Writer};

use crate::{Error, Result, Secret, get_secret, put_secret};

/// Kody ukonu.
pub mod action {
    pub const ACCOUNT_CREATE: u32 = 1;
    pub const ACCOUNT_DELETE: u32 = 2;
    pub const ACCOUNT_RESET_PASSWORD: u32 = 3;
    pub const ROLE_SET: u32 = 4;
    /// Prvni ukon jine sluzby (do [`LAST`]).
    pub const FIRST_SERVICE: u32 = 5;
    /// Systemova volba vzhledu (prihlasovaci obrazovka): parametry = text
    /// volby (`theme::Pref::to_words`, napr. `tmavy ne ne`). Sluzba ukonu
    /// je init (prikaz `vzhled --system`).
    pub const APPEARANCE_SYSTEM: u32 = 5;
    /// Zmena systemoveho souboru v `/system` (parametry
    /// [`super::SystemFile`]). Sluzba ukonu je init (prikaz shellu
    /// `system`); `/system/accounts` nejde nikdy.
    pub const SYSTEM_FILE: u32 = 6;
    pub const LAST: u32 = 15;
}

/// Jak dlouho plati grant (s).
pub const GRANT_TTL_SECS: i64 = 60;
/// Rezim spravy (volba v dialogu): s.
pub const ADMIN_MODE_SECS: i64 = 300;
/// Delka tokenu grantu.
pub const TOKEN_LEN: usize = 16;

/// Parametry ukonu uctu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Params {
    AccountCreate { name: String, display: String, password: Secret, admin: bool },
    AccountDelete { uid: u64 },
    ResetPassword { uid: u64, password: Secret },
    RoleSet { uid: u64, on: bool },
}

impl Params {
    pub fn action(&self) -> u32 {
        match self {
            Params::AccountCreate { .. } => action::ACCOUNT_CREATE,
            Params::AccountDelete { .. } => action::ACCOUNT_DELETE,
            Params::ResetPassword { .. } => action::ACCOUNT_RESET_PASSWORD,
            Params::RoleSet { .. } => action::ROLE_SET,
        }
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::new();
        match self {
            Params::AccountCreate { name, display, password, admin } => {
                w.str(name).str(display);
                put_secret(&mut w, password);
                w.u8(*admin as u8);
            }
            Params::AccountDelete { uid } => {
                w.u64(*uid);
            }
            Params::ResetPassword { uid, password } => {
                w.u64(*uid);
                put_secret(&mut w, password);
            }
            Params::RoleSet { uid, on } => {
                w.u64(*uid).u8(*on as u8);
            }
        }
        w.finish()
    }

    /// Parametry ukonu uctu; `NOT_SUPPORTED` = ukon jine sluzby.
    pub fn decode(action: u32, b: &[u8]) -> Result<Self> {
        let mut r = Reader::new(b);
        let p = match action {
            action::ACCOUNT_CREATE => Params::AccountCreate {
                name: r.str()?,
                display: r.str()?,
                password: get_secret(&mut r)?,
                admin: r.u8()? != 0,
            },
            action::ACCOUNT_DELETE => Params::AccountDelete { uid: r.u64()? },
            action::ACCOUNT_RESET_PASSWORD => Params::ResetPassword { uid: r.u64()?, password: get_secret(&mut r)? },
            action::ROLE_SET => Params::RoleSet { uid: r.u64()?, on: r.u8()? != 0 },
            a if (action::FIRST_SERVICE..=action::LAST).contains(&a) => return Err(Error::NotSupported),
            _ => return Err(Error::InvalidArgs),
        };
        if !r.is_empty() {
            return Err(Error::InvalidArgs);
        }
        Ok(p)
    }
}

/// Operace ukonu [`action::SYSTEM_FILE`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileOp {
    /// Zapsat soubor (zalozit nebo prepsat cely obsah).
    Write,
    /// Smazat soubor nebo prazdnou slozku.
    Remove,
    /// Zalozit slozku.
    Mkdir,
}

impl FileOp {
    pub fn to_u8(self) -> u8 {
        match self {
            FileOp::Write => 1,
            FileOp::Remove => 2,
            FileOp::Mkdir => 3,
        }
    }

    pub fn from_u8(v: u8) -> Result<Self> {
        match v {
            1 => Ok(FileOp::Write),
            2 => Ok(FileOp::Remove),
            3 => Ok(FileOp::Mkdir),
            _ => Err(Error::InvalidArgs),
        }
    }
}

/// Parametry ukonu [`action::SYSTEM_FILE`]: operace, cesta a u zapisu
/// delka a otisk obsahu (XXH3-128, spocita sluzba ukonu z dat, ktera
/// zapise). Grant tak plati jen pro tento obsah; dialog ukaze cestu
/// a velikost.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SystemFile {
    pub op: FileOp,
    pub path: String,
    pub len: u64,
    pub hash: u128,
}

impl SystemFile {
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::new();
        w.u8(self.op.to_u8()).str(&self.path).u64(self.len).u64(self.hash as u64).u64((self.hash >> 64) as u64);
        w.finish()
    }

    pub fn decode(b: &[u8]) -> Result<Self> {
        let mut r = Reader::new(b);
        let op = FileOp::from_u8(r.u8()?)?;
        let path = r.str()?;
        let len = r.u64()?;
        let hash = r.u64()? as u128 | (r.u64()? as u128) << 64;
        if !r.is_empty() {
            return Err(Error::InvalidArgs);
        }
        system_path(&path)?;
        Ok(Self { op, path, len, hash })
    }
}

/// Kontrola cesty ukonu [`action::SYSTEM_FILE`]: absolutni cesta pod
/// `/system/` bez prazdnych slozek, `.` a `..`. `ACCESS_DENIED` =
/// `/system/accounts` (databaze uctu: hashe hesel, klic zarizeni), ten
/// nezmeni ani role spravy; `INVALID_ARGS` = spatny tvar.
pub fn system_path(path: &str) -> Result<()> {
    let rest = path.strip_prefix("/system/").ok_or(Error::InvalidArgs)?;
    if rest.is_empty() || path.len() > 1024 {
        return Err(Error::InvalidArgs);
    }
    let mut parts = rest.split('/');
    if parts.clone().any(|p| p.is_empty() || p == "." || p == "..") {
        return Err(Error::InvalidArgs);
    }
    if parts.next() == Some("accounts") {
        return Err(Error::AccessDenied);
    }
    Ok(())
}

/// `CHECK_GRANT` (kanal `grants`): token, ukon, parametry.
pub fn encode_check(token: &[u8], action: u32, params: &[u8]) -> Vec<u8> {
    let mut w = Writer::op(crate::op::CHECK_GRANT);
    crate::put_blob(&mut w, token);
    w.u32(action);
    crate::put_blob(&mut w, params);
    w.finish()
}

pub fn decode_check(b: &[u8]) -> Result<(Vec<u8>, u32, Vec<u8>)> {
    let mut r = Reader::new(b);
    if r.u32()? != crate::op::CHECK_GRANT {
        return Err(Error::NotSupported);
    }
    let t = crate::get_blob(&mut r)?;
    let a = r.u32()?;
    let p = crate::get_blob(&mut r)?;
    if !r.is_empty() || t.len() != TOKEN_LEN {
        return Err(Error::InvalidArgs);
    }
    Ok((t, a, p))
}
