//! Klient nad kanaly accountd (prihlasovaci obrazovka, nastaveni uctu,
//! instalator, testy). Odpoved se ceka nejvys [`TIMEOUT`] (overeni
//! bezpecnostnim klicem ceka na dotek uzivatele).

use alloc::string::String;
use alloc::vec::Vec;
use core::time::Duration;

use libos::{Channel, time};
use wire::Reader;
use zeroize::Zeroize;

use crate::{AccountSummary, AuthReply, Error, Method, Request, Result, Secret, WhoAmI, decode_list, parse_reply};

pub const TIMEOUT: Duration = Duration::from_secs(60);

fn call(ch: &Channel, req: Request) -> Result<Vec<u8>> {
    let mut b = req.encode();
    let w = ch.write(&b, Vec::new());
    b.zeroize();
    w?;
    let m = ch.read(Some(time::deadline_after(TIMEOUT)))?;
    parse_reply(&m.data).map(<[u8]>::to_vec)
}

/// Kanal login.
pub struct Login(pub Channel);

impl Login {
    pub fn list(&self) -> Result<Vec<AccountSummary>> {
        decode_list(&call(&self.0, Request::List)?)
    }

    pub fn phrase(&self, uid: u64) -> Result<String> {
        Reader::new(&call(&self.0, Request::Phrase { uid })?).str()
    }

    pub fn auth(&self, uid: u64, method: Method, secret: Secret) -> Result<AuthReply> {
        AuthReply::decode(&call(&self.0, Request::Auth { uid, method, secret })?)
    }

    pub fn unlock(&self, session: u64, method: Method, secret: Secret) -> Result<AuthReply> {
        AuthReply::decode(&call(&self.0, Request::Unlock { session, method, secret })?)
    }
}

/// Kanal setup (instalator).
pub struct Setup(pub Channel);

impl Setup {
    pub fn create_first(&self, name: &str, display: &str, password: Secret) -> Result<u64> {
        let req = Request::CreateFirst { name: String::from(name), display: String::from(display), password };
        Reader::new(&call(&self.0, req)?).u64()
    }
}

/// Kanal self (vlastni ucet v relaci).
pub struct SelfService(pub Channel);

impl SelfService {
    pub fn whoami(&self) -> Result<WhoAmI> {
        WhoAmI::decode(&call(&self.0, Request::WhoAmI)?)
    }

    pub fn set_password(&self, old: Secret, new: Secret) -> Result<()> {
        call(&self.0, Request::SetPassword { old, new }).map(|_| ())
    }

    pub fn set_pin(&self, password: Secret, pin: Secret) -> Result<()> {
        call(&self.0, Request::SetPin { password, pin }).map(|_| ())
    }

    pub fn set_phrase(&self, phrase: &str) -> Result<()> {
        call(&self.0, Request::SetPhrase { phrase: String::from(phrase) }).map(|_| ())
    }

    /// Zapne sifrovani domova; vrati obnovovaci klic (jen jednou).
    pub fn encrypt_home(&self, password: Secret) -> Result<Secret> {
        call(&self.0, Request::EncryptHome { password }).map(Secret::new)
    }

    pub fn lock(&self) -> Result<()> {
        call(&self.0, Request::Lock).map(|_| ())
    }

    pub fn logout(&self) -> Result<()> {
        call(&self.0, Request::Logout).map(|_| ())
    }

    pub fn elevate(&self, action: u32, params: Vec<u8>) -> Result<Vec<u8>> {
        call(&self.0, Request::Elevate { action, params })
    }
}

/// Odpoved overeni jako vysledek: `Ok(relace)`, jinak `ACCESS_DENIED`
/// (spatne) nebo `SHOULD_WAIT` (zpomaleni).
pub fn session_of(r: AuthReply) -> Result<u64> {
    match r {
        AuthReply::Ok { session } => Ok(session),
        AuthReply::Denied { .. } => Err(Error::AccessDenied),
        AuthReply::LockedOut { .. } => Err(Error::ShouldWait),
    }
}
