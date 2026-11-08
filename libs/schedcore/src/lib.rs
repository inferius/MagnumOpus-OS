//! Logika planovace jadra, oddelena od jadra kvuli testum na hostiteli.
//!
//! - [`class`]: tridy QoS, vahy a kvanta.
//! - [`fair`]: EEVDF uvnitr tridy ([`fair::FairQueue`]) a vazeny virtualni
//!   cas trid ([`fair::ClassClock`]).
//! - [`rt`]: Realtime rozpocet (server s konstantni sirkou pasma, CBS)
//!   a prijeti noveho Realtime vlakna (admission).
//! - [`runqueue`]: fronta jednoho CPU, ktera vse spojuje (pouziva ji jadro
//!   i simulace v testech).
//! - [`topo`]: typy jader (CPUID 0x1A, MIDR), kapacity, PPTT.
//! - [`place`]: vyber CPU podle tridy a kapacity, kradeni prace.
//!
//! Vse je bez zamku a bez casu systemu: cas (ns) predava volajici. Navrh
//! a srovnani s jinymi systemy: `docs/cs/planovac.md`.

#![cfg_attr(not(test), no_std)]

extern crate alloc;

pub mod class;
pub mod fair;
pub mod place;
pub mod rt;
pub mod runqueue;
pub mod topo;

pub use class::Class;
pub use runqueue::{Ent, RunQueue};

#[cfg(test)]
mod class_tests;
#[cfg(test)]
mod fair_tests;
#[cfg(test)]
mod place_tests;
#[cfg(test)]
mod rt_tests;
#[cfg(test)]
mod runqueue_tests;
#[cfg(test)]
mod sim;
#[cfg(test)]
mod sim_tests;
#[cfg(test)]
mod topo_tests;
