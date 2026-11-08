//! Overeni: heslo, PIN s klicem zarizeni, bezpecnostni klic (CTAP2),
//! vyvojove prihlaseni, obnovovaci klic. Zpomaleni po chybach, pocitadlo
//! se ulozi PRED overenim (pad uprostred pokusu ho nevynuluje).

use alloc::vec::Vec;

use rand_core::CryptoRngCore;

use abi::{Error, Result};
use accountproto::ctap::{Client, PublicKey, Transport, cose};
use accountproto::{AuthReply, Method, Secret, flags};

use crate::device::DeviceKey;
use crate::engine::{Engine, Store};
use crate::kdf::{self, KdfParams};
use crate::record::Account;
use crate::throttle::{self, PIN_ATTEMPTS};
use crate::wrap::{self, kind};

/// Vysledek overeni.
#[derive(Debug)]
pub enum Outcome {
    /// Uspech; klic domova, kdyz je domov sifrovany a metoda ma obal.
    Ok { uid: u64, home_key: Option<Secret> },
    Denied { retry_after_ms: u64, pin_left: u8 },
    /// Pokus v dobe zpomaleni: nic se nepocitalo ani neoverovalo.
    LockedOut { retry_after_ms: u64 },
}

impl Outcome {
    pub fn reply(&self, session: u64) -> AuthReply {
        match self {
            Outcome::Ok { .. } => AuthReply::Ok { session },
            Outcome::Denied { retry_after_ms, pin_left } => AuthReply::Denied { retry_after_ms: *retry_after_ms, pin_left: *pin_left },
            Outcome::LockedOut { retry_after_ms } => AuthReply::LockedOut { retry_after_ms: *retry_after_ms },
        }
    }

    pub fn is_ok(&self) -> bool {
        matches!(self, Outcome::Ok { .. })
    }
}

/// Heslo v normalizovanem tvaru NFC (stejne heslo napsane ruznymi
/// metodami vstupu = stejne bajty).
pub fn normalize(p: &Secret) -> Result<Secret> {
    use unicode_normalization::UnicodeNormalization;
    let s = core::str::from_utf8(p.expose()).map_err(|_| Error::InvalidArgs)?;
    let n: alloc::string::String = s.nfc().collect();
    Ok(Secret::new(n.into_bytes()))
}

/// Vysledek overeni jedne metody (bez zpomaleni).
pub(crate) enum Check {
    Ok(Option<Secret>),
    Bad,
}

impl<S: Store, D: DeviceKey, R: CryptoRngCore> Engine<S, D, R> {
    /// Overi ucet `uid` metodou `method`. `key` = prenos k bezpecnostnimu
    /// klici (jen u `Method::Key`; secret je pak PIN klice).
    pub fn authenticate(
        &mut self,
        uid: u64,
        method: Method,
        secret: &Secret,
        key: Option<&mut dyn Transport>,
        now: i64,
    ) -> Result<Outcome> {
        let Ok(i) = self.index(uid) else {
            // Neznamy ucet: stejna prace jako u znameho (doba odpovedi
            // neprozradi, ze ucet neexistuje).
            let _ = kdf::derive(&self.cfg.params, secret.expose(), &[0u8; 16]);
            return Ok(Outcome::Denied { retry_after_ms: 0, pin_left: 0 });
        };
        let a = &self.accounts[i];
        if a.flags & flags::DISABLED != 0 {
            return Ok(Outcome::Denied { retry_after_ms: 0, pin_left: 0 });
        }
        let wait = throttle::wait_ms(a.failures, a.last_failure, now);
        if wait > 0 {
            return Ok(Outcome::LockedOut { retry_after_ms: wait });
        }
        if method == Method::Dev {
            // Bez tajemstvi, bez pocitani: bud je povolene, nebo ne.
            return Ok(if self.cfg.dev && a.flags & flags::DEV != 0 {
                self.success(i, now, false)?;
                Outcome::Ok { uid, home_key: None }
            } else {
                Outcome::Denied { retry_after_ms: 0, pin_left: 0 }
            });
        }
        if method == Method::Pin {
            match &a.pin {
                None => return Ok(Outcome::Denied { retry_after_ms: 0, pin_left: 0 }),
                Some(p) if p.failures >= PIN_ATTEMPTS => return Ok(Outcome::Denied { retry_after_ms: 0, pin_left: 0 }),
                _ => {}
            }
        }
        // Pocitadlo nahoru a ulozit PRED overenim.
        self.update(i, |a| {
            a.failures = a.failures.saturating_add(1);
            a.last_failure = now;
            if method == Method::Pin
                && let Some(p) = &mut a.pin
            {
                p.failures += 1;
            }
        })?;
        let check = match method {
            Method::Password => self.check_password(i, secret)?,
            Method::Pin => self.check_pin(i, secret)?,
            Method::Key => match key {
                Some(t) => self.check_key(i, secret, t)?,
                None => return Err(Error::NotFound),
            },
            Method::Recovery => self.check_recovery(i, secret)?,
            Method::Dev => unreachable!("vyrizeno vyse"),
        };
        match check {
            Check::Ok(home_key) => {
                self.success(i, now, method == Method::Password || method == Method::Key)?;
                if method == Method::Password {
                    self.maybe_rehash(i, secret)?;
                }
                Ok(Outcome::Ok { uid, home_key })
            }
            Check::Bad => {
                let a = &self.accounts[i];
                let pin_left = match (method, &a.pin) {
                    (Method::Pin, Some(p)) => PIN_ATTEMPTS.saturating_sub(p.failures),
                    _ => 0,
                };
                Ok(Outcome::Denied { retry_after_ms: throttle::wait_ms(a.failures, a.last_failure, now), pin_left })
            }
        }
    }

