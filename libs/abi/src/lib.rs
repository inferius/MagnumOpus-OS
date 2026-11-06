//! Rozhrani mezi jadrem a userspace.
//!
//! Vse, co musi jadro a programy v userspace chapat stejne: kody chyb,
//! cisla syscallu, prava handlu, signaly objektu, priznaky mapovani pameti
//! a format zpravy pri startu procesu. Zmena hodnoty je nekompatibilni
//! zmena rozhrani.
//!
//! Stabilni rozhrani pro aplikace je knihovna `libos`, ne primo cisla
//! syscallu (ta se smi menit spolu s libos). Kody chyb jsou ale stabilni
//! i pro aplikace - objevuji se v logach a v dokumentaci.
//!
//! Dokumentace: `docs/cs/` a `docs/en/` (chyby: `chyby.md` / `errors.md`).

#![cfg_attr(not(test), no_std)]

pub mod bootstrap;
pub mod error;
pub mod object;
pub mod process;
pub mod syscall;
pub mod vm;

pub use error::{Error, Result};
pub use object::{Handle, ObjectType, Rights, Signals};

#[cfg(test)]
mod tests;
