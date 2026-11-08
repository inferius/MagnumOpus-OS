//! Softwarove kresleni pro kompozitor a knihovnu UI.
//!
//! Pixel je `u32` 0xAARRGGBB (v pameti bajty B, G, R, A - format
//! virtio-gpu `B8G8R8A8_UNORM`) s predem vynasobenou alfou. Plocha
//! ([`Canvas`]) je pohled na cizi pamet (buffer aplikace, snimek
//! kompozitoru), knihovna sama nic nealokuje.
//!
//! Vsechny operace orezavaji na plochu a na `clip`; zadna nesaha mimo
//! pamet, i kdyz dostane nesmyslne souradnice.

#![cfg_attr(not(test), no_std)]

extern crate alloc;

pub mod backdrop;
pub mod damage;
pub mod icon;
pub mod png;
pub mod shadow;
pub mod shape;
pub mod text;

#[cfg(test)]
mod tests;

pub use guiproto::Rect;

/// Odmocnina bez `std` (Newtonova metoda z odhadu podle exponentu).
pub fn sqrtf(x: f32) -> f32 {
    if !(x > 0.0) || !x.is_finite() {
        return 0.0;
    }
    let mut y = f32::from_bits((x.to_bits() >> 1) + 0x1FC0_0000);
    for _ in 0..4 {
        y = 0.5 * (y + x / y);
    }
    y
}

/// Dolni cela cast bez `std`.
pub fn floorf(x: f32) -> f32 {
    if !x.is_finite() || x.abs() >= 8_388_608.0 {
        return x;
    }
    let i = x as i32 as f32;
    if i > x { i - 1.0 } else { i }
}

/// Barva z RGB (nepruhledna).
pub const fn rgb(r: u8, g: u8, b: u8) -> u32 {
    0xFF00_0000 | (r as u32) << 16 | (g as u32) << 8 | b as u32
}

/// Barva z RGBA bez premultiplikace -> premultiplied.
pub const fn rgba(r: u8, g: u8, b: u8, a: u8) -> u32 {
    (a as u32) << 24 | premul(r, a) << 16 | premul(g, a) << 8 | premul(b, a)
}

const fn premul(c: u8, a: u8) -> u32 {
    (c as u32 * a as u32 + 127) / 255
}

/// Vynasobi vsechny slozky (premultiplied) cinitelem 0..=255.
#[inline]
pub const fn scale(c: u32, k: u32) -> u32 {
    if k >= 255 {
        return c;
    }
    // Dve slozky naraz (maska 0x00FF00FF).
    let rb = ((c & 0x00FF_00FF) * k + 0x0080_0080) >> 8 & 0x00FF_00FF;
    let ag = (((c >> 8) & 0x00FF_00FF) * k + 0x0080_0080) & 0xFF00_FF00;
    rb | ag
}

/// Slozeni src-over: `s + d * (1 - sa)`. Nad nepruhlednym pixelem zustane
/// vysledek nepruhledny (celociselne `scale` by alfu dal o 1 mensi).
#[inline]
pub const fn over(d: u32, s: u32) -> u32 {
    let sa = s >> 24;
    if sa == 255 {
        return s;
    }
    if sa == 0 && s == 0 {
        return d;
    }
    let inv = 255 - sa;
    let out = add_sat(s, scale(d, inv));
    if d >> 24 == 255 { out | 0xFF00_0000 } else { out }
}

/// Soucet slozek s nasycenim (pro premultiplied barvy nepretece, ale
/// vadny vstup aplikace muze mit slozku vetsi nez alfu).
#[inline]
pub(crate) const fn add_sat(a: u32, b: u32) -> u32 {
    let mut out = 0;
    let mut sh = 0;
    while sh < 32 {
        let v = ((a >> sh) & 0xFF) + ((b >> sh) & 0xFF);
        out |= (if v > 255 { 255 } else { v }) << sh;
        sh += 8;
    }
    out
}

/// Kresleni: plocha nad cizi pameti.
pub struct Canvas<'a> {
    px: &'a mut [u32],
    pub w: u32,
    pub h: u32,
    /// Krok radku v pixelech.
    pub stride: u32,
    /// Orez (vzdy uvnitr plochy).
    clip: Rect,
}

/// Obraz jen pro cteni (zdroj kopirovani).
#[derive(Clone, Copy)]
pub struct Image<'a> {
    px: &'a [u32],
    pub w: u32,
    pub h: u32,
    pub stride: u32,
}

