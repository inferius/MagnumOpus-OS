//! Sprava vlastniho uctu: heslo, PIN, bezpecnostni klice, sifrovani
//! domova. Kazda zmena vyzaduje heslo (nebo obnovovaci klic) - i v
//! odemcene relaci, aby ji nemohl zmenit kdokoli u odemceneho pocitace.

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use rand_core::CryptoRngCore;

use abi::{Error, Result};
use accountproto::ctap::{Client, Transport, cose};
use accountproto::{Method, Secret, valid_display};

use crate::auth::{Outcome, normalize};
use crate::device::DeviceKey;
use crate::engine::{Engine, Store, check_password, check_pin};
use crate::kdf::{self, Verifier};
use crate::record::{Crypt, KeyRec, PinRec};
use crate::wrap::{self, Wrap, kind};

/// Nejvic bezpecnostnich klicu na ucet.
pub const MAX_KEYS: usize = 16;

/// Pripravene sifrovani domova (ulozi se az po prijeti klice FS).
pub struct PreparedCrypt {
    pub crypt: Crypt,
    pub home_key: Secret,
    /// Obnovovaci klic pro cloveka (`XXXXX-...`), ukaze se jen jednou.
    pub recovery: Secret,
}

fn replace_wrap(c: &mut Option<Crypt>, w: Wrap) {
    if let Some(c) = c {
        c.wraps.retain(|x| !(x.kind == w.kind && x.key_id == w.key_id));
        c.wraps.push(w);
    }
}

impl<S: Store, D: DeviceKey, R: CryptoRngCore> Engine<S, D, R> {
    /// Overi heslo nebo obnovovaci klic (se zpomalenim jako prihlaseni).
    /// Vrati klic domova, je-li domov sifrovany.
    pub fn reauth(&mut self, uid: u64, secret: &Secret, now: i64) -> Result<Option<Secret>> {
        let m = if wrap::parse_recovery(secret.expose()).is_some() { Method::Recovery } else { Method::Password };
        match self.authenticate(uid, m, secret, None, now)? {
            Outcome::Ok { home_key, .. } => Ok(home_key),
            Outcome::Denied { .. } => Err(Error::AccessDenied),
            Outcome::LockedOut { .. } => Err(Error::ShouldWait),
        }
    }

    fn need_home(&self, uid: u64, home: &Option<Secret>) -> Result<()> {
        // Sifrovany domov bez rozbaleneho klice by obal nove metody
        // nezalozil - radsi odmitnout nez potichu nechat metodu bez obalu.
        if self.account(uid).is_some_and(|a| a.crypt.is_some()) && home.is_none() {
            return Err(Error::Locked);
        }
        Ok(())
    }

    /// Zmena hesla (`old` = heslo nebo obnovovaci klic).
    pub fn set_password(&mut self, uid: u64, old: &Secret, new: &Secret, now: i64) -> Result<()> {
        check_password(new)?;
        let home = self.reauth(uid, old, now)?;
        self.need_home(uid, &home)?;
        let pw = normalize(new)?;
        let (v, d) = self.new_verifier(pw.expose())?;
        let w = match &home {
            Some(k) => Some(wrap::wrap(&mut self.rng, uid, kind::PASSWORD, 0, d.wrap_material(), k)?),
            None => None,
        };
        let i = self.index(uid)?;
        self.update(i, |a| {
            a.password = Some(v);
            if let Some(w) = w {
                replace_wrap(&mut a.crypt, w);
            }
        })
    }

    /// Nastavi PIN (prazdny = smaze).
    pub fn set_pin(&mut self, uid: u64, password: &Secret, pin: &Secret, now: i64) -> Result<()> {
        if !pin.is_empty() {
            check_pin(pin)?;
        }
        let home = self.reauth(uid, password, now)?;
        let i = self.index(uid)?;
        if pin.is_empty() {
            return self.update(i, |a| {
                a.pin = None;
                if let Some(c) = &mut a.crypt {
                    c.wraps.retain(|w| w.kind != kind::PIN);
                }
            });
        }
        self.need_home(uid, &home)?;
        let mut salt = [0u8; 16];
        self.rng.fill_bytes(&mut salt);
        let params = self.cfg.params;
        let (verify, mut wm) = self.pin_values(uid, &params, &salt, pin)?;
        let w = match &home {
            Some(k) => Some(wrap::wrap(&mut self.rng, uid, kind::PIN, 0, &wm, k)?),
            None => None,
        };
        zeroize::Zeroize::zeroize(&mut wm);
        self.update(i, |a| {
            a.pin = Some(PinRec { verifier: Verifier { params, salt, value: verify }, failures: 0 });
            if let Some(w) = w {
                replace_wrap(&mut a.crypt, w);
            }
        })
    }

