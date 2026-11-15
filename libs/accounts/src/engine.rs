//! Jadro sluzby uctu: ucty v pameti, ukladani pres [`Store`], zakladani.
//! Overeni je v `auth.rs`, sprava vlastnich metod v `manage.rs`.

use alloc::string::String;
use alloc::vec::Vec;

use rand_core::CryptoRngCore;

use abi::{Error, Result};
use accountproto::{AccountSummary, Secret, flags, valid_display, valid_name};

use crate::device::DeviceKey;
use crate::kdf::{self, KdfParams, Verifier};
use crate::record::{Account, Meta};

/// Kde lezi DB (accountd: soubory na FS, testy: pamet).
pub trait Store {
    /// Metadata (`None` = prazdna DB) a vsechny ucty; poskozeny zaznam =
    /// `Err((jmeno souboru, chyba))`.
    #[allow(clippy::type_complexity)]
    fn load(&mut self) -> Result<(Option<Meta>, Vec<core::result::Result<Account, (String, Error)>>)>;
    fn save(&mut self, a: &Account) -> Result<()>;
    fn save_meta(&mut self, m: &Meta) -> Result<()>;
    fn remove(&mut self, uid: u64) -> Result<()>;
}

/// DB v pameti (testy).
#[derive(Default)]
pub struct MemStore {
    pub meta: Option<Meta>,
    pub files: Vec<(u64, Vec<u8>)>,
    /// Pocet ulozeni (test "pocitadlo se ulozi pred overenim").
    pub saves: usize,
    /// Simulovana chyba zapisu.
    pub fail: bool,
}

impl Store for MemStore {
    fn load(&mut self) -> Result<(Option<Meta>, Vec<core::result::Result<Account, (String, Error)>>)> {
        let list = self
            .files
            .iter()
            .map(|(uid, b)| Account::decode(b).map_err(|e| (alloc::format!("{uid}"), e)))
            .collect();
        Ok((self.meta, list))
    }

    fn save(&mut self, a: &Account) -> Result<()> {
        if self.fail {
            return Err(Error::IoError);
        }
        self.saves += 1;
        let b = a.encode();
        match self.files.iter_mut().find(|(u, _)| *u == a.uid) {
            Some(f) => f.1 = b,
            None => self.files.push((a.uid, b)),
        }
        Ok(())
    }

    fn save_meta(&mut self, m: &Meta) -> Result<()> {
        if self.fail {
            return Err(Error::IoError);
        }
        self.meta = Some(*m);
        Ok(())
    }

    fn remove(&mut self, uid: u64) -> Result<()> {
        self.files.retain(|(u, _)| *u != uid);
        Ok(())
    }
}

/// Nastaveni sluzby.
#[derive(Debug, Clone, Copy)]
pub struct Config {
    pub params: KdfParams,
    /// Vyvojove automaticke prihlaseni povolene (jen ladici build
    /// a soubor `accounts-dev` v initfs).
    pub dev: bool,
}

/// Jmeno vyvojoveho uctu.
pub const DEV_NAME: &str = "vyvojar";

pub struct Engine<S, D, R> {
    pub store: S,
    pub device: D,
    pub rng: R,
    pub cfg: Config,
    pub(crate) accounts: Vec<Account>,
    pub(crate) meta: Meta,
    /// Poskozene zaznamy (jmeno souboru, chyba) - hlasi se, nepouziji.
    pub broken: Vec<(String, Error)>,
}

impl<S: Store, D: DeviceKey, R: CryptoRngCore> Engine<S, D, R> {
    pub fn new(mut store: S, device: D, rng: R, cfg: Config) -> Result<Self> {
        let (meta, list) = store.load()?;
        let mut accounts = Vec::new();
        let mut broken = Vec::new();
        for a in list {
            match a {
                Ok(a) => accounts.push(a),
                Err(e) => broken.push(e),
            }
        }
        accounts.sort_by_key(|a| a.uid);
        let mut meta = meta.unwrap_or_default();
        // Metadata starsi nez ucty (vypadek mezi zapisy): cisla nikdy
        // nepouzit znovu.
        if let Some(max) = accounts.iter().map(|a| a.uid).max() {
            meta.next_uid = meta.next_uid.max(max + 1);
        }
        Ok(Engine { store, device, rng, cfg, accounts, meta, broken })
    }

