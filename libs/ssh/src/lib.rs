//! SSH pro MagnumOpus (`docs/cs/ssh.md`).
//!
//! - [`keys`]: verejne klice uzivatelu (rozbor, otisk, overeni podpisu).
//!   Pouziva ji sluzba uctu; nezavisi na knihovne protokolu.
//! - Protokol (feature `proto`): server a klient nad sunset.
#![no_std]

extern crate alloc;

pub mod keys;

#[cfg(feature = "proto")]
pub use sunset;

#[cfg(test)]
mod tests;
