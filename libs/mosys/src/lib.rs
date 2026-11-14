//! mosys - objekty jadra pro programy v C (`include/magnumopus/sys.h`):
//! kanaly, VMO, ploty, udalosti a cekani na vic objektu. Pouziva ho proces
//! dekoderu medii `ffdec` (docs/cs/multimedia.md).
//!
//! Soucast `libc.a` i `libc.so` (jako `mosdl`): jedna kopie std v procesu.
//! Program, ktery nic z toho nevola, nic nezaplati (`--gc-sections`).
//! Symboly jsou tenke obalky syscallu z `libos::sys`; chyby jsou zaporne
//! kody jadra (`abi::Error`).

#![cfg_attr(target_os = "magnumopus", feature(rustc_private))]

/// Konstanty hlavicky (overi je host test proti `include/magnumopus/sys.h`).
pub mod consts {
    pub const SIG_SIGNALED: u32 = 0x1;
    pub const SIG_TERMINATED: u32 = 0x2;
    pub const SIG_READABLE: u32 = 0x4;
    pub const SIG_PEER_CLOSED: u32 = 0x8;
    pub const DEADLINE_INFINITE: i64 = -1;
    pub const DEADLINE_NOW: i64 = 0;
}

/// Termin z C (`-1` = navzdy, jinak absolutni ns) na termin syscallu.
pub fn deadline(ns: i64) -> u64 {
    if ns < 0 { u64::MAX } else { ns as u64 }
}

#[cfg(target_os = "magnumopus")]
mod os;

#[cfg(test)]
mod tests;
