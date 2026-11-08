//! Protokoly PIN/UV (CTAP 2.1, kap. 6.5.6 a 6.5.7): sdilene tajemstvi
//! z ECDH P-256 a z nej sifrovani (AES-256-CBC) a autentizace
//! (HMAC-SHA-256). Pouziva je platforma (accountd) i simulovany klic.
//!
//! - **Protokol 1:** tajemstvi = SHA-256(Z), IV = 0, autentizace = prvnich
//!   16 B HMAC.
//! - **Protokol 2:** tajemstvi = HKDF-SHA-256(Z, "CTAP2 HMAC key") ||
//!   HKDF-SHA-256(Z, "CTAP2 AES key"), nahodny IV pred sifrovym textem,
//!   autentizace = cely HMAC (32 B).

use alloc::vec::Vec;

use aes::Aes256;
use cbc::cipher::block_padding::NoPadding;
use cbc::cipher::{BlockDecryptMut, BlockEncryptMut, KeyIvInit};
use hmac::{Hmac, Mac};
use rand_core::CryptoRngCore;
use sha2::{Digest, Sha256};
use zeroize::Zeroize;

use super::cbor::Value;
use super::cose;

type HmacSha256 = Hmac<Sha256>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Protocol {
    V1,
    V2,
}

impl Protocol {
    pub fn from_u64(v: u64) -> Option<Self> {
        match v {
            1 => Some(Protocol::V1),
            2 => Some(Protocol::V2),
            _ => None,
        }
    }

    pub fn number(self) -> u64 {
        match self {
            Protocol::V1 => 1,
            Protocol::V2 => 2,
        }
    }
}

/// Sdilene tajemstvi protokolu PIN/UV (vynuluje se pri zahozeni).
pub struct Shared {
    pub proto: Protocol,
    /// V1: 32 B, V2: 64 B (klic HMAC || klic AES).
    key: Vec<u8>,
}

impl Drop for Shared {
    fn drop(&mut self) {
        self.key.zeroize();
    }
}

fn hkdf32(z: &[u8], info: &[u8]) -> [u8; 32] {
    let mut out = [0u8; 32];
    // Sul: 32 nulovych bajtu (CTAP 2.1, 6.5.7).
    hkdf::Hkdf::<Sha256>::new(Some(&[0u8; 32]), z)
        .expand(info, &mut out)
        .expect("32 B je platna delka HKDF");
    out
}

impl Shared {
    /// KDF ze souradnice x bodu ECDH (`z`).
    pub fn derive(proto: Protocol, z: &[u8]) -> Shared {
        let key = match proto {
            Protocol::V1 => Sha256::digest(z).to_vec(),
            Protocol::V2 => {
                let mut k = hkdf32(z, b"CTAP2 HMAC key").to_vec();
                k.extend_from_slice(&hkdf32(z, b"CTAP2 AES key"));
                k
            }
        };
        Shared { proto, key }
    }

    fn aes_key(&self) -> &[u8] {
        match self.proto {
            Protocol::V1 => &self.key,
            Protocol::V2 => &self.key[32..],
        }
    }

    fn hmac_key(&self) -> &[u8] {
        match self.proto {
            Protocol::V1 => &self.key,
            Protocol::V2 => &self.key[..32],
        }
    }

    /// Zasifruje `data` (delka nasobek 16 B).
    pub fn encrypt(&self, rng: &mut impl CryptoRngCore, data: &[u8]) -> Option<Vec<u8>> {
        if data.len() % 16 != 0 {
            return None;
        }
        let mut iv = [0u8; 16];
        if self.proto == Protocol::V2 {
            rng.fill_bytes(&mut iv);
        }
        let mut buf = data.to_vec();
        let n = buf.len();
        cbc::Encryptor::<Aes256>::new_from_slices(self.aes_key(), &iv)
            .ok()?
            .encrypt_padded_mut::<NoPadding>(&mut buf, n)
            .ok()?;
        Some(match self.proto {
            Protocol::V1 => buf,
            Protocol::V2 => {
                let mut out = iv.to_vec();
                out.extend_from_slice(&buf);
                buf.zeroize();
                out
            }
        })
    }