    pub fn accounts(&self) -> &[Account] {
        &self.accounts
    }

    pub fn account(&self, uid: u64) -> Option<&Account> {
        self.accounts.iter().find(|a| a.uid == uid)
    }

    pub(crate) fn index(&self, uid: u64) -> Result<usize> {
        self.accounts.iter().position(|a| a.uid == uid).ok_or(Error::NotFound)
    }

    pub fn by_name(&self, name: &str) -> Option<&Account> {
        self.accounts.iter().find(|a| a.name == name)
    }

    /// Vypis pro prihlasovaci obrazovku (vyvojovy ucet jen s povolenym
    /// vyvojovym prihlasenim).
    pub fn list(&self) -> Vec<AccountSummary> {
        let weak = !self.device.hardware();
        self.accounts
            .iter()
            .filter(|a| a.flags & flags::DEV == 0 || self.cfg.dev)
            .map(|a| a.summary(weak))
            .collect()
    }

    /// Existuje ucet cloveka (ne vyvojovy)?
    pub fn has_users(&self) -> bool {
        self.accounts.iter().any(|a| a.flags & flags::DEV == 0)
    }

    /// Kolik uctu ma roli spravy.
    /// Ucty lidi s roli spravy (vyvojovy ucet se nepocita: posledni roli
    /// cloveka nejde odebrat ani s nim).
    pub fn admins(&self) -> usize {
        self.accounts.iter().filter(|a| a.flags & flags::ADMIN != 0 && a.flags & (flags::DISABLED | flags::DEV) == 0).count()
    }

    pub(crate) fn new_verifier(&mut self, secret: &[u8]) -> Result<(Verifier, kdf::Derived)> {
        let mut salt = [0u8; 16];
        self.rng.fill_bytes(&mut salt);
        let params = self.cfg.params;
        let d = kdf::derive(&params, secret, &salt)?;
        Ok((Verifier { params, salt, value: *d.verifier() }, d))
    }

    /// Novy ucet (jeste neulozeny: nejdriv domov, pak [`Engine::commit`]).
    pub fn prepare(&mut self, name: &str, display: &str, password: Option<&Secret>, admin: bool, now: i64) -> Result<Account> {
        if !valid_name(name) || !valid_display(display) {
            return Err(Error::InvalidArgs);
        }
        if self.by_name(name).is_some() {
            return Err(Error::AlreadyExists);
        }
        if self.meta.next_uid > accountproto::ids::LAST_USER {
            return Err(Error::NoSpace);
        }
        let mut a = Account::new(self.meta.next_uid, name, display, now);
        if admin {
            a.flags |= flags::ADMIN;
        }
        if let Some(p) = password {
            check_password(p)?;
            let pw = crate::auth::normalize(p)?;
            a.password = Some(self.new_verifier(pw.expose())?.0);
        }
        // Sul pro hmac-secret vznikne az s prvnim bezpecnostnim klicem
        // (`enroll_key`): zalozeni uctu bez hesla (vyvojovy ucet) tak
        // nepotrebuje generator nahody (stroj bez entropie po startu).
        Ok(a)
    }

    /// Prvni ucet z instalace: ma roli spravy. Jen dokud zadny ucet
    /// cloveka neexistuje.
    pub fn prepare_first(&mut self, name: &str, display: &str, password: &Secret, now: i64) -> Result<Account> {
        if self.has_users() {
            return Err(Error::AccessDenied);
        }
        self.prepare(name, display, Some(password), true, now)
    }

    /// Vyvojovy ucet (`None` = uz existuje nebo vyvoj neni povoleny).
    pub fn prepare_dev(&mut self, now: i64) -> Result<Option<Account>> {
        if !self.cfg.dev {
            return Err(Error::AccessDenied);
        }
        if self.by_name(DEV_NAME).is_some() {
            return Ok(None);
        }
        // Role spravy (rozhodnuti 2026-11-15): ukony spravy potvrzuje
        // rezim vyvojare v accountd; plati jen s povolenym vyvojem
        // ([`Engine::is_admin`]).
        let mut a = self.prepare(DEV_NAME, "Vyvojar", None, true, now)?;
        a.flags |= flags::DEV;
        Ok(Some(a))
    }

