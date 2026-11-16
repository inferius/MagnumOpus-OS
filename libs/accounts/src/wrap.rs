//! Obaly klice domova a obnovovaci klic.
//!
//! Klic domova (256 bitu, nahodny) je zabaleny zvlast pro kazdou metodu:
//! `KEK = HKDF-SHA-256(tajemstvi metody, sul = uid, info = druh/id)`,
//! `obal = ChaCha20-Poly1305(KEK, klic domova, AAD = uid || druh || id)`.
//! Zmena hesla prebali jen jeden obal; data domova se nepresifrovavaji.

use alloc::vec::Vec;

use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce};
use rand_core::CryptoRngCore;
use sha2::{Digest, Sha256};
use wire::{Reader, Writer};
use zeroize::Zeroize;

use abi::{Error, Result};
use accountproto::Secret;

/// Druh obalu.
pub mod kind {
    pub const PASSWORD: u8 = 1;
    pub const PIN: u8 = 2;
    pub const KEY: u8 = 3;
    pub const RECOVERY: u8 = 5;
    /// Klic klienta SSH obaleny heslem (`key_id` = cislo klice;
    /// `client.rs`). Jina domena nez obaly klice domova.
    pub const CLIENT_KEY: u8 = 6;
}

/// Jeden obal klice domova.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Wrap {
    pub kind: u8,
    /// Id bezpecnostniho klice (u ostatnich 0).
    pub key_id: u32,
    pub nonce: [u8; 12],
    /// Zasifrovany klic domova + znacka (48 B).
    pub ct: Vec<u8>,
}

impl Wrap {
    pub fn put(&self, w: &mut Writer) {
        w.u8(self.kind).u32(self.key_id).bytes(&self.nonce);
        accountproto::put_blob(w, &self.ct);
    }

    pub fn get(r: &mut Reader) -> Result<Self> {
        Ok(Wrap {
            kind: r.u8()?,
            key_id: r.u32()?,
            nonce: r.take(12)?.try_into().map_err(|_| Error::Corrupted)?,
            ct: accountproto::get_blob(r)?,
        })
    }
}

fn aad(uid: u64, kind: u8, key_id: u32) -> [u8; 13] {
    let mut a = [0u8; 13];
    a[..8].copy_from_slice(&uid.to_le_bytes());
    a[8] = kind;
    a[9..].copy_from_slice(&key_id.to_le_bytes());
    a
}

fn kek(secret: &[u8], uid: u64, kind: u8, key_id: u32) -> [u8; 32] {
    let mut info = *b"mo-home-wrap-v1/\0\0\0\0\0";
    info[16] = kind;
    info[17..].copy_from_slice(&key_id.to_le_bytes());
    let mut out = [0u8; 32];
    hkdf::Hkdf::<Sha256>::new(Some(&uid.to_le_bytes()), secret)
        .expand(&info, &mut out)
        .expect("32 B je platna delka HKDF");
    out
}

/// Zabali klic domova tajemstvim metody.
pub fn wrap(rng: &mut impl CryptoRngCore, uid: u64, kind: u8, key_id: u32, secret: &[u8], home_key: &Secret) -> Result<Wrap> {
    let mut k = kek(secret, uid, kind, key_id);
    let cipher = ChaCha20Poly1305::new(Key::from_slice(&k));
    k.zeroize();
    let mut nonce = [0u8; 12];
    rng.fill_bytes(&mut nonce);
    let a = aad(uid, kind, key_id);
    let ct = cipher
        .encrypt(Nonce::from_slice(&nonce), Payload { msg: home_key.expose(), aad: &a })
        .map_err(|_| Error::InvalidArgs)?;
    Ok(Wrap { kind, key_id, nonce, ct })
}

