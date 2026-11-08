//! Pismo TrueType: cteni (`cmap`, `glyf`, `hmtx`, `kern`), parovy kerning
//! z `GPOS`, jednoduche zameny z `GSUB` (vlastnosti jako `tnum`) a rasterizer
//! s plosnym vyhlazenim a jemnym umistenim (zlomky pixelu), bez hintingu
//! a bez shapingu.
//!
//! Soubor pisma je neduveryhodny vstup: kazde cteni kontroluje meze,
//! vadny glyf je prazdny glyf, nikdy panika (test `garbage_never_panics`).
//!
//! Pisma systemu (`fonts/`, licence v `fonts/README.md`):
//! - Inter (SIL OFL 1.1) v rezech 300-700: pismo UI ([`inter`], cargo
//!   vlastnosti `inter-*`, kazdy program si vlozi jen rezy, ktere kresli),
//! - JetBrains Mono (SIL OFL 1.1) 400 a 600: terminal a kod ([`mono`]),
//! - Tuffy (public domain): zaloha, vzdy ([`TUFFY`]).
//!
//! ```ignore
//! let font = font::Font::parse(font::TUFFY).unwrap();
//! let g = font.rasterize(font.glyph_index('č'), 16.0);
//! canvas.mask(x + g.left, baseline + g.top, g.w, g.h, &g.coverage, color);
//! ```

#![cfg_attr(not(test), no_std)]

extern crate alloc;

mod layout;
mod math;
mod outline;
mod raster;
mod tables;

#[cfg(test)]
mod tests;

use alloc::vec::Vec;

pub use raster::Rasterizer;

/// Zalozni pismo systemu (Tuffy, public domain).
pub static TUFFY: &[u8] = include_bytes!("../fonts/Tuffy.ttf");

/// Vlozene rezy Interu: (vaha, data). Jen rezy zapnute cargo vlastnosti.
pub fn inter() -> &'static [(u16, &'static [u8])] {
    &[
        #[cfg(feature = "inter-300")]
        (300, include_bytes!("../fonts/Inter-Light.ttf")),
        #[cfg(feature = "inter-400")]
        (400, include_bytes!("../fonts/Inter-Regular.ttf")),
        #[cfg(feature = "inter-500")]
        (500, include_bytes!("../fonts/Inter-Medium.ttf")),
        #[cfg(feature = "inter-600")]
        (600, include_bytes!("../fonts/Inter-SemiBold.ttf")),
        #[cfg(feature = "inter-700")]
        (700, include_bytes!("../fonts/Inter-Bold.ttf")),
    ]
}

/// Vlozene rezy JetBrains Mono: (vaha, data).
pub fn mono() -> &'static [(u16, &'static [u8])] {
    &[
        #[cfg(feature = "mono-400")]
        (400, include_bytes!("../fonts/JetBrainsMono-Regular.ttf")),
        #[cfg(feature = "mono-600")]
        (600, include_bytes!("../fonts/JetBrainsMono-SemiBold.ttf")),
    ]
}

/// Rez nejblize vaze `weight` podle pravidel CSS: pro vahu nad 500 nejdriv
/// tezsi rezy, pod 400 nejdriv lehci (`650` -> 700, `300` -> 300 nebo 400).
pub fn pick_weight(available: &[u16], weight: u16) -> Option<usize> {
    // Mensi klic = lepsi (poradi z CSS Fonts, vyber vahy).
    let key = |w: u16| -> (u32, u32) {
        let d = w.abs_diff(weight) as u32;
        if weight > 500 {
            (if w >= weight { 0 } else { 1 }, d)
        } else if weight < 400 {
            (if w <= weight { 0 } else { 1 }, d)
        } else if w >= weight && w <= 500 {
            (0, d)
        } else if w < weight {
            (1, d)
        } else {
            (2, d)
        }
    };
    (0..available.len()).min_by_key(|&i| key(available[i]))
}

/// Chyba pri cteni pisma.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FontError {
    /// Neni to TrueType (spatna hlavicka, chybi povinna tabulka).
    NotTrueType,
    /// Tabulka mimo soubor nebo s nesmyslnymi hodnotami.
    Corrupted,
    /// Format, ktery knihovna neumi (napr. CFF, tabulka cmap bez Unicode).
    Unsupported,
}

/// Bod obrysu v jednotkach pisma (y nahoru).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Point {
    pub x: f32,
    pub y: f32,
}

/// Usek obrysu: usecka nebo kvadraticka krivka.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Segment {
    Line(Point, Point),
    Quad(Point, Point, Point),
}

/// Vykresleny glyf: maska pokryti 0..=255 a jeji umisteni vuci pocatku
/// (pero na zakladne): levy horni roh masky je `(pen_x + left,
/// baseline + top)`, `top` je zaporny nad zakladnou.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Glyph {
    pub w: u32,
    pub h: u32,
    pub left: i32,
    pub top: i32,
    /// Posun pera v pixelech (bez kerningu).
    pub advance: f32,
    pub coverage: Vec<u8>,
}

