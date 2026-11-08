//! Vrstva agentu AI: spolecne typy pro `agentd`, klienty a sluzby.
//!
//! - [`provider`]: rozhrani modelu nezavisle na poskytovateli; [`msg`]
//!   zpravy, [`stream`] udalosti proudu, [`price`] ceny a limity utraty.
//! - [`claude`]: prvni poskytovatel (Messages API, kodek a doprava jako
//!   trait), [`fake`]: deterministicky falesny model pro testy.
//! - [`tool`]: strojove citelny popis nastroju a overeni vstupu;
//!   [`registry`]: protokol sluzeb, ktere nastroje registruji.
//! - [`policy`]: pravidla (volne / potvrdit / odmitnout, pracovni prostor,
//!   cesty, ktere neopusti pocitac); [`action`]: akce k potvrzeni
//!   a jejich provedeni u klienta; [`audit`]: zaznamy auditu.
//! - [`proto`]: protokol brana -> agentd a klient <-> agentd.
//! - [`secret`]: API klic (nikdy do logu) a jeho sifrovani vazane na
//!   uzivatele.
//!
//! Navrh a bezpecnost: `docs/cs/agent.md`.

#![cfg_attr(not(test), no_std)]

extern crate alloc;

pub mod action;
pub mod audit;
pub mod claude;
pub mod fake;
pub mod json;
pub mod msg;
pub mod policy;
pub mod price;
pub mod proto;
pub mod provider;
pub mod registry;
pub mod secret;
pub mod stream;
pub mod tool;

#[cfg(test)]
mod tests;

pub use abi::Error;
pub type Result<T> = core::result::Result<T, Error>;
