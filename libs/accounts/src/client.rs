//! Klice klienta SSH (`docs/cs/ssh.md`, kapitola Klient): identity, kterymi
//! se uzivatel prihlasuje k jinym serverum. Soukromy klic (seed ed25519)
//! drzi jen sluzba uctu, obaleny heslem uctu (`wrap::kind::CLIENT_KEY`);
//! program `ssh` dostane jen verejny klic a podpis.
//!
//! - Vytvoreni (`ssh-keygen`) a import (`ssh-key import`) chteji heslo.
//! - Odemceni (seedy do pameti sluzby na dobu relace) heslem: pri
//!   prihlaseni heslem u pocitace rovnou, jinak na dotaz.
//! - Zmena hesla klice prebali; nove heslo od spravy nebo obnovovaci klic
//!   je rozbalit neumi: klice zaniknou (bez stareho hesla nejdou otevrit).
//! - Servery, pro ktere uzivatel pouziti klice potvrdil (`hosts`).

use alloc::string::String;
use alloc::vec::Vec;

use abi::{Error, Result};
use accountproto::Secret;
use rand_core::CryptoRngCore;
use ssh::keys::{PublicKey, ed25519_public};

use crate::device::DeviceKey;
use crate::engine::{Engine, Store};
use crate::record::{Account, ClientKeyRec};
use crate::wrap::{self, Wrap, kind};

/// Nejvic klicu klienta na ucet.
pub const MAX_CLIENT_KEYS: usize = 16;
/// Nejvic potvrzenych serveru na klic.
pub const MAX_HOSTS: usize = 256;
/// Nejdelsi jmeno serveru (`host` nebo `[host]:port`).
pub const MAX_HOST_LEN: usize = 255;
const MAX_COMMENT: usize = 120;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientKeyInfo {
    pub id: u32,
    pub public: PublicKey,
    pub fingerprint: String,
    pub comment: String,
    pub added: i64,
    /// Pocet potvrzenych serveru.
    pub hosts: u32,
}

fn clean(s: &str, max: usize) -> String {
    s.chars().filter(|c| !c.is_control()).take(max).collect()
}

/// Platne jmeno serveru pro zaznam potvrzeni.
pub fn valid_host(h: &str) -> bool {
    !h.is_empty() && h.len() <= MAX_HOST_LEN && h.bytes().all(|b| b.is_ascii_graphic())
}

fn seed_of(s: &Secret) -> Result<[u8; 32]> {
    s.expose().try_into().map_err(|_| Error::Corrupted)
}

/// Nove obaly (cislo klice, obal); klice bez noveho obalu zaniknou.
pub(crate) fn apply_rewrap(a: &mut Account, rewrapped: Vec<(u32, Wrap)>) {
    a.client_keys.retain(|k| rewrapped.iter().any(|(id, _)| *id == k.id));
    for k in &mut a.client_keys {
        if let Some((_, w)) = rewrapped.iter().find(|(id, _)| *id == k.id) {
            k.wrap = w.clone();
        }
    }
}

impl<S: Store, D: DeviceKey, R: CryptoRngCore> Engine<S, D, R> {
    /// Novy klic klienta: `seed` = import (soukromy klic ze souboru),
    /// `None` = vygenerovat. Chce heslo. Vraci cislo a verejny klic.
    pub fn client_key_create(&mut self, uid: u64, password: &Secret, comment: &str, seed: Option<&Secret>, now: i64) -> Result<(u32, PublicKey)> {
        self.reauth(uid, password, now)?;
        let i = self.index(uid)?;
        let material = self.password_material(i, password)?.ok_or(Error::AccessDenied)?;
        let a = &self.accounts[i];
        if a.client_keys.len() >= MAX_CLIENT_KEYS {
            return Err(Error::NoSpace);
        }
        let mut seed: [u8; 32] = match seed {
            Some(s) => seed_of(s)?,
            None => {
                let mut b = [0u8; 32];
                self.rng.fill_bytes(&mut b);
                b
            }
        };
        let mut public = ed25519_public(&seed);
        let s = Secret::new(seed.to_vec());
        zeroize::Zeroize::zeroize(&mut seed);
        if a.client_keys.iter().any(|k| k.public == public.key) {
            return Err(Error::AlreadyExists);
        }
        let id = a.next_client_id;
        let w = wrap::wrap(&mut self.rng, uid, kind::CLIENT_KEY, id, material.expose(), &s)?;
        let comment = clean(comment, MAX_COMMENT);
        public.comment = comment.clone();
        self.update(i, |a| {
            a.client_keys.push(ClientKeyRec { id, public: public.key, comment, added: now, wrap: w, hosts: Vec::new() });
            a.next_client_id = id.saturating_add(1);
        })?;
        Ok((id, public))
    }

