//! Zaznam uctu na disku (`/db/users/<uid>`) a metadata DB (`/db/meta`).
//!
//! Format: magie `MOAC`, verze `u32`, pole (`libs/wire`), na konci
//! BLAKE2s-256 vsech predchozich bajtu. Poskozeny soubor = `CORRUPTED`
//! (ucet se hlasi jako poskozeny, ostatni funguji).

use alloc::string::String;
use alloc::vec::Vec;

use blake2::{Blake2s256, Digest};
use wire::{Reader, Writer};

use abi::{Error, Result};
use accountproto::{AccountSummary, Method, flags, get_blob, get_u64s, put_blob, put_u64s};

use crate::kdf::Verifier;
use crate::wrap::Wrap;

const MAGIC: &[u8; 4] = b"MOAC";
const META_MAGIC: &[u8; 4] = b"MOAM";
pub const FORMAT: u32 = 1;

/// Registrovany bezpecnostni klic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyRec {
    pub id: u32,
    pub name: String,
    /// Id prihlasovaciho udaje (od klice).
    pub cred_id: Vec<u8>,
    /// Verejny klic COSE.
    pub public: Vec<u8>,
    pub sign_count: u32,
    /// Klic ma PIN (prihlaseni vyzaduje UV).
    pub uv: bool,
    /// Klic umi hmac-secret (obal klice domova).
    pub hmac_secret: bool,
}

/// PIN uctu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PinRec {
    /// Parametry a sul Argon2id; hodnota = HMAC klicem zarizeni.
    pub verifier: Verifier,
    /// Neuspesne pokusy PINu (po [`crate::throttle::PIN_ATTEMPTS`] blokace).
    pub failures: u8,
}

/// Sifrovani domova: obaly klice domova.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Crypt {
    pub wraps: Vec<Wrap>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Account {
    pub uid: u64,
    pub name: String,
    pub display: String,
    pub groups: Vec<u64>,
    pub flags: u32,
    pub phrase: String,
    pub password: Option<Verifier>,
    pub pin: Option<PinRec>,
    pub keys: Vec<KeyRec>,
    pub next_key_id: u32,
    /// Sul pro hmac-secret bezpecnostnich klicu (nuly = jeste zadny klic;
    /// vznikne s prvnim klicem).
    pub key_salt: [u8; 32],
    /// Neuspesne pokusy po sobe (vsechny metody) a cas posledniho.
    pub failures: u32,
    pub last_failure: i64,
    pub home: String,
    pub crypt: Option<Crypt>,
    pub created: i64,
    pub last_login: i64,
}

impl Account {
    pub fn new(uid: u64, name: &str, display: &str, now: i64) -> Self {
        Account {
            uid,
            name: String::from(name),
            display: String::from(display),
            groups: alloc::vec![accountproto::ids::GROUP_USERS],
            flags: 0,
            phrase: String::new(),
            password: None,
            pin: None,
            keys: Vec::new(),
            next_key_id: 1,
            key_salt: [0; 32],
            failures: 0,
            last_failure: 0,
            home: alloc::format!("/home/{name}"),
            crypt: None,
            created: now,
            last_login: 0,
        }
    }

    /// Masky metod, ktere ucet ma.
    pub fn methods(&self) -> u8 {
        let mut m = 0;
        if self.password.is_some() {
            m |= Method::Password.bit();
        }
        if self.pin.is_some() {
            m |= Method::Pin.bit();
        }
        if !self.keys.is_empty() {
            m |= Method::Key.bit();
        }
        if self.flags & flags::DEV != 0 {
            m |= Method::Dev.bit();
        }
        if self.has_wrap(crate::wrap::kind::RECOVERY) {
            m |= Method::Recovery.bit();
        }
        m
    }

    pub fn has_wrap(&self, kind: u8) -> bool {
        self.crypt.as_ref().is_some_and(|c| c.wraps.iter().any(|w| w.kind == kind))
    }

