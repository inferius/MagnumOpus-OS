//! Mereni skladani snimku po castech (kde je cas: plocha, stiny, okna,
//! rozmazani listy, vrstva nad okny).
//!
//! Hodiny dodava sluzba (`Comp::set_prof_clock`); bez nich se nemeri nic
//! a mereni nic nestoji. `compd` vypise rozpad u snimku cele obrazovky
//! a prumer po davce snimku (`docs/cs/gui.md`, kapitola o vykonu).

use alloc::format;
use alloc::string::String;

/// Cast snimku.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Part {
    /// Obraz plochy pod okny.
    Desktop = 0,
    /// Stiny oken.
    Shadow,
    /// Okna (telo, titulek, obsah aplikace, ram, rohy).
    Window,
    /// Rozmazane pozadi listy (prepocet mezipameti).
    BarBlur,
    /// Vrstva nad okny (lista, popovery, paleta, dialog, kurzor).
    Top,
    /// Z vrstvy nad okny: lista.
    Bar,
    /// Z vrstvy nad okny: ztlumeni a systemovy dialog.
    Dialog,
}

pub const PARTS: usize = 7;
const NAMES: [&str; PARTS] = ["plocha", "stiny", "okna", "rozmazani listy", "nad okny", "z toho lista", "z toho dialog"];

/// Soucty casu po castech (ns) od posledniho `take`.
#[derive(Debug, Clone, Default)]
pub struct Prof {
    clock: Option<fn() -> u64>,
    pub parts: [u64; PARTS],
    /// Vypnout vynechani zakrytych oblasti (srovnani a mereni).
    pub no_cull: bool,
}

impl Prof {
    pub fn set_clock(&mut self, clock: Option<fn() -> u64>) {
        self.clock = clock;
    }

    /// Cas zacatku mereni (0 bez hodin).
    #[inline]
    pub fn start(&self) -> u64 {
        self.clock.map_or(0, |f| f())
    }

    /// Pricte cas od `t0` k casti `p`.
    #[inline]
    pub fn add(&mut self, p: Part, t0: u64) {
        if let Some(f) = self.clock {
            self.parts[p as usize] += f().saturating_sub(t0);
        }
    }

    /// Rozpad od `before` (stav `parts` pred snimkem) jako text a odecteni
    /// z prubezneho souctu (snimek cele obrazovky se vypise zvlast a do
    /// prumeru ostatnich se nepocita).
    pub fn split_off(&mut self, before: &[u64; PARTS]) -> String {
        let mut d = [0u64; PARTS];
        for i in 0..PARTS {
            d[i] = self.parts[i].saturating_sub(before[i]);
        }
        self.parts = *before;
        describe(&d, 1)
    }

    /// Prumer po `frames` snimcich jako text a vynulovani.
    pub fn take(&mut self, frames: u64) -> String {
        let s = describe(&self.parts, frames);
        self.parts = [0; PARTS];
        s
    }
}

/// `plocha 12 us, stiny ...` (prumer na snimek).
pub fn describe(parts: &[u64; PARTS], frames: u64) -> String {
    let n = frames.max(1);
    let mut s = String::new();
    for (i, name) in NAMES.iter().enumerate() {
        if i > 0 {
            s.push_str(", ");
        }
        s.push_str(&format!("{name} {} us", parts[i] / n / 1000));
    }
    s
}