    /// Vyvojovy ucet z drivejsi verze (bez role spravy) roli dostane. Jen
    /// s povolenym vyvojem.
    pub fn ensure_dev_admin(&mut self) -> Result<()> {
        if !self.cfg.dev {
            return Err(Error::AccessDenied);
        }
        let Some(i) = self.accounts.iter().position(|a| a.flags & flags::DEV != 0) else { return Ok(()) };
        if self.accounts[i].flags & flags::ADMIN != 0 {
            return Ok(());
        }
        self.update(i, |a| a.flags |= flags::ADMIN)
    }

    /// Smi ucet potvrzovat ukony spravy? Vyvojovy ucet jen s povolenym
    /// vyvojem (stejna DB bez vyvojoveho profilu ho nepouzije).
    pub fn is_admin(&self, a: &Account) -> bool {
        a.flags & flags::ADMIN != 0 && a.flags & flags::DISABLED == 0 && (a.flags & flags::DEV == 0 || self.cfg.dev)
    }

    /// Ulozi novy ucet (metadata pred zaznamem: cislo se nepouzije znovu
    /// ani po vypadku mezi zapisy).
    pub fn commit(&mut self, a: Account) -> Result<u64> {
        if self.by_name(&a.name).is_some() || self.account(a.uid).is_some() {
            return Err(Error::AlreadyExists);
        }
        let mut meta = self.meta;
        meta.next_uid = meta.next_uid.max(a.uid + 1);
        self.store.save_meta(&meta)?;
        self.meta = meta;
        self.store.save(&a)?;
        let uid = a.uid;
        self.accounts.push(a);
        self.accounts.sort_by_key(|a| a.uid);
        Ok(uid)
    }

    /// Ulozi zmenu uctu (v pameti i na disku; pri chybe zapisu zustane
    /// v pameti puvodni stav).
    pub(crate) fn update(&mut self, i: usize, f: impl FnOnce(&mut Account)) -> Result<()> {
        let mut a = self.accounts[i].clone();
        f(&mut a);
        self.store.save(&a)?;
        self.accounts[i] = a;
        Ok(())
    }

    pub fn set_phrase(&mut self, uid: u64, phrase: &str) -> Result<()> {
        if !accountproto::valid_phrase(phrase) {
            return Err(Error::InvalidArgs);
        }
        let i = self.index(uid)?;
        self.update(i, |a| a.phrase = String::from(phrase))
    }

    /// Nastavi nebo odebere roli spravy (posledni roli odebrat nejde).
    pub fn set_admin(&mut self, uid: u64, on: bool) -> Result<()> {
        let i = self.index(uid)?;
        // Roli vyvojoveho uctu urcuje vyvojovy profil, ne ukon spravy.
        if self.accounts[i].flags & flags::DEV != 0 {
            return Err(Error::AccessDenied);
        }
        if !on && self.accounts[i].flags & flags::ADMIN != 0 && self.admins() <= 1 {
            return Err(Error::AccessDenied);
        }
        self.update(i, |a| {
            if on {
                a.flags |= flags::ADMIN;
            } else {
                a.flags &= !flags::ADMIN;
            }
        })
    }

    /// Smaze ucet ze DB (domov presune init do kose spravy). Posledni
    /// ucet s roli smazat nejde.
    pub fn delete(&mut self, uid: u64) -> Result<Account> {
        let i = self.index(uid)?;
        if self.accounts[i].flags & flags::ADMIN != 0 && self.admins() <= 1 {
            return Err(Error::AccessDenied);
        }
        self.store.remove(uid)?;
        Ok(self.accounts.remove(i))
    }
}

/// Nejkratsi heslo (znaky). Delka je hlavni sila hesla; pravidla typu
/// "velke pismeno a cislo" se nevynucuji (NIST SP 800-63B).
pub const MIN_PASSWORD: usize = 8;

pub fn check_password(p: &Secret) -> Result<()> {
    let s = core::str::from_utf8(p.expose()).map_err(|_| Error::InvalidArgs)?;
    let n = s.chars().count();
    if n < MIN_PASSWORD || p.len() > 512 {
        return Err(Error::InvalidArgs);
    }
    Ok(())
}

/// PIN: 4-12 cislic.
pub fn check_pin(p: &Secret) -> Result<()> {
    let b = p.expose();
    if (4..=12).contains(&b.len()) && b.iter().all(u8::is_ascii_digit) { Ok(()) } else { Err(Error::InvalidArgs) }
}
