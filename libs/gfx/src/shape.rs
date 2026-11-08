//! Tvary s vyhlazenymi hranami: zaobleny obdelnik (vypln a ram), orez rohu
//! obsahu okna.
//!
//! Pokryti pixelu se pocita analyticky ze vzdalenosti od hrany (vzdalenostni
//! pole zaobleneho obdelniku): pixel o stredu ve vzdalenosti `d` od hrany
//! (zaporne uvnitr) ma pokryti `clamp(0.5 - d)`. Stejny vzorec ma pozdeji
//! shader na GPU, takze obe cesty vypadaji stejne.

use alloc::vec::Vec;

use crate::{Canvas, Rect, add_sat, over, scale};

/// Vzdalenost bodu (`px`, `py`, stred pixelu) od hrany zaobleneho
/// obdelniku `r` s polomerem `rad` (zaporne uvnitr).
#[inline]
pub fn round_rect_distance(px: f32, py: f32, r: Rect, rad: f32) -> f32 {
    let hw = r.w as f32 * 0.5;
    let hh = r.h as f32 * 0.5;
    let rad = rad.min(hw).min(hh).max(0.0);
    let cx = r.x as f32 + hw;
    let cy = r.y as f32 + hh;
    let qx = (px - cx).abs() - (hw - rad);
    let qy = (py - cy).abs() - (hh - rad);
    let ox = qx.max(0.0);
    let oy = qy.max(0.0);
    let outside = crate::sqrtf(ox * ox + oy * oy);
    outside + qx.max(qy).min(0.0) - rad
}

/// Pokryti 0..=255 ze vzdalenosti od hrany.
#[inline]
pub fn coverage(d: f32) -> u32 {
    let c = 0.5 - d;
    if c <= 0.0 {
        0
    } else if c >= 1.0 {
        255
    } else {
        (c * 255.0 + 0.5) as u32
    }
}

/// Je radek `y` v pasu, kde zaobleni nehraje roli (cely radek je uvnitr)?
#[inline]
fn straight_row(r: Rect, rad: i32, y: i32) -> bool {
    y >= r.y + rad && y < r.y + r.h as i32 - rad
}

