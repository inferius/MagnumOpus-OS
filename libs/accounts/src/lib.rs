//! Logika sluzby uctu `accountd` bez zavislosti na jadru (testuje se na
//! hostiteli): zaznamy uctu, Argon2id, PIN vazany na klic zarizeni,
//! zpomaleni po chybach, bezpecnostni klice CTAP2, obaly klice domova
//! a obnovovaci klic.
//!
//! Popis: `docs/cs/ucty.md`. Kryptografie je z RustCrypto (argon2,
//! chacha20poly1305, hkdf, hmac, sha2, blake2); vlastni kryptografie zde
//! neni.

#![cfg_attr(not(test), no_std)]

extern crate alloc;

pub mod auth;
pub mod device;
pub mod engine;
pub mod kdf;
pub mod manage;
pub mod record;
pub mod throttle;
pub mod wrap;

#[cfg(test)]
mod tests;

pub use auth::Outcome;
pub use device::{DeviceKey, FileKey};
pub use engine::{Config, DEV_NAME, Engine, MemStore, Store};
pub use kdf::KdfParams;
pub use record::{Account, Meta};
