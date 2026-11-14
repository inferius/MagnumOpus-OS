//! Obnova sluzeb po padu: protokol listku a kostra klienta.
//!
//! Kdyz spadne systemova sluzba, supervisor initu ji spusti znovu a klient
//! se pripoji sam (viz `docs/cs/obnova-sluzeb.md`):
//!
//! - **listek** (lease) je kanal klienta k supervisoru svazany s jednim
//!   receptem relace. Pres nej klient dostane novou relaci se stejnou
//!   identitou ([`proto`]),
//! - **pravidla** pro rozpracovane pozadavky a cekani ([`policy`]),
//! - **[`Conn`]** (feature `client`): kanal relace + listek; volani pres
//!   nej se po padu sluzby samo obnovi, nebo vrati jasnou chybu.
//!
//! Protokol a pravidla jsou bez zavislosti na jadru (testy na hostiteli).

#![cfg_attr(not(test), no_std)]

extern crate alloc;

#[cfg(test)]
mod tests;

pub mod policy;
/// Protokol listku (crate `leaseproto`).
pub use leaseproto as proto;

#[cfg(feature = "client")]
mod conn;

#[cfg(feature = "client")]
pub use conn::{Conn, ConnState, Hook};
pub use policy::{DEFAULT_WAIT, Event, Kind};

pub use abi::{Error, Result};