impl Canvas<'_> {
    /// Vyplni zaobleny obdelnik barvou `c` (src-over) s vyhlazenymi rohy.
    pub fn round_fill(&mut self, r: Rect, radius: u32, c: u32) {
        let rad = radius.min(r.w / 2).min(r.h / 2) as i32;
        if rad == 0 {
            return self.blend(r, c);
        }
        let opaque = c >> 24 == 255;
        self.rows(r, |row, rx, y| {
            if straight_row(r, rad, y) {
                if opaque {
                    row.fill(c);
                } else {
                    for p in row.iter_mut() {
                        *p = over(*p, c);
                    }
                }
                return;
            }
            let fy = y as f32 + 0.5;
            for (i, p) in row.iter_mut().enumerate() {
                let x = rx + i as i32;
                // Mimo rohove ctverce je radek uvnitr.
                let k = if x >= r.x + rad && x < r.x + r.w as i32 - rad {
                    255
                } else {
                    coverage(round_rect_distance(x as f32 + 0.5, fy, r, rad as f32))
                };
                if k == 255 && opaque {
                    *p = c;
                } else if k > 0 {
                    *p = over(*p, scale(c, k));
                }
            }
        });
    }

    /// Obraz `img` (`r.w x r.h` pixelu) do obdelniku `r` jen uvnitr
    /// zaobleneho tvaru (vyhlazene hrany), src-over.
    pub fn round_image(&mut self, img: &[u32], r: Rect, radius: u32) {
        if img.len() < (r.w * r.h) as usize {
            return;
        }
        let rad = radius.min(r.w / 2).min(r.h / 2) as f32;
        self.rows(r, |row, rx, y| {
            let ly = (y - r.y) as u32;
            for (i, p) in row.iter_mut().enumerate() {
                let x = rx + i as i32;
                let k = coverage(round_rect_distance(x as f32 + 0.5, y as f32 + 0.5, r, rad));
                if k > 0 {
                    let s = img[(ly * r.w + (x - r.x) as u32) as usize];
                    *p = over(*p, scale(s, k));
                }
            }
        });
    }

    /// Ram zaobleneho obdelniku sirky `width` dovnitr od hrany `r`.
    pub fn round_stroke(&mut self, r: Rect, radius: u32, width: u32, c: u32) {
        if r.is_empty() || width == 0 {
            return;
        }
        let rad = radius.min(r.w / 2).min(r.h / 2) as f32;
        let wf = width as f32;
        let inner = Rect::new(r.x + width as i32, r.y + width as i32, r.w.saturating_sub(2 * width), r.h.saturating_sub(2 * width));
        let irad = (rad - wf).max(0.0);
        let band = rad as i32 + width as i32 + 1;
        self.rows(r, |row, rx, y| {
            let fy = y as f32 + 0.5;
            let edge_row = y < r.y + band || y >= r.y + r.h as i32 - band;
            for (i, p) in row.iter_mut().enumerate() {
                let x = rx + i as i32;
                // Uvnitr mimo ram (rychla cesta pro stred).
                if !edge_row && x >= r.x + width as i32 && x < r.x + r.w as i32 - width as i32 {
                    continue;
                }
                let fx = x as f32 + 0.5;
                let outer = coverage(round_rect_distance(fx, fy, r, rad));
                let inn = if inner.is_empty() { 0 } else { coverage(round_rect_distance(fx, fy, inner, irad)) };
                let k = outer.saturating_sub(inn);
                if k > 0 {
                    *p = over(*p, scale(c, k));
                }
            }
        });
    }

    /// Zaobleny obdelnik s vyplni `fill` a ramem `border` sirky `width`
    /// v jednom pruchodu: kazdy pixel se prolne jen jednou (vypln a ram se
    /// nejdriv slozi), takze vysledek nezavisi na tom, co je pod nim
    /// a v jakem poradi se vrstvy skladaji.
    pub fn round_fill_bordered(&mut self, r: Rect, radius: u32, fill: u32, border: u32, width: u32) {
        if r.is_empty() {
            return;
        }
        let rad = radius.min(r.w / 2).min(r.h / 2) as f32;
        let inner = Rect::new(r.x + width as i32, r.y + width as i32, r.w.saturating_sub(2 * width), r.h.saturating_sub(2 * width));
        let irad = (rad - width as f32).max(0.0);
        self.rows(r, |row, rx, y| {
            let fy = y as f32 + 0.5;
            for (i, p) in row.iter_mut().enumerate() {
                let fx = (rx + i as i32) as f32 + 0.5;
                let ko = coverage(round_rect_distance(fx, fy, r, rad));
                if ko == 0 {
                    continue;
                }
                let ki = if inner.is_empty() { 0 } else { coverage(round_rect_distance(fx, fy, inner, irad)).min(ko) };
                let s = add_sat(scale(fill, ki), scale(border, ko - ki));
                *p = over(*p, s);
            }
        });
    }

    /// Ram okna uvnitr obdelniku `r` (bez vnejsiho vyhlazeni: to dodaji
    /// rohy okna, viz [`Canvas::round_corner_patch`]). Pokryti je
    /// `255 - vnitrni tvar`.
    pub fn inner_ring(&mut self, r: Rect, radius: u32, width: u32, c: u32) {
        if r.is_empty() || width == 0 {
            return;
        }
        let rad = radius.min(r.w / 2).min(r.h / 2) as f32;
        let inner = Rect::new(r.x + width as i32, r.y + width as i32, r.w.saturating_sub(2 * width), r.h.saturating_sub(2 * width));
        let irad = (rad - width as f32).max(0.0);
        let band = rad as i32 + width as i32 + 1;
        self.rows(r, |row, rx, y| {
            let fy = y as f32 + 0.5;
            let edge_row = y < r.y + band || y >= r.y + r.h as i32 - band;
            for (i, p) in row.iter_mut().enumerate() {
                let x = rx + i as i32;
                if !edge_row && x >= r.x + width as i32 && x < r.x + r.w as i32 - width as i32 {
                    continue;
                }
                let ki = if inner.is_empty() { 0 } else { coverage(round_rect_distance(x as f32 + 0.5, fy, inner, irad)) };
                let k = 255 - ki;
                if k > 0 {
                    *p = over(*p, scale(c, k));
                }
            }
        });
    }

    /// Rohova zaplata okna: pixely v rohovych ctvercich `r` (polomer
    /// `radius`) vynasobi pokrytim zaobleneho tvaru (mimo tvar pruhledne).
    /// Pouzije se na nakresleny obsah okna v mezibufferu (GPU) nebo s
    /// [`Canvas::save_corners`] primo ve snimku (CPU).
    pub fn round_corner_patch(&mut self, r: Rect, radius: u32) {
        let rad = radius.min(r.w / 2).min(r.h / 2);
        let clip = self.clip;
        for (cx, cy) in corners(r, rad) {
            for y in 0..rad as i32 {
                for x in 0..rad as i32 {
                    let (px, py) = (cx + x, cy + y);
                    if !clip.contains(px, py) {
                        continue;
                    }
                    let k = coverage(round_rect_distance(px as f32 + 0.5, py as f32 + 0.5, r, rad as f32));
                    if k < 255 {
                        let idx = (py as u32 * self.stride + px as u32) as usize;
                        self.px[idx] = scale(self.px[idx], k);
                    }
                }
            }
        }
    }

    /// Ulozi pixely ctyr rohu obdelniku `r` (ctverce `radius` x `radius`)
    /// pred kreslenim okna; [`Canvas::restore_corners`] je pak vrati tam,
    /// kde je okno zaoblene. Tak se zaobli i obsah aplikace, ktery se
    /// kresli jako obycejny obdelnik.
    pub fn save_corners(&self, r: Rect, radius: u32) -> Vec<u32> {
        let rad = radius.min(r.w / 2).min(r.h / 2);
        let mut out = Vec::with_capacity((rad * rad * 4) as usize);
        for (cx, cy) in corners(r, rad) {
            for y in 0..rad as i32 {
                for x in 0..rad as i32 {
                    out.push(self.get_signed(cx + x, cy + y));
                }
            }
        }
        out
    }

    /// Vrati rohy ulozene [`Canvas::save_corners`]: mimo zaobleny tvar plne,
    /// na hrane podle pokryti (vyhlazene). Jen v orezu.
    pub fn restore_corners(&mut self, r: Rect, radius: u32, saved: &[u32]) {
        let rad = radius.min(r.w / 2).min(r.h / 2);
        if saved.len() < (rad * rad * 4) as usize {
            return;
        }
        let clip = self.clip;
        let mut i = 0;
        for (cx, cy) in corners(r, rad) {
            for y in 0..rad as i32 {
                for x in 0..rad as i32 {
                    let (px, py) = (cx + x, cy + y);
                    let under = saved[i];
                    i += 1;
                    if !clip.contains(px, py) {
                        continue;
                    }
                    let k = coverage(round_rect_distance(px as f32 + 0.5, py as f32 + 0.5, r, rad as f32));
                    if k == 255 {
                        continue;
                    }
                    let idx = (py as u32 * self.stride + px as u32) as usize;
                    let top = self.px[idx];
                    // Okno pres puvodni pozadi s pokrytim k.
                    self.px[idx] = over(under, scale(top, k));
                }
            }
        }
    }

    fn get_signed(&self, x: i32, y: i32) -> u32 {
        if x < 0 || y < 0 { 0 } else { self.get(x as u32, y as u32) }
    }
}

