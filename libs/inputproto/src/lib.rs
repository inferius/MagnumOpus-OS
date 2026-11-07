//! Protokol vstupu (verze 1).
//!
//! Dve vrstvy, obe jako zpravy kanalu:
//!
//! - [`device`]: ovladac vstupniho zarizeni (nebo zdroj umeleho vstupu)
//!   -> sluzba `inputd`. Surove udalosti ve tvaru evdev (typ, kod,
//!   hodnota), seskupene do snimku zakoncenych `SYN_REPORT`.
//! - [`event`]: `inputd` -> vlastnik fokusu (konzole, terminal, pozdeji
//!   kompozitor). Hotove udalosti: fyzicka klavesa, vlozeny text podle
//!   rozlozeni, pohyb a tlacitka mysi, kolecko, fokus, bezpecna zkratka.
//!
//! Kody klaves a tlacitek ([`code`]) jsou cisla z Linuxu (evdev,
//! `input-event-codes.h`): stejna pouziva virtio-input, takze je ovladac
//! jen preda dal, a kod nezavisi na rozlozeni (hry, zkratky).
//!
//! Knihovna je `no_std` bez zavislosti na jadre, testuje se na hostiteli
//! (`cargo test -p inputproto`).
//!
//! Dokumentace: `docs/cs/vstup.md`, `docs/en/input.md`.

#![cfg_attr(not(test), no_std)]

extern crate alloc;

pub mod code;
pub mod control;
pub mod device;
pub mod event;

pub use event::{Action, Button, Event, Record};

/// Bity modifikatoru v [`Record::mods`] (stav po zpracovani udalosti).
pub mod mods {
    /// Levy nebo pravy Shift.
    pub const SHIFT: u16 = 1 << 0;
    /// Levy nebo pravy Ctrl.
    pub const CTRL: u16 = 1 << 1;
    /// Levy Alt (a pravy Alt v rozlozeni bez AltGr, napr. US).
    pub const ALT: u16 = 1 << 2;
    /// Pravy Alt v rozlozeni s treti urovni (CZ): pise `@`, `€`, `{` ...
    pub const ALTGR: u16 = 1 << 3;
    /// Klavesa Windows / Super / Cmd.
    pub const META: u16 = 1 << 4;
    /// Caps Lock zapnuty (prepinac, ne drzena klavesa).
    pub const CAPS_LOCK: u16 = 1 << 5;
    /// Num Lock zapnuty.
    pub const NUM_LOCK: u16 = 1 << 6;
}

#[cfg(test)]
mod tests;
