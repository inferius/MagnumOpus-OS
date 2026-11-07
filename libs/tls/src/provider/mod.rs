//! Poskytovatel kryptografie pro rustls nad knihovnami RustCrypto (cisty
//! Rust, bez C a bez SIMD - prelozi se pro cile bez OS na x86_64 i aarch64).
//!
//! Jen TLS 1.3:
//! - sady: `TLS13_AES_128_GCM_SHA256`, `TLS13_AES_256_GCM_SHA384`,
//!   `TLS13_CHACHA20_POLY1305_SHA256`,
//! - vymena klicu: X25519, secp256r1, secp384r1,
//! - overeni podpisu: ECDSA P-256/P-384, Ed25519, RSA PKCS#1 v1.5 a PSS
//!   (SHA-256/384/512).
//!
//! Klient nepodepisuje (bez klientskeho certifikatu), takze `KeyProvider`
//! soukrome klice nenacita. RSA se pouziva jen k overeni podpisu (verejny
//! klic); casovy utok na desifrovani RSA (RUSTSEC-2023-0071) se ho netyka.

mod aead;
mod hash;
mod kx;
mod verify;

use alloc::sync::Arc;

use rustls::crypto::{CipherSuiteCommon, CryptoProvider, GetRandomFailed, KeyProvider, SecureRandom};
use rustls::{CipherSuite, SupportedCipherSuite, Tls13CipherSuite};

pub use verify::ALGORITHMS;

/// Co knihovna potrebuje od systemu: kryptograficky bezpecnou nahodu
/// a realny cas (platnost certifikatu).
pub struct Platform {
    /// Naplni buffer nahodnymi bajty; `false` = nahoda neni k dispozici
    /// (napr. nenasazeny generator jadra). Pak se spojeni nenavaze.
    pub random: fn(&mut [u8]) -> bool,
    /// Realny cas v sekundach od 1970 UTC; `None` = neznamy (certifikat pak
    /// nejde overit a spojeni se nenavaze).
    pub now: fn() -> Option<u64>,
}

impl core::fmt::Debug for Platform {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("Platform")
    }
}

impl SecureRandom for Platform {
    fn fill(&self, buf: &mut [u8]) -> Result<(), GetRandomFailed> {
        if (self.random)(buf) { Ok(()) } else { Err(GetRandomFailed) }
    }
}

impl KeyProvider for Platform {
    fn load_private_key(
        &self,
        _key: pki_types::PrivateKeyDer<'static>,
    ) -> Result<Arc<dyn rustls::sign::SigningKey>, rustls::Error> {
        Err(rustls::Error::General(alloc::string::String::from(
            "klientske certifikaty nejsou podporovane",
        )))
    }
}

impl rustls::time_provider::TimeProvider for Platform {
    fn current_time(&self) -> Option<pki_types::UnixTime> {
        (self.now)().map(|s| pki_types::UnixTime::since_unix_epoch(core::time::Duration::from_secs(s)))
    }
}

/// Generator pro RustCrypto (rand_core) nad nahodou platformy.
pub(crate) struct Rng(pub &'static Platform);

impl rand_core::RngCore for Rng {
    fn next_u32(&mut self) -> u32 {
        let mut b = [0u8; 4];
        self.fill_bytes(&mut b);
        u32::from_le_bytes(b)
    }

    fn next_u64(&mut self) -> u64 {
        let mut b = [0u8; 8];
        self.fill_bytes(&mut b);
        u64::from_le_bytes(b)
    }

    fn fill_bytes(&mut self, dest: &mut [u8]) {
        // Bez nahody nesmi vzniknout klic: radeji konec procesu nez slaby
        // klic (try_fill_bytes vola rustls jen pres SecureRandom).
        assert!((self.0.random)(dest), "tls: nahoda neni k dispozici");
    }

    fn try_fill_bytes(&mut self, dest: &mut [u8]) -> Result<(), rand_core::Error> {
        if (self.0.random)(dest) {
            Ok(())
        } else {
            Err(rand_core::Error::from(core::num::NonZeroU32::new(rand_core::Error::CUSTOM_START).unwrap()))
        }
    }
}

impl rand_core::CryptoRng for Rng {}

pub const TLS13_AES_128_GCM_SHA256: SupportedCipherSuite = SupportedCipherSuite::Tls13(&Tls13CipherSuite {
    common: CipherSuiteCommon {
        suite: CipherSuite::TLS13_AES_128_GCM_SHA256,
        hash_provider: hash::SHA256,
        // RFC 8446 5.5: AES-GCM nejvys 2^24,5 zaznamu jednim klicem.
        confidentiality_limit: 1 << 23,
    },
    hkdf_provider: &rustls::crypto::tls13::HkdfUsingHmac(hash::HMAC_SHA256),
    aead_alg: &aead::AES_128_GCM,
    // QUIC zatim neni.
    quic: None,
});

pub const TLS13_AES_256_GCM_SHA384: SupportedCipherSuite = SupportedCipherSuite::Tls13(&Tls13CipherSuite {
    common: CipherSuiteCommon {
        suite: CipherSuite::TLS13_AES_256_GCM_SHA384,
        hash_provider: hash::SHA384,
        confidentiality_limit: 1 << 23,
    },
    hkdf_provider: &rustls::crypto::tls13::HkdfUsingHmac(hash::HMAC_SHA384),
    aead_alg: &aead::AES_256_GCM,
    // QUIC zatim neni.
    quic: None,
});

pub const TLS13_CHACHA20_POLY1305_SHA256: SupportedCipherSuite = SupportedCipherSuite::Tls13(&Tls13CipherSuite {
    common: CipherSuiteCommon {
        suite: CipherSuite::TLS13_CHACHA20_POLY1305_SHA256,
        hash_provider: hash::SHA256,
        confidentiality_limit: u64::MAX,
    },
    hkdf_provider: &rustls::crypto::tls13::HkdfUsingHmac(hash::HMAC_SHA256),
    aead_alg: &aead::CHACHA20_POLY1305,
    // QUIC zatim neni.
    quic: None,
});

/// Poskytovatel kryptografie s nahodou a casem `platform`.
pub fn provider(platform: &'static Platform) -> CryptoProvider {
    CryptoProvider {
        cipher_suites: alloc::vec![
            TLS13_AES_128_GCM_SHA256,
            TLS13_AES_256_GCM_SHA384,
            TLS13_CHACHA20_POLY1305_SHA256,
        ],
        kx_groups: kx::all(platform),
        signature_verification_algorithms: verify::ALGORITHMS,
        secure_random: platform,
        key_provider: platform,
    }
}