impl<'a> Image<'a> {
    /// `None`, kdyz pamet na rozmery nestaci.
    pub fn new(px: &'a [u32], w: u32, h: u32, stride: u32) -> Option<Self> {
        if stride < w || (h as u64 * stride as u64) > px.len() as u64 {
            return None;
        }
        Some(Self { px, w, h, stride })
    }

    pub fn get(&self, x: u32, y: u32) -> u32 {
        if x < self.w && y < self.h {
            self.px[(y * self.stride + x) as usize]
        } else {
            0
        }
    }

    pub fn row(&self, y: u32) -> &'a [u32] {
        let s = (y * self.stride) as usize;
        &self.px[s..s + self.w as usize]
    }
}

impl<'a> Canvas<'a> {
    /// `None`, kdyz pamet na rozmery nestaci.
    pub fn new(px: &'a mut [u32], w: u32, h: u32, stride: u32) -> Option<Self> {
        if stride < w || (h as u64 * stride as u64) > px.len() as u64 {
            return None;
        }
        Some(Self {
            px,
            w,
            h,
            stride,
            clip: Rect::new(0, 0, w, h),
        })
    }

    pub fn bounds(&self) -> Rect {
        Rect::new(0, 0, self.w, self.h)
    }

    /// Pixely platna (cteni, napr. odeslani nakresleneho vyrezu GPU).
    pub fn pixels(&self) -> &[u32] {
        self.px
    }

    /// Krok radku v pixelech.
    pub fn stride(&self) -> u32 {
        self.stride
    }

    pub fn clip(&self) -> Rect {
        self.clip
    }

    /// Nastavi orez (prunik s plochou).
    pub fn set_clip(&mut self, r: Rect) {
        self.clip = r.intersect(&self.bounds());
    }

    pub fn reset_clip(&mut self) {
        self.clip = self.bounds();
    }

