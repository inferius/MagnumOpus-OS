//! Kresleni textu pismem z `libs/font` s mezipameti glyfu.
//!
//! - **Jemne umisteni**: pero lezi na ctvrtinach pixelu; glyf se vykresli
//!   pro jeden ze ctyr posunu (klic mezipameti), takze mezery mezi pismeny
//!   odpovidaji metrikam pisma a nekoliskuji po celych pixelech.
//! - **Gama**: tmavy text na svetlem pozadi by pri prolinani v sRGB vypadal
//!   tence; pokryti `c` se proto u tmaveho textu zesili tabulkou
//!   `(1 - k) c + k sqrt(c)` s `k` = [`DARK_TEXT_BOOST`] (priblizne
//!   `c^(1/1.35)`). Svetly text na tmavem pozadi se nemeni.
//! - **Rodina** ([`Family`]): vic rezu jednoho pisma (vahy z tokenu),
//!   vyber nejblizsiho rezu podle pravidel CSS.

use alloc::collections::BTreeMap;
use alloc::vec::Vec;

use font::{Font, Glyph};

use crate::Canvas;

/// Nejvic glyfu v mezipameti (pak se cela vyprazdni).
const CACHE_MAX: usize = 4096;
/// Pocet poloh glyfu v jednom pixelu (jemne umisteni).
pub const SUBPIXEL: u32 = 4;
/// Podil odmocniny v zesileni pokryti tmaveho textu (0 = bez zmeny).
pub const DARK_TEXT_BOOST: f32 = 0.35;

/// Pismo s mezipameti vykreslenych glyfu (klic: glyf, velikost v
/// ctvrtinach pixelu, posun v ctvrtinach pixelu).
pub struct Text {
    font: Font<'static>,
    cache: BTreeMap<(u16, u32, u8), Glyph>,
    /// Tabulka gama pro tmavy text.
    dark_lut: [u8; 256],
}

/// Kurzor pera pri kresleni po znacich (pro textova pole a mrizku).
#[derive(Debug, Clone, Copy)]
pub struct Pen {
    pub x: f32,
    prev: Option<u16>,
}

fn gamma_lut(k: f32) -> [u8; 256] {
    let mut t = [0u8; 256];
    let k = k.clamp(0.0, 1.0);
    for (i, v) in t.iter_mut().enumerate() {
        let x = i as f32 / 255.0;
        let y = x * (1.0 - k) + crate::sqrtf(x) * k;
        *v = (y * 255.0 + 0.5).min(255.0) as u8;
    }
    t
}

impl Text {
    pub fn new(font: Font<'static>) -> Self {
        Self { font, cache: BTreeMap::new(), dark_lut: gamma_lut(DARK_TEXT_BOOST) }
    }

    /// Zalozni pismo systemu (Tuffy).
    pub fn system() -> Self {
        Self::new(Font::parse(font::TUFFY).expect("vestavene pismo je platne"))
    }