/// Rozbali; spatne tajemstvi nebo poskozeny obal = `ACCESS_DENIED`.
pub fn unwrap(uid: u64, w: &Wrap, secret: &[u8]) -> Result<Secret> {
    let mut k = kek(secret, uid, w.kind, w.key_id);
    let cipher = ChaCha20Poly1305::new(Key::from_slice(&k));
    k.zeroize();
    let a = aad(uid, w.kind, w.key_id);
    cipher
        .decrypt(Nonce::from_slice(&w.nonce), Payload { msg: &w.ct, aad: &a })
        .map(Secret::new)
        .map_err(|_| Error::AccessDenied)
}

/// Novy klic domova.
pub fn new_home_key(rng: &mut impl CryptoRngCore) -> Secret {
    let mut k = alloc::vec![0u8; 32];
    rng.fill_bytes(&mut k);
    Secret::new(k)
}

// --- obnovovaci klic ---

/// Abeceda Crockford base32 (bez I, L, O, U).
const ALPHABET: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";
/// Bajtu obnovovaciho klice (160 bitu = 32 znaku).
pub const RECOVERY_BYTES: usize = 20;
/// Kontrolnich znaku (preklep se pozna drive, nez se zkusi rozbalit).
const CHECK_CHARS: usize = 3;

fn base32(b: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut acc: u32 = 0;
    let mut bits = 0;
    for &x in b {
        acc = (acc << 8) | x as u32;
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            out.push(ALPHABET[((acc >> bits) & 31) as usize]);
        }
    }
    if bits > 0 {
        out.push(ALPHABET[((acc << (5 - bits)) & 31) as usize]);
    }
    out
}

fn check(chars: &[u8]) -> [u8; CHECK_CHARS] {
    let h = Sha256::digest(chars);
    let mut c = [0u8; CHECK_CHARS];
    for (i, v) in c.iter_mut().enumerate() {
        *v = ALPHABET[(h[i] & 31) as usize];
    }
    c
}

/// Novy obnovovaci klic: (bajty jako tajemstvi, text pro cloveka
/// `XXXXX-XXXXX-...`, 7 skupin po 5 znacich).
pub fn new_recovery(rng: &mut impl CryptoRngCore) -> (Secret, Secret) {
    let mut raw = [0u8; RECOVERY_BYTES];
    rng.fill_bytes(&mut raw);
    let mut chars = base32(&raw);
    chars.extend_from_slice(&check(&chars));
    let mut text = Vec::new();
    for (i, c) in chars.iter().enumerate() {
        if i > 0 && i % 5 == 0 {
            text.push(b'-');
        }
        text.push(*c);
    }
    chars.zeroize();
    let s = Secret::from_slice(&raw);
    raw.zeroize();
    (s, Secret::new(text))
}

/// Precte obnovovaci klic z textu (velikost pismen, pomlcky a mezery
/// nevadi; O = 0, I/L = 1). `None` = preklep (kontrolni znaky nesedi).
pub fn parse_recovery(text: &[u8]) -> Option<Secret> {
    let mut chars: Vec<u8> = Vec::new();
    for &c in text {
        let c = match c.to_ascii_uppercase() {
            b'-' | b' ' => continue,
            b'O' => b'0',
            b'I' | b'L' => b'1',
            c => c,
        };
        if !ALPHABET.contains(&c) {
            chars.zeroize();
            return None;
        }
        chars.push(c);
    }
    let body_len = (RECOVERY_BYTES * 8).div_ceil(5);
    if chars.len() != body_len + CHECK_CHARS || chars[body_len..] != check(&chars[..body_len]) {
        chars.zeroize();
        return None;
    }
    let mut out = Vec::with_capacity(RECOVERY_BYTES);
    let mut acc: u32 = 0;
    let mut bits = 0;
    for &c in &chars[..body_len] {
        let v = ALPHABET.iter().position(|a| *a == c)? as u32;
        acc = (acc << 5) | v;
        bits += 5;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    chars.zeroize();
    (out.len() == RECOVERY_BYTES).then(|| Secret::new(out))
}