    pub fn as_image(&self) -> Image<'_> {
        Image {
            px: self.px,
            w: self.w,
            h: self.h,
            stride: self.stride,
        }
    }

    pub fn get(&self, x: u32, y: u32) -> u32 {
        self.as_image().get(x, y)
    }

    pub(crate) fn rows(&mut self, r: Rect, mut f: impl FnMut(&mut [u32], i32, i32)) {
        let r = r.intersect(&self.clip);
        if r.is_empty() {
            return;
        }
        for y in r.y..r.y + r.h as i32 {
            let s = (y as u32 * self.stride + r.x as u32) as usize;
            f(&mut self.px[s..s + r.w as usize], r.x, y);
        }
    }

    /// Vyplni obdelnik barvou (prepise, bez prolinani).
    pub fn fill(&mut self, r: Rect, c: u32) {
        self.rows(r, |row, _, _| row.fill(c));
    }

    /// Prolne barvu do obdelniku (src-over).
    pub fn blend(&mut self, r: Rect, c: u32) {
        if c >> 24 == 255 {
            return self.fill(r, c);
        }
        if c == 0 {
            return;
        }
        self.rows(r, |row, _, _| {
            for p in row {
                *p = over(*p, c);
            }
        });
    }

    /// Ram obdelniku sirky `t`.
    pub fn outline(&mut self, r: Rect, t: u32, c: u32) {
        if r.is_empty() {
            return;
        }
        let t = t.min(r.w / 2 + 1).min(r.h / 2 + 1);
        self.blend(Rect::new(r.x, r.y, r.w, t), c);
        self.blend(Rect::new(r.x, (r.bottom() - t as i64) as i32, r.w, t), c);
        let inner = r.h.saturating_sub(2 * t);
        self.blend(Rect::new(r.x, r.y + t as i32, t, inner), c);
        self.blend(Rect::new((r.right() - t as i64) as i32, r.y + t as i32, t, inner), c);
    }

    /// Vyplni obdelnik se zaoblenymi rohy polomeru `radius` (vyhlazene).
    pub fn rounded(&mut self, r: Rect, radius: u32, c: u32) {
        let rad = radius.min(r.w / 2).min(r.h / 2) as i32;
        if rad == 0 {
            return self.blend(r, c);
        }
        let (x0, y0) = (r.x, r.y);
        let (w, h) = (r.w as i32, r.h as i32);
        self.rows(r, |row, rx, y| {
            let ly = y - y0;
            for (i, p) in row.iter_mut().enumerate() {
                let lx = rx + i as i32 - x0;
                // Vzdalenost od stredu rohu (jen v rozich).
                let cx = if lx < rad { rad - lx } else if lx >= w - rad { lx - (w - rad) + 1 } else { 0 };
                let cy = if ly < rad { rad - ly } else if ly >= h - rad { ly - (h - rad) + 1 } else { 0 };
                let k = if cx > 0 && cy > 0 { corner_coverage(cx, cy, rad) } else { 255 };
                if k > 0 {
                    *p = over(*p, scale(c, k));
                }
            }
        });
    }

    /// Prolne masku pokryti (napr. glyf: 0 = nic, 255 = plne) barvou `c`;
    /// leva horni rohova pozice masky je `(x, y)`.
    pub fn mask(&mut self, x: i32, y: i32, mw: u32, mh: u32, mask: &[u8], c: u32) {
        self.mask_lut(x, y, mw, mh, mask, c, None);
    }

    /// Jako [`Canvas::mask`], pokryti nejdriv projde tabulkou `lut`
    /// (gama textu).
    #[allow(clippy::too_many_arguments)]
    pub fn mask_lut(&mut self, x: i32, y: i32, mw: u32, mh: u32, mask: &[u8], c: u32, lut: Option<&[u8; 256]>) {
        if mask.len() < (mw * mh) as usize {
            return;
        }
        self.rows(Rect::new(x, y, mw, mh), |row, rx, ry| {
            let my = (ry - y) as u32;
            for (i, p) in row.iter_mut().enumerate() {
                let mx = (rx - x) as u32 + i as u32;
                let mut k = mask[(my * mw + mx) as usize];
                if let Some(l) = lut {
                    k = l[k as usize];
                }
                if k > 0 {
                    *p = over(*p, scale(c, k as u32));
                }
            }
        });
    }

    /// Zkopiruje vyrez `src_rect` obrazu `src` na obdelnik `dst` (skaluje
    /// nejblizsim sousedem, kdyz se rozmery lisi) s pruhlednosti
    /// `opacity` a prolinanim src-over. `opaque` = zdroj je nepruhledny
    /// (rychla kopie bez prolinani pri `opacity` 255).
    pub fn draw_image(&mut self, src: &Image<'_>, src_rect: Rect, dst: Rect, opacity: u8, opaque: bool) {
        let sr = src_rect.intersect(&Rect::new(0, 0, src.w, src.h));
        if sr.is_empty() || dst.is_empty() || opacity == 0 {
            return;
        }
        // Pomer stran se pocita z puvodniho vyrezu (orez zdroje zmensi cil).
        let (sw, sh) = (src_rect.w as i64, src_rect.h as i64);
        let k = opacity as u32;
        let same = src_rect.w == dst.w && src_rect.h == dst.h;
        self.rows(dst, |row, rx, y| {
            let ly = (y - dst.y) as i64;
            let sy = src_rect.y as i64 + if same { ly } else { ly * sh / dst.h as i64 };
            if sy < sr.y as i64 || sy >= sr.bottom() {
                return;
            }
            let srow = src.row(sy as u32);
            if same && opaque && k == 255 {
                // Rychla cesta: souvisla kopie radku.
                let lx0 = (rx - dst.x) as i64 + src_rect.x as i64;
                let (a, b) = (lx0.max(sr.x as i64), (lx0 + row.len() as i64).min(sr.right()));
                if b > a {
                    let off = (a - lx0) as usize;
                    row[off..off + (b - a) as usize].copy_from_slice(&srow[a as usize..b as usize]);
                }
                return;
            }
            for (i, p) in row.iter_mut().enumerate() {
                let lx = (rx - dst.x) as i64 + i as i64;
                let sx = src_rect.x as i64 + if same { lx } else { lx * sw / dst.w as i64 };
                if sx < sr.x as i64 || sx >= sr.right() {
                    continue;
                }
                let s = scale(srow[sx as usize], k);
                *p = if opaque && k == 255 { s } else { over(*p, s) };
            }
        });
    }
}

/// Pokryti pixelu rohu (0..=255): pixel v rohu na pozici (`cx`, `cy`)
/// od stredu kruznice polomeru `r`, 4x4 vzorky (celociselne).
fn corner_coverage(cx: i32, cy: i32, r: i32) -> u32 {
    let r2 = (r * 4) * (r * 4);
    let mut n = 0;
    for sy in 0..4 {
        for sx in 0..4 {
            // Stred podvzorku ve ctvrtinach pixelu.
            let dx = (cx - 1) * 4 + sx;
            let dy = (cy - 1) * 4 + sy;
            if dx * dx + dy * dy < r2 {
                n += 1;
            }
        }
    }
    n * 255 / 16
}
