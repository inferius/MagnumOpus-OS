//! Klient nad kanaly accountd (prihlasovaci obrazovka, nastaveni uctu,
//! instalator, testy). Odpoved se ceka nejvys [`TIMEOUT`] (overeni
//! bezpecnostnim klicem ceka na dotek uzivatele).

use alloc::string::String;
use alloc::vec::Vec;
use core::time::Duration;

use libos::{Channel, time};
use wire::Reader;
use zeroize::Zeroize;

use crate::{AccountName, AccountSummary, AuthReply, Error, Method, Request, Result, Secret, WhoAmI, decode_list, decode_names, parse_reply};

pub const TIMEOUT: Duration = Duration::from_secs(60);

fn call(ch: &Channel, req: Request) -> Result<Vec<u8>> {
    let mut b = req.encode();
    let w = ch.write(&b, Vec::new());
    b.zeroize();
    w?;
    let m = ch.read(Some(time::deadline_after(TIMEOUT)))?;
    parse_reply(&m.data).map(<[u8]>::to_vec)
}

/// Kanal names (jen cteni zobrazovanych jmen).
pub struct Names(pub Channel);

impl Names {
    pub fn names(&self) -> Result<Vec<AccountName>> {
        decode_names(&call(&self.0, Request::Names)?)
    }
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

    /// Prida klic SSH (radek `authorized_keys`); vraci cislo a otisk.
    pub fn ssh_key_add(&self, password: Secret, line: &str) -> Result<(u32, String)> {
        let b = call(&self.0, Request::SshKeyAdd { password, line: String::from(line) })?;
        let mut r = Reader::new(&b);
        Ok((r.u32()?, r.str()?))
    }

    pub fn ssh_keys(&self) -> Result<Vec<crate::ssh::SshKey>> {
        crate::ssh::decode_keys(&call(&self.0, Request::SshKeyList)?)
    }

    pub fn ssh_key_remove(&self, id: u32) -> Result<()> {
        call(&self.0, Request::SshKeyRemove { id }).map(|_| ())
    }

    /// Vzdaleny pristup uctu (zapnuti chce heslo, vypnuti ne).
    pub fn set_remote(&self, password: Secret, on: bool) -> Result<()> {
        call(&self.0, Request::SetRemote { password, on }).map(|_| ())
    }

    /// Rezim vyvojare (jen vyvojove sestaveni): `set` = `DEV_QUERY` |
    /// `DEV_OFF` | `DEV_ON`; vraci stav.
    #[cfg(feature = "dev-mode")]
    pub fn dev_mode(&self, set: u8) -> Result<bool> {
        call(&self.0, Request::DevMode { set }).map(|b| b.first() == Some(&1))
    }
}

/// Kanal ssh (jen sluzba sshd).
pub struct SshAuth(pub Channel);

impl SshAuth {
    pub fn probe(&self, user: &str, addr: &str, key: &[u8]) -> Result<bool> {
        let b = call(&self.0, Request::SshProbe { user: String::from(user), addr: String::from(addr), key: key.to_vec() })?;
        Ok(b.first() == Some(&1))
    }

    pub fn auth(&self, user: &str, addr: &str, key: &[u8], data: &[u8], sig: &[u8]) -> Result<crate::ssh::SshAuthReply> {
        let req = Request::SshAuth {
            user: String::from(user),
            addr: String::from(addr),
            key: key.to_vec(),
            data: data.to_vec(),
            sig: sig.to_vec(),
        };
        crate::ssh::SshAuthReply::decode(&call(&self.0, req)?)
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
