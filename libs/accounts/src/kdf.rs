//! Argon2id (RFC 9106, verze 0x13) pro hesla a PINy.
//!
//! Jeden beh dava 64 B: prvnich 32 B je overovaci hodnota (v DB), druhych
//! 32 B je material obalu klice domova. Z overovaci hodnoty material
//! odvodit nejde (obe poloviny jsou vystupy jedne hashovaci funkce).

use argon2::{Algorithm, Argon2, Params, Version};
use subtle::ConstantTimeEq;
use wire::{Reader, Writer};
use zeroize::Zeroize;

use abi::{Error, Result};

/// Parametry Argon2id.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KdfParams {
    /// Pamet v KiB.
    pub m_kib: u32,
    /// Pocet pruchodu.
    pub t: u32,
    /// Paralelismus (pocet drah; pocita se v jednom vlakne).
    pub p: u32,
}

impl KdfParams {
    /// `cargo xtask run` a testy v QEMU: 8 MiB, t=2 (nativne ~5 ms).
    pub const QEMU: KdfParams = KdfParams { m_kib: 8 * 1024, t: 2, p: 1 };
    /// Skutecne PC: 64 MiB, t=3 (nativne ~70 ms na PC z roku 2024).
    pub const HW: KdfParams = KdfParams { m_kib: 64 * 1024, t: 3, p: 1 };
    /// Testy na hostiteli (rychle).
    pub const TEST: KdfParams = KdfParams { m_kib: 64, t: 1, p: 1 };

    /// Jsou tyto parametry slabsi nez `other` (pak se hodnota po
    /// uspesnem prihlaseni prepocita)?
    pub fn weaker_than(&self, other: &KdfParams) -> bool {
        self.m_kib < other.m_kib || (self.m_kib == other.m_kib && self.t < other.t)
    }

    pub fn put(&self, w: &mut Writer) {
        w.u32(self.m_kib).u32(self.t).u32(self.p);
    }

    pub fn get(r: &mut Reader) -> Result<Self> {
        let p = KdfParams { m_kib: r.u32()?, t: r.u32()?, p: r.u32()? };
        // Rozumne meze (ochrana proti poskozenemu zaznamu, ktery by si
        // rekl o gigabajty pameti).
        if p.m_kib < 8 || p.m_kib > 1024 * 1024 || p.t == 0 || p.t > 64 || p.p == 0 || p.p > 16 {
            return Err(Error::Corrupted);
        }
        Ok(p)
    }
}

/// Vystup Argon2id (vynuluje se pri zahozeni).
pub struct Derived {
    out: [u8; 64],
}

impl Derived {
    pub fn verifier(&self) -> &[u8; 32] {
        self.out[..32].try_into().expect("32 B")
    }

    pub fn wrap_material(&self) -> &[u8; 32] {
        self.out[32..].try_into().expect("32 B")
    }
}

impl Drop for Derived {
    fn drop(&mut self) {
        self.out.zeroize();
    }
}

/// Argon2id z `secret` a `salt`.
pub fn derive(params: &KdfParams, secret: &[u8], salt: &[u8]) -> Result<Derived> {
    let p = Params::new(params.m_kib, params.t, params.p, Some(64)).map_err(|_| Error::InvalidArgs)?;
    let a = Argon2::new(Algorithm::Argon2id, Version::V0x13, p);
    let mut out = [0u8; 64];
    a.hash_password_into(secret, salt, &mut out).map_err(|_| Error::NoMemory)?;
    Ok(Derived { out })
}

/// Ulozena overovaci hodnota (parametry, sul, hodnota).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Verifier {
    pub params: KdfParams,
    pub salt: [u8; 16],
    pub value: [u8; 32],
}

impl Verifier {
    pub fn put(&self, w: &mut Writer) {
        self.params.put(w);
        w.bytes(&self.salt).bytes(&self.value);
    }

    pub fn get(r: &mut Reader) -> Result<Self> {
        let params = KdfParams::get(r)?;
        let salt = r.take(16)?.try_into().map_err(|_| Error::Corrupted)?;
        let value = r.take(32)?.try_into().map_err(|_| Error::Corrupted)?;
        Ok(Verifier { params, salt, value })
    }
}

/// Porovnani v konstantnim case.
pub fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && bool::from(a.ct_eq(b))
}
