//! PS/2 (radic i8042): konstanty radice, dekodovani scan kodu klavesnice
//! (sada 1, s prekladem radice) a paketu mysi (standardni, IntelliMouse
//! s koleckem a s 5 tlacitky) na udalosti `inputproto` (evdev).
//!
//! Knihovna nesaha na hardware (to dela ovladac `user/ps2`), takze se
//! testuje na hostiteli (`cargo test -p i8042`).
//!
//! Dokumentace: `docs/cs/vstup.md` (kapitola PS/2), `docs/en/input.md`.

#![cfg_attr(not(test), no_std)]

extern crate alloc;

pub mod controller;
pub mod mouse;
pub mod scancode;

#[cfg(test)]
mod tests;
