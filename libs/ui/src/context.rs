//! Kontext okna pro adaptivni UI: velikost okna, trida sirky, obrazovka,
//! pracovni plocha a skala. Plati okno po okne (ne globalne): aplikace ho
//! dostane ve `view` a podle tridy sirky sveho okna voli rozvrzeni.
//!
//! Tridy a prahy jsou z tokenu `breakpoint.*` (`design/tokens.toml`,
//! `design/mobil.html`): compact < 600, medium 600-839, expanded >= 840.
//! Hystereze: do vetsi tridy hned od prahu, zpet az pod prahem minus
//! `breakpoint.hysteresis` (okno na hrane neskace mezi rozvrzenimi).

use guiproto::Rect;
use theme::Breakpoints;

/// Trida sirky okna.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum SizeClass {
    Compact,
    Medium,
    Expanded,
}

impl SizeClass {
    fn threshold(self, bp: &Breakpoints) -> u32 {
        match self {
            SizeClass::Compact => 0,
            SizeClass::Medium => bp.medium,
            SizeClass::Expanded => bp.expanded,
        }
    }

    fn down(self) -> Self {
        match self {
            SizeClass::Expanded => SizeClass::Medium,
            _ => SizeClass::Compact,
        }
    }

    /// Trida pro sirku `w` s hysterezi proti predchozi tride `prev`.
    pub fn of(w: u32, prev: Option<SizeClass>, bp: &Breakpoints) -> SizeClass {
        let raw = if w >= bp.expanded {
            SizeClass::Expanded
        } else if w >= bp.medium {
            SizeClass::Medium
        } else {
            SizeClass::Compact
        };
        let Some(mut c) = prev else { return raw };
        if raw >= c {
            return raw;
        }
        // Dolu jen pod prah minus hystereze (i o vic trid najednou).
        while c > raw && w + bp.hysteresis < c.threshold(bp) {
            c = c.down();
        }
        c
    }
}

/// Kontext jednoho okna. Neuplny vycet poli (`non_exhaustive`): pribudou
/// dalsi (drzeni zarizeni, vzdalenost pohledu, bezpecny okraj, ukazatele,
/// klavesnice) bez zmeny API aplikaci.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Context {
    /// Velikost obsahu okna (logicke px).
    pub w: u32,
    pub h: u32,
    pub class: SizeClass,
    /// Vystup, na kterem okno lezi: cela obrazovka a pracovni plocha (bez
    /// listy) v logickych px.
    pub output: u32,
    pub screen: Rect,
    pub work_area: Rect,
    /// Skala vystupu v procentech.
    pub scale: u32,
    pub refresh_mhz: u32,
    /// Roste s kazdou zmenou kontextu od kompozitoru.
    pub generation: u32,
    /// Formaty, ktere je prave ve schrance (polozka "Vlozit" aktivni).
    pub clipboard: alloc::vec::Vec<alloc::string::String>,
}

impl Context {
    pub(crate) fn new(w: u32, h: u32, bp: &Breakpoints) -> Self {
        Self {
            w,
            h,
            class: SizeClass::of(w, None, bp),
            output: 0,
            screen: Rect::new(0, 0, w, h),
            work_area: Rect::new(0, 0, w, h),
            scale: 100,
            refresh_mhz: 0,
            generation: 0,
            clipboard: alloc::vec::Vec::new(),
        }
    }

    /// Je ve schrance format `f`?
    pub fn clipboard_has(&self, f: &str) -> bool {
        self.clipboard.iter().any(|x| x == f)
    }

    /// Nova velikost okna; vraci, zda se zmenila trida sirky.
    pub(crate) fn resize(&mut self, w: u32, h: u32, bp: &Breakpoints) -> bool {
        self.w = w;
        self.h = h;
        let c = SizeClass::of(w, Some(self.class), bp);
        let changed = c != self.class;
        self.class = c;
        changed
    }
}
