//! Vzdaleny pristup (SSH, `docs/cs/ssh.md`): klice SSH uctu, povoleni
//! vzdaleneho pristupu a overeni prihlaseni klicem.
//!
//! - Pridat klic a zapnout vzdaleny pristup chce heslo (i v odemcene
//!   relaci: jinak by kazdy program v relaci mohl pridat cizi klic).
//!   Odebrat klic nebo pristup vypnout jde bez hesla.
//! - Podpis prihlaseni overuje sluzba uctu, ne proces spojeni (ten cte
//!   data ze site). Podepsana data musi patrit tomu uzivateli a klici;
//!   stejny identifikator relace SSH se nepouzije dvakrat.
//! - Zpomaleni vzdalenych pokusu je v pameti podle adresy i jmena a se
//!   zpomalenim mistniho prihlaseni se nesdili: utocnik ze site nesmi
//!   zablokovat majitele u klavesnice.
//! - Heslem se na dalku neprihlasi nikdo (v1 jen klice, i FIDO `sk-`).

use alloc::collections::{BTreeMap, VecDeque};
use alloc::string::String;
use alloc::vec::Vec;

use rand_core::CryptoRngCore;

use abi::{Error, Result};
use accountproto::{Secret, flags};
use ssh::keys::PublicKey;

use crate::device::DeviceKey;
use crate::engine::{Engine, Store};
use crate::record::SshKeyRec;
use crate::throttle;

/// Nejvic klicu SSH na ucet.
pub const MAX_SSH_KEYS: usize = 32;
/// Nejdelsi komentar klice (zbytek se zahodi).
const MAX_COMMENT: usize = 120;
/// Kolik adres a jmen si zpomaleni pamatuje (nejstarsi se zapomene).
pub const GUARD_CAP: usize = 4096;
/// Kolik identifikatoru relaci SSH si pamatuje proti opakovani podpisu.
const SEEN_CAP: usize = 4096;

/// Klic SSH ve vypisu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SshKeyInfo {
    pub id: u32,
    pub algo: String,
    pub fingerprint: String,
    pub comment: String,
    pub added: i64,
}

/// Vysledek vzdaleneho prihlaseni.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemoteAuth {
    Ok { uid: u64 },
    /// Odmitnuto; dalsi pokus z teto adresy / na toto jmeno nejdriv za
    /// `retry_after_ms` (0 = hned). Duvod se neprozradi.
    Denied { retry_after_ms: u64 },
}

/// Pocitadla neuspechu (pocet po sobe, cas posledniho).
#[derive(Default)]
struct Counter {
    map: BTreeMap<String, (u32, i64)>,
    order: VecDeque<String>,
}

impl Counter {
    fn wait_ms(&self, k: &str, now: i64) -> u64 {
        self.map.get(k).map_or(0, |&(f, last)| throttle::wait_ms(f, last, now))
    }

    fn fail(&mut self, k: &str, now: i64) {
        if !self.map.contains_key(k) {
            if self.order.len() >= GUARD_CAP
                && let Some(old) = self.order.pop_front()
            {
                self.map.remove(&old);
            }
            self.order.push_back(String::from(k));
        }
        let e = self.map.entry(String::from(k)).or_insert((0, now));
        e.0 = e.0.saturating_add(1);
        e.1 = now;
    }

    fn clear(&mut self, k: &str) {
        self.map.remove(k);
    }
}

/// Zpomaleni vzdalenych pokusu a pamet pouzitych relaci (jen v pameti
/// sluzby, po restartu prazdne).
#[derive(Default)]
pub struct RemoteGuard {
    by_addr: Counter,
    by_name: Counter,
    seen: VecDeque<Vec<u8>>,
}

impl RemoteGuard {
    pub fn new() -> Self {
        Self::default()
    }

    fn wait_ms(&self, addr: &str, name: &str, now: i64) -> u64 {
        self.by_addr.wait_ms(addr, now).max(self.by_name.wait_ms(name, now))
    }

    fn fail(&mut self, addr: &str, name: &str, now: i64) -> RemoteAuth {
        self.by_addr.fail(addr, now);
        self.by_name.fail(name, now);
        RemoteAuth::Denied { retry_after_ms: self.wait_ms(addr, name, now) }
    }

    /// Identifikator relace SSH jeste nebyl pouzity (a ted uz je).
    fn fresh(&mut self, session_id: &[u8]) -> bool {
        if self.seen.iter().any(|s| s == session_id) {
            return false;
        }
        if self.seen.len() >= SEEN_CAP {
            self.seen.pop_front();
        }
        self.seen.push_back(session_id.to_vec());
        true
    }
}

/// Podepsana data prihlaseni klicem (RFC 4252, 7): relace, `50`, jmeno,
/// sluzba, `publickey`, `TRUE`, typ, blob klice. Vraci identifikator
/// relace, jmeno a blob klice.
pub fn parse_signed(data: &[u8]) -> Result<(&[u8], &str, &[u8])> {
    struct R<'a>(&'a [u8]);
    impl<'a> R<'a> {
        fn s(&mut self) -> Result<&'a [u8]> {
            let (len, rest) = self.0.split_first_chunk::<4>().ok_or(Error::InvalidArgs)?;
            let len = u32::from_be_bytes(*len) as usize;
            if len > rest.len() {
                return Err(Error::InvalidArgs);
            }
            let (v, rest) = rest.split_at(len);
            self.0 = rest;
            Ok(v)
        }
        fn b(&mut self) -> Result<u8> {
            let (&b, rest) = self.0.split_first().ok_or(Error::InvalidArgs)?;
            self.0 = rest;
            Ok(b)
        }
    }
    let mut r = R(data);
    let session = r.s()?;
    if r.b()? != 50 {
        return Err(Error::InvalidArgs);
    }
    let user = core::str::from_utf8(r.s()?).map_err(|_| Error::InvalidArgs)?;
    if r.s()? != b"ssh-connection" || r.s()? != b"publickey" || r.b()? != 1 {
        return Err(Error::InvalidArgs);
    }
    let _algo = r.s()?;
    let blob = r.s()?;
    if !r.0.is_empty() || session.is_empty() {
        return Err(Error::InvalidArgs);
    }
    Ok((session, user, blob))
}