    /// Vynuluje pocitadla; `strong` (heslo, klic) odblokuje i PIN.
    fn success(&mut self, i: usize, now: i64, strong: bool) -> Result<()> {
        self.update(i, |a| {
            a.failures = 0;
            a.last_login = now;
            if let Some(p) = &mut a.pin
                && (strong || p.failures < PIN_ATTEMPTS)
            {
                p.failures = 0;
            }
        })
    }

    pub(crate) fn check_password(&self, i: usize, secret: &Secret) -> Result<Check> {
        let a = &self.accounts[i];
        let Some(v) = &a.password else {
            let _ = kdf::derive(&self.cfg.params, secret.expose(), &[0u8; 16]);
            return Ok(Check::Bad);
        };
        let pw = normalize(secret).unwrap_or_default();
        let d = kdf::derive(&v.params, pw.expose(), &v.salt)?;
        if !kdf::ct_eq(d.verifier(), &v.value) {
            return Ok(Check::Bad);
        }
        Ok(Check::Ok(self.unwrap_home(a, kind::PASSWORD, 0, d.wrap_material())))
    }

    /// Overovaci hodnota PINu: HMAC klicem zarizeni nad Argon2id.
    pub(crate) fn pin_values(&self, uid: u64, params: &KdfParams, salt: &[u8], pin: &Secret) -> Result<([u8; 32], [u8; 32])> {
        let d = kdf::derive(params, pin.expose(), salt)?;
        let mut data = Vec::with_capacity(40);
        data.extend_from_slice(&uid.to_le_bytes());
        data.extend_from_slice(d.verifier());
        let verify = self.device.mac(b"pin-verify", &data);
        data.truncate(8);
        data.extend_from_slice(d.wrap_material());
        let wrap = self.device.mac(b"pin-wrap", &data);
        zeroize::Zeroize::zeroize(&mut data);
        Ok((verify, wrap))
    }

    fn check_pin(&self, i: usize, secret: &Secret) -> Result<Check> {
        let a = &self.accounts[i];
        let Some(p) = &a.pin else { return Ok(Check::Bad) };
        let (verify, mut wrap) = self.pin_values(a.uid, &p.verifier.params, &p.verifier.salt, secret)?;
        let ok = kdf::ct_eq(&verify, &p.verifier.value);
        let r = if ok { Check::Ok(self.unwrap_home(a, kind::PIN, 0, &wrap)) } else { Check::Bad };
        zeroize::Zeroize::zeroize(&mut wrap);
        Ok(r)
    }

