//! Kresleni textu pismem z `libs/font` s mezipameti glyfu.

use alloc::collections::BTreeMap;
use alloc::vec::Vec;

use font::{Font, Glyph};

use crate::Canvas;

/// Nejvic glyfu v mezipameti (pak se cela vyprazdni).
const CACHE_MAX: usize = 2048;

/// Pismo s mezipameti vykreslenych glyfu (klic: glyf a velikost v
/// ctvrtinach pixelu).
pub struct Text {
    font: Font<'static>,
    cache: BTreeMap<(u16, u32), Glyph>,
}

/// Kurzor pera pri kresleni po znacich (pro textova pole a mrizku).
#[derive(Debug, Clone, Copy)]
pub struct Pen {
    pub x: f32,
    prev: Option<u16>,
}

impl Text {
    pub fn new(font: Font<'static>) -> Self {
        Self {
            font,
            cache: BTreeMap::new(),
        }
    }

    /// Vychozi pismo systemu.
    pub fn system() -> Self {
        Self::new(Font::parse(font::TUFFY).expect("vestavene pismo je platne"))
    }

    pub fn font(&self) -> &Font<'static> {
        &self.font
    }

    fn key(px: f32) -> u32 {
        (px.clamp(1.0, font::MAX_PX) * 4.0) as u32
    }

    fn glyph(&mut self, gid: u16, px: f32) -> &Glyph {
        let k = (gid, Self::key(px));
        if !self.cache.contains_key(&k) {
            if self.cache.len() >= CACHE_MAX {
                self.cache.clear();
            }
            let g = self.font.rasterize(gid, k.1 as f32 / 4.0);
            self.cache.insert(k, g);
        }
        &self.cache[&k]
    }

    /// Vyska radku a uroven zakladny od horniho okraje radku.
    pub fn line(&self, px: f32) -> (u32, i32) {
        let m = self.font.line_metrics(px);
        (m.height(), font_ceil(m.ascent) as i32)
    }

    pub fn width(&self, s: &str, px: f32) -> u32 {
        self.font.text_width(s, px)
    }

    /// Nakresli text, `(x, baseline)` je pocatek pera na zakladne. Vraci
    /// konec pera (x).
    pub fn draw(&mut self, c: &mut Canvas<'_>, x: i32, baseline: i32, s: &str, px: f32, color: u32) -> i32 {
        let mut pen = Pen { x: x as f32, prev: None };
        for ch in s.chars() {
            self.draw_char(c, &mut pen, baseline, ch, px, color);
        }
        pen.x as i32
    }

    /// Jeden znak (s kerningem vuci predchozimu).
    pub fn draw_char(&mut self, c: &mut Canvas<'_>, pen: &mut Pen, baseline: i32, ch: char, px: f32, color: u32) {
        let gid = self.font.glyph_index(ch);
        if let Some(p) = pen.prev {
            pen.x += self.font.kern(p, gid, px);
        }
        let x = font_round(pen.x);
        let g = self.glyph(gid, px);
        let adv = g.advance;
        if g.w > 0 && g.h > 0 {
            let (w, h, l, t) = (g.w, g.h, g.left, g.top);
            // Glyf je v mezipameti; maska se pujci (kopie by byla draha).
            let cov: *const Vec<u8> = &g.coverage;
            // SAFETY: `cov` ukazuje do polozky mezipameti, ktera se behem
            // volani `mask` nemeni (mask k `self` nepristupuje).
            c.mask(x + l, baseline + t, w, h, unsafe { &*cov }, color);
        }
        pen.x += adv;
        pen.prev = Some(gid);
    }

    /// Pozice pera po kazdem znaku (pro kurzor a klik v textovem poli):
    /// `out[i]` = x pred znakem `i`, posledni = konec.
    pub fn positions(&self, s: &str, px: f32) -> Vec<i32> {
        let mut out = Vec::with_capacity(s.len() + 1);
        let mut x = 0.0f32;
        let mut prev = None;
        for ch in s.chars() {
            let g = self.font.glyph_index(ch);
            if let Some(p) = prev {
                x += self.font.kern(p, g, px);
            }
            out.push(font_round(x));
            x += self.font.advance(g, px);
            prev = Some(g);
        }
        out.push(font_round(x));
        out
    }
}

impl Pen {
    pub fn at(x: i32) -> Self {
        Self { x: x as f32, prev: None }
    }
}

fn font_round(x: f32) -> i32 {
    if x >= 0.0 { (x + 0.5) as i32 } else { (x - 0.5) as i32 }
}

fn font_ceil(x: f32) -> f32 {
    let i = x as i32 as f32;
    if i < x { i + 1.0 } else { i }
}
