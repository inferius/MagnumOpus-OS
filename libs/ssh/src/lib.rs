//! SSH pro MagnumOpus (`docs/cs/ssh.md`).
//!
//! - [`keys`]: verejne klice uzivatelu (rozbor, otisk, overeni podpisu).
//!   Pouziva ji sluzba uctu; nezavisi na knihovne protokolu.
//! - Protokol (feature `proto`): server a klient nad sunset.
#![cfg_attr(not(test), no_std)]

extern crate alloc;

pub mod keys;
pub mod sftp;

#[cfg(feature = "proto")]
pub mod server;
#[cfg(feature = "proto")]
pub use sunset;

#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_sftp;
#[cfg(all(test, feature = "proto"))]
mod tests_server;