    pub fn font(&self) -> &Font<'static> {
        &self.font
    }

    fn key(px: f32) -> u32 {
        (px.clamp(1.0, font::MAX_PX) * 4.0) as u32
    }

    fn glyph(&mut self, gid: u16, px: f32, sub: u8) -> &Glyph {
        let k = (gid, Self::key(px), sub);
        if !self.cache.contains_key(&k) {
            if self.cache.len() >= CACHE_MAX {
                self.cache.clear();
            }
            let g = self.font.rasterize_at(gid, k.1 as f32 / 4.0, sub as f32 / SUBPIXEL as f32);
            self.cache.insert(k, g);
        }
        &self.cache[&k]
    }

    /// Vyska radku a uroven zakladny od horniho okraje radku.
    pub fn line(&self, px: f32) -> (u32, i32) {
        let m = self.font.line_metrics(px);
        (m.height(), font_ceil(m.ascent) as i32)
    }

    /// Zakladna pro radek vysky `line_h` (text svisle na stred podle
    /// metrik pisma, jako `line-height` v CSS).
    pub fn baseline_in(&self, px: f32, line_h: u32) -> i32 {
        let m = self.font.line_metrics(px);
        let content = m.ascent + m.descent;
        let top = (line_h as f32 - content) / 2.0;
        font_round(top + m.ascent)
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
        font_round(pen.x)
    }

    /// Text zkraceny na sirku `max` (tri tecky na konci, kdyz se nevejde).
    /// Vraci konec pera.
    #[allow(clippy::too_many_arguments)]
    pub fn draw_ellipsized(&mut self, c: &mut Canvas<'_>, x: i32, baseline: i32, s: &str, px: f32, color: u32, max: u32) -> i32 {
        if self.width(s, px) <= max {
            return self.draw(c, x, baseline, s, px, color);
        }
        let dots = "...";
        let room = max.saturating_sub(self.width(dots, px));
        let mut end = 0;
        for (i, ch) in s.char_indices() {
            let next = i + ch.len_utf8();
            if self.width(&s[..next], px) > room {
                break;
            }
            end = next;
        }
        let pen = self.draw(c, x, baseline, s[..end].trim_end(), px, color);
        self.draw(c, pen, baseline, dots, px, color)
    }

    /// Jeden znak (s kerningem vuci predchozimu).
    pub fn draw_char(&mut self, c: &mut Canvas<'_>, pen: &mut Pen, baseline: i32, ch: char, px: f32, color: u32) {
        let gid = self.font.glyph_index(ch);
        if let Some(p) = pen.prev {
            pen.x += self.font.kern(p, gid, px);
        }
        // Cela cast pera a posun v ctvrtinach pixelu.
        let q = font_round(pen.x * SUBPIXEL as f32);
        let x = q.div_euclid(SUBPIXEL as i32);
        let sub = q.rem_euclid(SUBPIXEL as i32) as u8;
        let dark = is_dark(color);
        let lut: *const [u8; 256] = &self.dark_lut;
        let g = self.glyph(gid, px, sub);
        let adv = g.advance;
        if g.w > 0 && g.h > 0 {
            let (w, h, l, t) = (g.w, g.h, g.left, g.top);
            // Glyf je v mezipameti; maska se pujci (kopie by byla draha).
            let cov: *const Vec<u8> = &g.coverage;
            // SAFETY: `cov` a `lut` ukazuji do `self` (mezipamet, tabulka),
            // ktere se behem volani `mask_lut` nemeni (k `self` nepristupuje).
            let lut = if dark { Some(unsafe { &*lut }) } else { None };
            c.mask_lut(x + l, baseline + t, w, h, unsafe { &*cov }, color, lut);
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

/// Je barva (premultiplied) tmava (jas pod polovinou)?
fn is_dark(c: u32) -> bool {
    let a = c >> 24;
    if a == 0 {
        return false;
    }
    let (r, g, b) = ((c >> 16 & 0xFF) * 255 / a, (c >> 8 & 0xFF) * 255 / a, (c & 0xFF) * 255 / a);
    (r * 2 + g * 5 + b) / 8 < 128
}

impl Pen {
    pub fn at(x: i32) -> Self {
        Self { x: x as f32, prev: None }
    }
}

/// Rodina pisma: rezy podle vahy (vahy z tokenu, vyber jako v CSS).
pub struct Family {
    faces: Vec<(u16, Text)>,
}

impl Family {
    /// Rodina z dat rezu `(vaha, TTF)` se zapnutymi vlastnostmi `GSUB`.
    /// Poskozeny rez se preskoci; bez rezu zalozni pismo.
    pub fn new(faces: &[(u16, &'static [u8])], features: &[[u8; 4]]) -> Self {
        let mut out = Vec::new();
        for &(w, data) in faces {
            if let Ok(f) = Font::parse(data) {
                out.push((w, Text::new(f.with_features(features))));
            }
        }
        if out.is_empty() {
            out.push((400, Text::system()));
        }
        Self { faces: out }
    }

    /// Jen zalozni pismo (testy, nouzovy rezim).
    pub fn system() -> Self {
        Self { faces: alloc::vec![(400, Text::system())] }
    }

    /// Rez pro vahu `weight`.
    pub fn face(&mut self, weight: u16) -> &mut Text {
        let have: Vec<u16> = self.faces.iter().map(|f| f.0).collect();
        let i = font::pick_weight(&have, weight).unwrap_or(0);
        &mut self.faces[i].1
    }

    /// Rez pro vahu `weight` (jen cteni: sirky, metriky).
    pub fn face_ref(&self, weight: u16) -> &Text {
        let have: Vec<u16> = self.faces.iter().map(|f| f.0).collect();
        let i = font::pick_weight(&have, weight).unwrap_or(0);
        &self.faces[i].1
    }

    pub fn weights(&self) -> Vec<u16> {
        self.faces.iter().map(|f| f.0).collect()
    }
}

fn font_round(x: f32) -> i32 {
    if x >= 0.0 { (x + 0.5) as i32 } else { (x - 0.5) as i32 }
}

fn font_ceil(x: f32) -> f32 {
    let i = x as i32 as f32;
    if i < x { i + 1.0 } else { i }
}