    fn check_recovery(&self, i: usize, secret: &Secret) -> Result<Check> {
        let a = &self.accounts[i];
        let Some(raw) = wrap::parse_recovery(secret.expose()) else { return Ok(Check::Bad) };
        let Some(w) = a.crypt.as_ref().and_then(|c| c.wraps.iter().find(|w| w.kind == kind::RECOVERY)) else {
            return Ok(Check::Bad);
        };
        Ok(match wrap::unwrap(a.uid, w, raw.expose()) {
            Ok(k) => Check::Ok(Some(k)),
            Err(_) => Check::Bad,
        })
    }

    /// Bezpecnostni klic: prihlaseni s overenim podpisu, UV podle toho,
    /// zda ma klic PIN, `hmac-secret` pro klic domova, pocitadlo podpisu.
    fn check_key(&mut self, i: usize, key_pin: &Secret, t: &mut dyn Transport) -> Result<Check> {
        let a = &self.accounts[i];
        if a.keys.is_empty() {
            return Ok(Check::Bad);
        }
        let allow: Vec<(Vec<u8>, PublicKey)> = a
            .keys
            .iter()
            .filter_map(|k| {
                let v = accountproto::ctap::cbor::decode(&k.public).ok()?;
                Some((k.cred_id.clone(), cose::PublicKey::from_cose(&v)?))
            })
            .collect();
        let salt = a.key_salt;
        let encrypted = a.crypt.is_some();
        let mut client = match Client::new(t) {
            Ok(c) => c,
            Err(_) => return Err(Error::IoError),
        };
        let token = if key_pin.is_empty() {
            None
        } else {
            match client.pin_token(&mut self.rng, key_pin.expose()) {
                Ok(t) => Some(t),
                Err(_) => return Ok(Check::Bad),
            }
        };
        let mut cdh = [0u8; 32];
        self.rng.fill_bytes(&mut cdh);
        let use_salt = encrypted.then_some(&salt);
        let assertion = match client.get_assertion(&mut self.rng, &cdh, &allow, token.as_ref(), use_salt) {
            Ok(x) => x,
            Err(_) => return Ok(Check::Bad),
        };
        let Some(k) = self.accounts[i].keys.iter().position(|k| k.cred_id == assertion.id) else {
            return Ok(Check::Bad);
        };
        let rec = &self.accounts[i].keys[k];
        // Klic registrovany s PINem musi overit uzivatele (UV).
        if rec.uv && assertion.flags & accountproto::ctap::msg::flag::UV == 0 {
            return Ok(Check::Bad);
        }
        // Pocitadlo podpisu musi rust (0 = klic pocitadlo nevede).
        if (assertion.sign_count != 0 || rec.sign_count != 0) && assertion.sign_count <= rec.sign_count {
            return Ok(Check::Bad);
        }
        let key_id = rec.id;
        let count = assertion.sign_count;
        let home = match assertion.hmac_output {
            Some(mut out) => {
                let k = self.unwrap_home(&self.accounts[i], kind::KEY, key_id, &out);
                zeroize::Zeroize::zeroize(&mut out);
                k
            }
            None => None,
        };
        self.update(i, |a| a.keys[k].sign_count = count)?;
        Ok(Check::Ok(home))
    }

    /// Rozbali klic domova obalem dane metody (`None` = domov neni
    /// sifrovany nebo metoda obal nema).
    pub(crate) fn unwrap_home(&self, a: &Account, k: u8, key_id: u32, material: &[u8]) -> Option<Secret> {
        let w = a.crypt.as_ref()?.wraps.iter().find(|w| w.kind == k && w.key_id == key_id)?;
        wrap::unwrap(a.uid, w, material).ok()
    }

    /// Hodnota hesla se slabsimi parametry se po prihlaseni prepocita.
    fn maybe_rehash(&mut self, i: usize, secret: &Secret) -> Result<()> {
        let Some(v) = &self.accounts[i].password else { return Ok(()) };
        if !v.params.weaker_than(&self.cfg.params) || self.accounts[i].crypt.is_some() {
            // U sifrovaneho domova by se musel prebalit i obal; udela to
            // pristi zmena hesla.
            return Ok(());
        }
        let pw = normalize(secret)?;
        let (nv, _) = self.new_verifier(pw.expose())?;
        self.update(i, |a| a.password = Some(nv))
    }
}
