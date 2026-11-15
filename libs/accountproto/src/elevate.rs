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