/// Svisle metriky v pixelech pro danou velikost.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LineMetrics {
    /// Nad zakladnou (kladne).
    pub ascent: f32,
    /// Pod zakladnou (kladne).
    pub descent: f32,
    pub line_gap: f32,
}

impl LineMetrics {
    /// Vyska radku zaokrouhlena nahoru.
    pub fn height(&self) -> u32 {
        math::ceil(self.ascent + self.descent + self.line_gap) as u32
    }
}

/// Precteny font (odkazuje do dat souboru).
pub struct Font<'a> {
    t: tables::Tables<'a>,
    /// Lookupy kerningu z `GPOS` (kdyz chybi tabulka `kern`).
    kern: layout::Layout<'a>,
    /// Zapnute jednoduche zameny z `GSUB`.
    subst: layout::Layout<'a>,
}

/// Nejvetsi velikost pisma v pixelech (ochrana pameti rasterizeru).
pub const MAX_PX: f32 = 512.0;

impl<'a> Font<'a> {
    pub fn parse(data: &'a [u8]) -> Result<Self, FontError> {
        let t = tables::Tables::parse(data)?;
        let kern = if t.has_kern_table() { layout::Layout::default() } else { layout::Layout::new(t.gpos, &[*b"kern"]) };
        Ok(Self { t, kern, subst: layout::Layout::default() })
    }

    /// Zapne vlastnosti `GSUB` s jednoduchymi zamenami (napr. `tnum`
    /// tabulkove cislice, `cv11`). Jine typy zamen (ligatury) se nepouziji.
    pub fn with_features(mut self, tags: &[[u8; 4]]) -> Self {
        self.subst = layout::Layout::new(self.t.gsub, tags);
        self
    }

    pub fn units_per_em(&self) -> u16 {
        self.t.units_per_em
    }

    pub fn glyph_count(&self) -> u16 {
        self.t.num_glyphs
    }

    /// Index glyfu pro znak (0 = chybejici glyf "tofu").
    pub fn glyph_index(&self, ch: char) -> u16 {
        let g = self.t.cmap_lookup(ch as u32);
        if g >= self.t.num_glyphs {
            return 0;
        }
        if self.subst.is_empty() || g == 0 {
            return g;
        }
        let s = self.subst.single(g);
        if s < self.t.num_glyphs { s } else { g }
    }

    /// Ma pismo glyf pro znak?
    pub fn has_glyph(&self, ch: char) -> bool {
        self.glyph_index(ch) != 0
    }

    /// Posun pera v jednotkach pisma.
    pub fn advance_units(&self, gid: u16) -> u16 {
        self.t.advance(gid)
    }

    /// Kerning dvojice v jednotkach pisma (0 = zadny).
    pub fn kern_units(&self, left: u16, right: u16) -> i16 {
        if self.kern.is_empty() { self.t.kern(left, right) } else { self.kern.pair(left, right) as i16 }
    }

    fn scale(&self, px: f32) -> f32 {
        px.clamp(1.0, MAX_PX) / self.t.units_per_em as f32
    }

    pub fn advance(&self, gid: u16, px: f32) -> f32 {
        self.advance_units(gid) as f32 * self.scale(px)
    }

    pub fn kern(&self, left: u16, right: u16, px: f32) -> f32 {
        self.kern_units(left, right) as f32 * self.scale(px)
    }

    pub fn line_metrics(&self, px: f32) -> LineMetrics {
        let s = self.scale(px);
        LineMetrics {
            ascent: self.t.ascender as f32 * s,
            descent: -(self.t.descender as f32) * s,
            line_gap: self.t.line_gap.max(0) as f32 * s,
        }
    }

    /// Obrys glyfu (jednotky pisma). Vadny glyf = prazdny obrys.
    pub fn outline(&self, gid: u16) -> Vec<Segment> {
        outline::glyph(&self.t, gid)
    }

    /// Vykresli glyf ve velikosti `px` (em v pixelech).
    pub fn rasterize(&self, gid: u16, px: f32) -> Glyph {
        self.rasterize_at(gid, px, 0.0)
    }

    /// Jako [`Font::rasterize`], glyf posunuty o `dx` (0..1) pixelu doprava
    /// (jemne umisteni: pero nemusi lezet na celem pixelu).
    pub fn rasterize_at(&self, gid: u16, px: f32, dx: f32) -> Glyph {
        let s = self.scale(px);
        let advance = self.advance_units(gid) as f32 * s;
        let segs = self.outline(gid);
        let mut g = raster::render(&segs, s, dx.clamp(0.0, 1.0));
        g.advance = advance;
        g
    }

    /// Sirka textu v pixelech (s kerningem, pero zaokrouhlene na pixely
    /// stejne jako pri kresleni).
    pub fn text_width(&self, text: &str, px: f32) -> u32 {
        let mut x = 0.0f32;
        let mut prev: Option<u16> = None;
        for ch in text.chars() {
            let g = self.glyph_index(ch);
            if let Some(p) = prev {
                x += self.kern(p, g, px);
            }
            x += self.advance(g, px);
            prev = Some(g);
        }
        math::ceil(x) as u32
    }
}
