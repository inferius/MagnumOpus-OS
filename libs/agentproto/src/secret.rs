//! Tajemstvi (API klic) a jeho sifrovani vazane na uzivatele.
//!
//! - [`Secret`]: bajty, ktere se nikdy nevypisi (`Debug` = `***`) a po
//!   zahozeni se prepisi nulami.
//! - [`seal`] / [`open`]: ChaCha20-Poly1305. Klic = HKDF-SHA256 z tajemstvi
//!   stroje (32 B, lezi v `/system/agent`) se solí "uzivatel"; pridana data
//!   AEAD = verze, uzivatel, poskytovatel. Zasifrovany klic jednoho
//!   uzivatele nejde pouzit pro jineho (jiny klic i pridana data) ani pro
//!   jineho poskytovatele.
//!
//! **Riziko:** tajemstvi stroje chrani jen prava FS, ne hardware. Kdo
//! precte disk mimo system, klic desifruje (viz `docs/cs/agent.md`).

use alloc::vec::Vec;

use chacha20poly1305::aead::{AeadInPlace, KeyInit};
use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce, Tag};
use hmac::{Hmac, Mac};
use sha2::Sha256;

use crate::{Error, Result};

/// Verze formatu zasifrovaneho klice.
pub const VERSION: u8 = 1;
const MAGIC: &[u8; 4] = b"AGK1";

pub struct Secret(Vec<u8>);

impl Secret {
    pub fn new(data: Vec<u8>) -> Self {
        Self(data)
    }

    pub fn empty() -> Self {
        Self(Vec::new())
    }

    /// Obsah (jen pro odeslani poskytovateli; nikdy do logu).
    pub fn expose(&self) -> &[u8] {
        &self.0
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl Drop for Secret {
    fn drop(&mut self) {
        for b in self.0.iter_mut() {
            // Volatile zapis: prekladac nule nevynecha.
            unsafe { core::ptr::write_volatile(b, 0) };
        }
    }
}

impl core::fmt::Debug for Secret {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("Secret(***)")
    }
}

type HmacSha256 = Hmac<Sha256>;

fn hmac(key: &[u8], parts: &[&[u8]]) -> [u8; 32] {
    let mut m = <HmacSha256 as Mac>::new_from_slice(key).expect("HMAC prijme libovolny klic");
    for p in parts {
        m.update(p);
    }
    m.finalize().into_bytes().into()
}

/// HKDF-SHA256 (RFC 5869) pro jeden blok vystupu (32 B).
fn hkdf(ikm: &[u8], salt: &[u8], info: &[u8]) -> [u8; 32] {
    let prk = hmac(salt, &[ikm]);
    hmac(&prk, &[info, &[1u8]])
}

fn aad(user: u64, provider: &str) -> Vec<u8> {
    let mut a = Vec::new();
    a.extend_from_slice(MAGIC);
    a.push(VERSION);
    a.extend_from_slice(&user.to_le_bytes());
    a.extend_from_slice(provider.as_bytes());
    a
}

fn user_key(machine: &[u8; 32], user: u64) -> Key {
    let k = hkdf(machine, &user.to_le_bytes(), b"magnumopus agentd api key v1");
    *Key::from_slice(&k)
}

/// Zasifruje `plain` pro uzivatele a poskytovatele. `nonce` = 12 nahodnych
/// bajtu (volajici je bere z CSPRNG jadra). Vystup: magie, verze, nonce,
/// zasifrovany text, tag.
pub fn seal(machine: &[u8; 32], user: u64, provider: &str, nonce: &[u8; 12], plain: &[u8]) -> Vec<u8> {
    let cipher = ChaCha20Poly1305::new(&user_key(machine, user));
    let mut buf = plain.to_vec();
    let tag = cipher
        .encrypt_in_place_detached(Nonce::from_slice(nonce), &aad(user, provider), &mut buf)
        .expect("delka v limitu ChaCha20-Poly1305");
    let mut out = Vec::with_capacity(4 + 1 + 12 + buf.len() + 16);
    out.extend_from_slice(MAGIC);
    out.push(VERSION);
    out.extend_from_slice(nonce);
    out.extend_from_slice(&buf);
    out.extend_from_slice(&tag);
    for b in buf.iter_mut() {
        *b = 0;
    }
    out
}

/// Desifruje; spatny uzivatel, poskytovatel nebo poskozena data =
/// `ACCESS_DENIED` (bez rozliseni, at nic neprozradi).
pub fn open(machine: &[u8; 32], user: u64, provider: &str, sealed: &[u8]) -> Result<Secret> {
    if sealed.len() < 4 + 1 + 12 + 16 || &sealed[..4] != MAGIC || sealed[4] != VERSION {
        return Err(Error::Corrupted);
    }
    let nonce = Nonce::from_slice(&sealed[5..17]);
    let (body, tag) = sealed[17..].split_at(sealed.len() - 17 - 16);
    let cipher = ChaCha20Poly1305::new(&user_key(machine, user));
    let mut buf = body.to_vec();
    cipher
        .decrypt_in_place_detached(nonce, &aad(user, provider), &mut buf, Tag::from_slice(tag))
        .map_err(|_| Error::AccessDenied)?;
    Ok(Secret(buf))
}
