//! Pismo TrueType: cteni (`cmap`, `glyf`, `hmtx`, `kern`) a rasterizer
//! s plosnym vyhlazenim, bez hintingu a bez shapingu.
//!
//! Soubor pisma je neduveryhodny vstup: kazde cteni kontroluje meze,
//! vadny glyf je prazdny glyf, nikdy panika (test `garbage_never_panics`).
//!
//! Vychozi pismo systemu je Tuffy (Thatcher Ulrich, public domain):
//! [`TUFFY`]. Pokryva latinku vcetne cestiny.
//!
//! ```ignore
//! let font = font::Font::parse(font::TUFFY).unwrap();
//! let g = font.rasterize(font.glyph_index('č'), 16.0);
//! canvas.mask(x + g.left, baseline + g.top, g.w, g.h, &g.coverage, color);
//! ```

#![cfg_attr(not(test), no_std)]

extern crate alloc;

mod math;
mod outline;
mod raster;
mod tables;

#[cfg(test)]
mod tests;

use alloc::vec::Vec;

pub use raster::Rasterizer;

/// Vychozi pismo systemu (Tuffy, public domain).
pub static TUFFY: &[u8] = include_bytes!("../fonts/Tuffy.ttf");

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
}

/// Nejvetsi velikost pisma v pixelech (ochrana pameti rasterizeru).
pub const MAX_PX: f32 = 512.0;

impl<'a> Font<'a> {
    pub fn parse(data: &'a [u8]) -> Result<Self, FontError> {
        Ok(Self {
            t: tables::Tables::parse(data)?,
        })
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
        if g < self.t.num_glyphs { g } else { 0 }
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
        self.t.kern(left, right)
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
        let s = self.scale(px);
        let advance = self.advance_units(gid) as f32 * s;
        let segs = self.outline(gid);
        let mut g = raster::render(&segs, s);
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