    /// Klice klienta uctu (jen verejne casti).
    pub fn client_keys(&self, uid: u64) -> Result<Vec<ClientKeyInfo>> {
        let a = self.account(uid).ok_or(Error::NotFound)?;
        Ok(a.client_keys
            .iter()
            .map(|k| {
                let p = ssh::keys::PublicKey { algo: ssh::keys::Algo::Ed25519, key: k.public, comment: k.comment.clone() };
                ClientKeyInfo { fingerprint: p.fingerprint(), public: p, id: k.id, comment: k.comment.clone(), added: k.added, hosts: k.hosts.len() as u32 }
            })
            .collect())
    }

    /// Odebere klic klienta (bez hesla: jen ubira).
    pub fn client_key_remove(&mut self, uid: u64, id: u32) -> Result<()> {
        let i = self.index(uid)?;
        if !self.accounts[i].client_keys.iter().any(|k| k.id == id) {
            return Err(Error::NotFound);
        }
        self.update(i, |a| a.client_keys.retain(|k| k.id != id))
    }

    /// Odemkne klice klienta heslem (se zpomalenim po chybach). Vraci
    /// (cislo, seed).
    pub fn client_unlock(&mut self, uid: u64, password: &Secret, now: i64) -> Result<Vec<(u32, Secret)>> {
        self.reauth(uid, password, now)?;
        self.client_unlock_verified(uid, password)
    }

    /// Jako `client_unlock`, ale bez dalsiho overeni: jen hned po uspesnem
    /// prihlaseni stejnym heslem (sluzba uctu odemkne klice relace).
    pub fn client_unlock_verified(&self, uid: u64, password: &Secret) -> Result<Vec<(u32, Secret)>> {
        let i = self.index(uid)?;
        if self.accounts[i].client_keys.is_empty() {
            return Ok(Vec::new());
        }
        let material = self.password_material(i, password)?.ok_or(Error::AccessDenied)?;
        Ok(self.client_seeds(i, Some(&material)))
    }

    /// Potvrdil uzivatel pouziti klice `id` pro server `host`?
    pub fn client_host_known(&self, uid: u64, id: u32, host: &str) -> bool {
        self.account(uid)
            .and_then(|a| a.client_keys.iter().find(|k| k.id == id))
            .is_some_and(|k| k.hosts.iter().any(|h| h.eq_ignore_ascii_case(host)))
    }

    /// Zapamatuje potvrzeni klice `id` pro `host` (nejstarsi vypadne nad
    /// limitem).
    pub fn client_host_approve(&mut self, uid: u64, id: u32, host: &str) -> Result<()> {
        if !valid_host(host) {
            return Err(Error::InvalidArgs);
        }
        let i = self.index(uid)?;
        if !self.accounts[i].client_keys.iter().any(|k| k.id == id) {
            return Err(Error::NotFound);
        }
        if self.client_host_known(uid, id, host) {
            return Ok(());
        }
        let host = String::from(host);
        self.update(i, |a| {
            if let Some(k) = a.client_keys.iter_mut().find(|k| k.id == id) {
                if k.hosts.len() >= MAX_HOSTS {
                    k.hosts.remove(0);
                }
                k.hosts.push(host);
            }
        })
    }

    /// Seedy klicu, ktere jde rozbalit materialem hesla.
    pub(crate) fn client_seeds(&self, i: usize, material: Option<&Secret>) -> Vec<(u32, Secret)> {
        let Some(m) = material else { return Vec::new() };
        let a = &self.accounts[i];
        a.client_keys.iter().filter_map(|k| wrap::unwrap(a.uid, &k.wrap, m.expose()).ok().map(|s| (k.id, s))).collect()
    }

    /// Nove obaly seedu materialem noveho hesla.
    pub(crate) fn rewrap_clients(&mut self, uid: u64, seeds: &[(u32, Secret)], material: &[u8]) -> Result<Vec<(u32, Wrap)>> {
        seeds.iter().map(|(id, s)| Ok((*id, wrap::wrap(&mut self.rng, uid, kind::CLIENT_KEY, *id, material, s)?))).collect()
    }
}