impl<S: Store, D: DeviceKey, R: CryptoRngCore> Engine<S, D, R> {
    /// Prida klic SSH (radek `authorized_keys` / `*.pub`). Chce heslo.
    /// Vraci cislo a otisk klice.
    pub fn ssh_key_add(&mut self, uid: u64, password: &Secret, line: &str, now: i64) -> Result<(u32, String)> {
        let key = PublicKey::parse_line(line)?;
        self.reauth(uid, password, now)?;
        let i = self.index(uid)?;
        let blob = key.blob();
        let a = &self.accounts[i];
        if a.ssh_keys.iter().any(|k| k.blob == blob) {
            return Err(Error::AlreadyExists);
        }
        if a.ssh_keys.len() >= MAX_SSH_KEYS {
            return Err(Error::NoSpace);
        }
        let id = a.next_ssh_id;
        let comment: String = key.comment.chars().filter(|c| !c.is_control()).take(MAX_COMMENT).collect();
        self.update(i, |a| {
            a.ssh_keys.push(SshKeyRec { id, blob, comment, added: now });
            a.next_ssh_id = id.saturating_add(1);
        })?;
        Ok((id, key.fingerprint()))
    }

    /// Klice SSH uctu.
    pub fn ssh_keys(&self, uid: u64) -> Result<Vec<SshKeyInfo>> {
        let a = self.account(uid).ok_or(Error::NotFound)?;
        Ok(a
            .ssh_keys
            .iter()
            .map(|k| SshKeyInfo {
                id: k.id,
                algo: PublicKey::from_blob(&k.blob).map(|p| String::from(p.algo_name())).unwrap_or_default(),
                fingerprint: ssh::keys::fingerprint(&k.blob),
                comment: k.comment.clone(),
                added: k.added,
            })
            .collect())
    }

    /// Odebere klic SSH (bez hesla: odebrani pristup jen ubere).
    pub fn ssh_key_remove(&mut self, uid: u64, id: u32) -> Result<()> {
        let i = self.index(uid)?;
        if !self.accounts[i].ssh_keys.iter().any(|k| k.id == id) {
            return Err(Error::NotFound);
        }
        self.update(i, |a| a.ssh_keys.retain(|k| k.id != id))
    }

    /// Povoli nebo zakaze vzdaleny pristup uctu. Zapnuti chce heslo.
    pub fn set_remote(&mut self, uid: u64, password: &Secret, on: bool, now: i64) -> Result<()> {
        if on {
            self.reauth(uid, password, now)?;
        }
        let i = self.index(uid)?;
        self.update(i, |a| {
            if on {
                a.flags |= flags::REMOTE;
            } else {
                a.flags &= !flags::REMOTE;
            }
        })
    }

    /// Ucet `name`, ktery smi na dalku a ma klic `blob`.
    fn remote_key(&self, name: &str, blob: &[u8]) -> Option<(u64, PublicKey)> {
        let a = self.by_name(name)?;
        if a.flags & flags::REMOTE == 0 || a.flags & flags::DISABLED != 0 {
            return None;
        }
        let k = a.ssh_keys.iter().find(|k| k.blob == blob)?;
        Some((a.uid, PublicKey::from_blob(&k.blob).ok()?))
    }

    /// Dotaz klienta pred podpisem: prijme server tento klic? (Odpovida
    /// jen ano/ne; neznamy ucet = ne, stejne jako cizi klic.)
    pub fn ssh_probe(&self, guard: &RemoteGuard, name: &str, addr: &str, blob: &[u8], now: i64) -> bool {
        guard.wait_ms(addr, name, now) == 0 && self.remote_key(name, blob).is_some()
    }

    /// Prihlaseni klicem: `data` = podepsana data (RFC 4252), `sig` =
    /// podpis (blob protokolu). Klic FIDO s PINem musi prijit i s overenim
    /// uzivatele.
    #[allow(clippy::too_many_arguments)]
    pub fn ssh_auth(&mut self, guard: &mut RemoteGuard, name: &str, addr: &str, blob: &[u8], data: &[u8], sig: &[u8], now: i64) -> RemoteAuth {
        let wait = guard.wait_ms(addr, name, now);
        if wait > 0 {
            return RemoteAuth::Denied { retry_after_ms: wait };
        }
        let ok = (|| -> Result<u64> {
            let (session, user, signed_blob) = parse_signed(data)?;
            if user != name || signed_blob != blob {
                return Err(Error::AccessDenied);
            }
            let (uid, key) = self.remote_key(name, blob).ok_or(Error::AccessDenied)?;
            key.verify(data, sig, false)?;
            if !guard.fresh(session) {
                return Err(Error::AccessDenied);
            }
            Ok(uid)
        })();
        match ok {
            Ok(uid) => {
                guard.by_addr.clear(addr);
                guard.by_name.clear(name);
                RemoteAuth::Ok { uid }
            }
            Err(_) => guard.fail(addr, name, now),
        }
    }
}