    /// Registrace bezpecnostniho klice. Klic s PINem vyzaduje `key_pin`
    /// (registrace s UV); u sifrovaneho domova se klice dotkne dvakrat
    /// (registrace a `hmac-secret` pro obal).
    pub fn enroll_key(&mut self, uid: u64, password: &Secret, key_pin: &Secret, name: &str, t: &mut dyn Transport, now: i64) -> Result<u32> {
        if !valid_display(name) {
            return Err(Error::InvalidArgs);
        }
        if self.account(uid).ok_or(Error::NotFound)?.keys.len() >= MAX_KEYS {
            return Err(Error::NoSpace);
        }
        let home = self.reauth(uid, password, now)?;
        self.need_home(uid, &home)?;
        let i = self.index(uid)?;
        let exclude: Vec<Vec<u8>> = self.accounts[i].keys.iter().map(|k| k.cred_id.clone()).collect();
        if self.accounts[i].key_salt == [0; 32] {
            let mut s = [0u8; 32];
            self.rng.fill_bytes(&mut s);
            self.update(i, |a| a.key_salt = s)?;
        }
        let salt = self.accounts[i].key_salt;
        let mut client = Client::new(t).map_err(|_| Error::IoError)?;
        if client.pin_set() && key_pin.is_empty() {
            // Klic s PINem bez tokenu registraci odmitne (PUAT_REQUIRED).
            return Err(Error::AccessDenied);
        }
        let token = if key_pin.is_empty() {
            None
        } else {
            Some(client.pin_token(&mut self.rng, key_pin.expose()).map_err(|_| Error::AccessDenied)?)
        };
        let mut cdh = [0u8; 32];
        self.rng.fill_bytes(&mut cdh);
        let cred = client
            .make_credential(&cdh, &uid.to_le_bytes(), &self.accounts[i].name.clone(), &exclude, token.as_ref())
            .map_err(|e| match e {
                accountproto::ctap::CtapError::Status(accountproto::ctap::msg::status::CREDENTIAL_EXCLUDED) => Error::AlreadyExists,
                _ => Error::IoError,
            })?;
        let id = self.accounts[i].next_key_id;
        let mut rec = KeyRec {
            id,
            name: String::from(name),
            cred_id: cred.id.clone(),
            public: cose::encode_public(&cred.public),
            sign_count: cred.sign_count,
            uv: token.is_some(),
            hmac_secret: cred.hmac_secret,
        };
        let mut w = None;
        if let Some(k) = &home
            && cred.hmac_secret
        {
            rand_core::RngCore::fill_bytes(&mut self.rng, &mut cdh);
            let allow = vec![(cred.id.clone(), cred.public.clone())];
            let a = client
                .get_assertion(&mut self.rng, &cdh, &allow, token.as_ref(), Some(&salt))
                .map_err(|_| Error::IoError)?;
            let mut out = a.hmac_output.ok_or(Error::NotSupported)?;
            rec.sign_count = a.sign_count;
            w = Some(wrap::wrap(&mut self.rng, uid, kind::KEY, id, &out, k)?);
            zeroize::Zeroize::zeroize(&mut out);
        }
        self.update(i, |a| {
            a.keys.push(rec);
            a.next_key_id += 1;
            if let Some(w) = w {
                replace_wrap(&mut a.crypt, w);
            }
        })?;
        Ok(id)
    }

    pub fn remove_key(&mut self, uid: u64, password: &Secret, id: u32, now: i64) -> Result<()> {
        self.reauth(uid, password, now)?;
        let i = self.index(uid)?;
        if !self.accounts[i].keys.iter().any(|k| k.id == id) {
            return Err(Error::NotFound);
        }
        self.update(i, |a| {
            a.keys.retain(|k| k.id != id);
            if let Some(c) = &mut a.crypt {
                c.wraps.retain(|w| !(w.kind == kind::KEY && w.key_id == id));
            }
        })
    }

    /// Nove heslo od spravy (ukon potvrzeny clovekem s roli). Stare heslo
    /// se nezna, takze u sifrovaneho domova obal hesla zanikne: domov
    /// odemkne az majitel obnovovacim klicem (sprava data nevidi).
    pub fn reset_password(&mut self, uid: u64, new: &Secret) -> Result<()> {
        check_password(new)?;
        let pw = normalize(new)?;
        let (v, _) = self.new_verifier(pw.expose())?;
        let i = self.index(uid)?;
        self.update(i, |a| {
            a.password = Some(v);
            a.failures = 0;
            if let Some(c) = &mut a.crypt {
                c.wraps.retain(|w| w.kind != kind::PASSWORD);
            }
        })
    }

    /// Pripravi sifrovani domova: klic domova, obnovovaci klic, obaly pro
    /// heslo a obnovovaci klic. Ulozi se az [`Engine::commit_crypt`] po
    /// prijeti klice FS. PIN a klice dostanou obal pri pristim nastaveni.
    pub fn prepare_crypt(&mut self, uid: u64, password: &Secret, now: i64) -> Result<PreparedCrypt> {
        if self.account(uid).ok_or(Error::NotFound)?.crypt.is_some() {
            return Err(Error::AlreadyExists);
        }
        self.reauth(uid, password, now)?;
        let a = self.account(uid).ok_or(Error::NotFound)?;
        let v = a.password.clone().ok_or(Error::AccessDenied)?;
        let pw = normalize(password)?;
        let d = kdf::derive(&v.params, pw.expose(), &v.salt)?;
        let home_key = wrap::new_home_key(&mut self.rng);
        let (raw, recovery) = wrap::new_recovery(&mut self.rng);
        let wraps = vec![
            wrap::wrap(&mut self.rng, uid, kind::PASSWORD, 0, d.wrap_material(), &home_key)?,
            wrap::wrap(&mut self.rng, uid, kind::RECOVERY, 0, raw.expose(), &home_key)?,
        ];
        Ok(PreparedCrypt { crypt: Crypt { wraps }, home_key, recovery })
    }

    pub fn commit_crypt(&mut self, uid: u64, crypt: Crypt) -> Result<()> {
        let i = self.index(uid)?;
        self.update(i, |a| a.crypt = Some(crypt))
    }
}