/// Rohove ctverce zaobleneho obdelniku (polomer `rad`).
pub fn corner_rects(r: Rect, rad: u32) -> [Rect; 4] {
    let rad = rad.min(r.w / 2).min(r.h / 2);
    corners(r, rad).map(|(x, y)| Rect::new(x, y, rad, rad))
}

/// Obdelnik bez rohovych ctvercu jako tri pruhy (horni a dolni mezi rohy,
/// prostredni pres celou sirku).
pub fn core_rects(r: Rect, rad: u32) -> [Rect; 3] {
    let rad = rad.min(r.w / 2).min(r.h / 2);
    let mid_h = r.h.saturating_sub(2 * rad);
    [
        Rect::new(r.x + rad as i32, r.y, r.w.saturating_sub(2 * rad), rad),
        Rect::new(r.x, r.y + rad as i32, r.w, mid_h),
        Rect::new(r.x + rad as i32, r.y + (rad + mid_h) as i32, r.w.saturating_sub(2 * rad), rad),
    ]
}

/// Leve horni rohy ctyr rohovych ctvercu.
fn corners(r: Rect, rad: u32) -> [(i32, i32); 4] {
    let (x1, y1) = (r.x + r.w as i32 - rad as i32, r.y + r.h as i32 - rad as i32);
    [(r.x, r.y), (x1, r.y), (r.x, y1), (x1, y1)]
}