    pub fn summary(&self, pin_weak: bool) -> AccountSummary {
        let mut f = self.flags;
        if self.pin.is_some() && pin_weak {
            f |= flags::PIN_WEAK;
        }
        if self.crypt.is_some() {
            f |= flags::ENCRYPTED;
        }
        AccountSummary { uid: self.uid, name: self.name.clone(), display: self.display.clone(), methods: self.methods(), flags: f }
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::new();
        w.bytes(MAGIC).u32(FORMAT);
        w.u64(self.uid).str(&self.name).str(&self.display);
        put_u64s(&mut w, &self.groups);
        w.u32(self.flags).str(&self.phrase);
        match &self.password {
            Some(v) => {
                w.u8(1);
                v.put(&mut w);
            }
            None => {
                w.u8(0);
            }
        }
        match &self.pin {
            Some(p) => {
                w.u8(1);
                p.verifier.put(&mut w);
                w.u8(p.failures);
            }
            None => {
                w.u8(0);
            }
        }
        w.u32(self.keys.len() as u32);
        for k in &self.keys {
            w.u32(k.id).str(&k.name);
            put_blob(&mut w, &k.cred_id);
            put_blob(&mut w, &k.public);
            w.u32(k.sign_count).u8(k.uv as u8).u8(k.hmac_secret as u8);
        }
        w.u32(self.next_key_id).bytes(&self.key_salt);
        w.u32(self.failures).u64(self.last_failure as u64).str(&self.home);
        match &self.crypt {
            Some(c) => {
                w.u8(1).u32(c.wraps.len() as u32);
                for x in &c.wraps {
                    x.put(&mut w);
                }
            }
            None => {
                w.u8(0);
            }
        }
        w.u64(self.created as u64).u64(self.last_login as u64);
        seal(w.finish())
    }

    pub fn decode(b: &[u8]) -> Result<Self> {
        let body = unseal(b, MAGIC)?;
        let mut r = Reader::new(body);
        if r.u32()? != FORMAT {
            return Err(Error::NotSupported);
        }
        let uid = r.u64()?;
        let name = r.str()?;
        let display = r.str()?;
        let groups = get_u64s(&mut r)?;
        let flags = r.u32()?;
        let phrase = r.str()?;
        let password = match r.u8()? {
            0 => None,
            _ => Some(Verifier::get(&mut r)?),
        };
        let pin = match r.u8()? {
            0 => None,
            _ => Some(PinRec { verifier: Verifier::get(&mut r)?, failures: r.u8()? }),
        };
        let n = r.u32()? as usize;
        if n > 64 {
            return Err(Error::Corrupted);
        }
        let mut keys = Vec::new();
        for _ in 0..n {
            keys.push(KeyRec {
                id: r.u32()?,
                name: r.str()?,
                cred_id: get_blob(&mut r)?,
                public: get_blob(&mut r)?,
                sign_count: r.u32()?,
                uv: r.u8()? != 0,
                hmac_secret: r.u8()? != 0,
            });
        }
        let next_key_id = r.u32()?;
        let key_salt = r.take(32)?.try_into().map_err(|_| Error::Corrupted)?;
        let failures = r.u32()?;
        let last_failure = r.u64()? as i64;
        let home = r.str()?;
        let crypt = match r.u8()? {
            0 => None,
            _ => {
                let n = r.u32()? as usize;
                if n > 128 {
                    return Err(Error::Corrupted);
                }
                Some(Crypt { wraps: (0..n).map(|_| Wrap::get(&mut r)).collect::<Result<_>>()? })
            }
        };
        let created = r.u64()? as i64;
        let last_login = r.u64()? as i64;
        if !r.is_empty() || !accountproto::valid_name(&name) {
            return Err(Error::Corrupted);
        }
        Ok(Account {
            uid,
            name,
            display,
            groups,
            flags,
            phrase,
            password,
            pin,
            keys,
            next_key_id,
            key_salt,
            failures,
            last_failure,
            home,
            crypt,
            created,
            last_login,
        })
    }
}

/// Metadata DB: dalsi volna cisla (cislo smazaneho uctu se nepouzije
/// znovu).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Meta {
    pub next_uid: u64,
    pub next_gid: u64,
}

impl Default for Meta {
    fn default() -> Self {
        Meta { next_uid: accountproto::ids::FIRST_USER, next_gid: accountproto::ids::FIRST_GROUP }
    }
}

impl Meta {
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::new();
        w.bytes(META_MAGIC).u32(FORMAT).u64(self.next_uid).u64(self.next_gid);
        seal(w.finish())
    }

    pub fn decode(b: &[u8]) -> Result<Self> {
        let mut r = Reader::new(unseal(b, META_MAGIC)?);
        if r.u32()? != FORMAT {
            return Err(Error::NotSupported);
        }
        let m = Meta { next_uid: r.u64()?, next_gid: r.u64()? };
        if !r.is_empty() {
            return Err(Error::Corrupted);
        }
        Ok(m)
    }
}

fn seal(mut b: Vec<u8>) -> Vec<u8> {
    let h = Blake2s256::digest(&b);
    b.extend_from_slice(&h);
    b
}

/// Overi magii a kontrolni soucet; vrati telo za magii.
fn unseal<'a>(b: &'a [u8], magic: &[u8; 4]) -> Result<&'a [u8]> {
    if b.len() < 4 + 32 || &b[..4] != magic {
        return Err(Error::Corrupted);
    }
    let (body, sum) = b.split_at(b.len() - 32);
    if Blake2s256::digest(body).as_slice() != sum {
        return Err(Error::Corrupted);
    }
    Ok(&body[4..])
}
