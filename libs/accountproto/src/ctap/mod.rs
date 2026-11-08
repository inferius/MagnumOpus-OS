//! CTAP2 (FIDO2) pro bezpecnostni klice: CBOR, prenos CTAPHID, klice
//! COSE, protokoly PIN/UV, strana platformy a simulovany klic.
//!
//! Kryptografie je z RustCrypto (p256, ed25519-dalek, sha2, hmac, hkdf,
//! aes, cbc); vlastni je jen kodovani zprav. Popis: `docs/cs/ucty.md`,
//! kapitola "Bezpecnostni klic".

pub mod cbor;
pub mod client;
pub mod cose;
pub mod hid;
pub mod hidclient;
pub mod msg;
pub mod pin;
#[cfg(any(test, feature = "soft"))]
pub mod soft;

pub use client::{Assertion, Client, Credential, CtapError, CtapResult, RP_ID, Token, Transport};
pub use cose::PublicKey;