    /// Rozsifruje; `None` = spatna delka.
    pub fn decrypt(&self, data: &[u8]) -> Option<Vec<u8>> {
        let (iv, ct) = match self.proto {
            Protocol::V1 => ([0u8; 16], data),
            Protocol::V2 => {
                if data.len() < 16 {
                    return None;
                }
                (data[..16].try_into().ok()?, &data[16..])
            }
        };
        if ct.len() % 16 != 0 {
            return None;
        }
        let mut buf = ct.to_vec();
        cbc::Decryptor::<Aes256>::new_from_slices(self.aes_key(), &iv)
            .ok()?
            .decrypt_padded_mut::<NoPadding>(&mut buf)
            .ok()?;
        Some(buf)
    }

    /// Autentizace zpravy sdilenym tajemstvim.
    pub fn authenticate(&self, msg: &[u8]) -> Vec<u8> {
        authenticate(self.proto, self.hmac_key(), msg)
    }

    /// Overi autentizaci v konstantnim case.
    pub fn verify(&self, msg: &[u8], tag: &[u8]) -> bool {
        verify(self.proto, self.hmac_key(), msg, tag)
    }
}

/// Autentizace klicem (sdilene tajemstvi nebo token PIN/UV).
pub fn authenticate(proto: Protocol, key: &[u8], msg: &[u8]) -> Vec<u8> {
    let mut mac = <HmacSha256 as Mac>::new_from_slice(key).expect("HMAC prijme klic libovolne delky");
    mac.update(msg);
    let full = mac.finalize().into_bytes();
    match proto {
        Protocol::V1 => full[..16].to_vec(),
        Protocol::V2 => full.to_vec(),
    }
}

/// Overi autentizaci v konstantnim case.
pub fn verify(proto: Protocol, key: &[u8], msg: &[u8], tag: &[u8]) -> bool {
    use subtle::ConstantTimeEq;
    let want = authenticate(proto, key, msg);
    want.len() == tag.len() && bool::from(want.ct_eq(tag))
}

/// Platforma: dohoda s klicem. Vrati klic platformy (COSE) pro klic
/// a sdilene tajemstvi. `None` = neplatny bod klice.
pub fn encapsulate(rng: &mut impl CryptoRngCore, proto: Protocol, peer: &Value) -> Option<(Value, Shared)> {
    let peer = cose::ecdh_point(peer)?;
    let secret = p256::ecdh::EphemeralSecret::random(rng);
    let (x, y) = cose::xy(&secret.public_key());
    let z = secret.diffie_hellman(&peer);
    let shared = Shared::derive(proto, z.raw_secret_bytes());
    Some((cose::ecdh_key(&x, &y), shared))
}

/// Klic: druha strana dohody (staticky klic dohody klice).
pub fn decapsulate(secret: &p256::SecretKey, proto: Protocol, peer: &Value) -> Option<Shared> {
    let peer = cose::ecdh_point(peer)?;
    let z = p256::ecdh::diffie_hellman(secret.to_nonzero_scalar(), peer.as_affine());
    Some(Shared::derive(proto, z.raw_secret_bytes()))
}

/// `LEFT(SHA-256(pin), 16)` - co klic s PINem porovnava.
pub fn pin_hash(pin: &[u8]) -> [u8; 16] {
    let mut h = [0u8; 16];
    h.copy_from_slice(&Sha256::digest(pin)[..16]);
    h
}

/// PIN doplneny nulami na 64 B (setPIN, changePIN). `None` = PIN delsi
/// nez 63 B nebo kratsi nez 4 znaky.
pub fn pad_pin(pin: &[u8]) -> Option<[u8; 64]> {
    let chars = core::str::from_utf8(pin).ok()?.chars().count();
    if chars < 4 || pin.len() > 63 {
        return None;
    }
    let mut p = [0u8; 64];
    p[..pin.len()].copy_from_slice(pin);
    Some(p)
}
