//! Stin zaobleneho obdelniku: Gaussovo rozmazani predpocitane do masky
//! z 9 dilu (rohy, hrany, stred), ktera se pri kresleni jen natahne.
//!
//! Maska vznikne jednou pro kombinaci (polomer, rozmazani); kresleni stinu
//! okna libovolne velikosti je pak jen cteni z tabulky - pohyb okna stin
//! nezdrazi. Rozmazani je CSS `blur` (smerodatna odchylka = blur / 2),
//! aproximace Gaussova jadra tremi pruchody krabickoveho filtru.

use alloc::vec;
use alloc::vec::Vec;

use crate::shape::{coverage, round_rect_distance};
use crate::{Canvas, Rect, over, scale};

/// Predpocitany stin jedne vrstvy.
#[derive(Debug, Clone)]
pub struct ShadowMask {
    /// Rozmer masky (ctverec `n x n`).
    n: u32,
    /// Velikost rohoveho dilu (od okraje masky ke stredu).
    t: u32,
    /// Okraj rozmazani kolem tvaru (px).
    m: u32,
    /// Alfa 0..=255.
    a: Vec<u8>,
}

/// Nejvetsi rozmazani a polomer (ochrana pameti: maska je `n^2` bajtu).
const MAX_BLUR: u32 = 128;
const MAX_RADIUS: u32 = 128;

impl ShadowMask {
    /// Maska pro tvar s polomerem `radius` rozmazany o `blur`.
    pub fn new(radius: u32, blur: u32) -> Self {
        let (r, blur) = (radius.min(MAX_RADIUS), blur.min(MAX_BLUR));
        let m = blur; // 2 sigma; Gauss je za 2 sigma pod 2,5 %.
        let t = r + 2 * m;
        let n = 2 * t + 1;
        let shape = Rect::new(m as i32, m as i32, n - 2 * m, n - 2 * m);
        let mut a = vec![0f32; (n * n) as usize];
        for y in 0..n {
            for x in 0..n {
                let d = round_rect_distance(x as f32 + 0.5, y as f32 + 0.5, shape, r as f32);
                a[(y * n + x) as usize] = coverage(d) as f32;
            }
        }
        if blur > 0 {
            let sigma = blur as f32 / 2.0;
            for w in box_sizes(sigma) {
                box_pass(&mut a, n, w, true);
                box_pass(&mut a, n, w, false);
            }
        }
        Self { n, t, m, a: a.iter().map(|&v| (v + 0.5).clamp(0.0, 255.0) as u8) .collect() }
    }

    /// Okraj stinu kolem tvaru (o kolik stin presahuje).
    pub fn margin(&self) -> u32 {
        self.m
    }

    /// Alfa v bode (`lx`, `ly`) plochy stinu velke `aw x ah`.
    #[inline]
    fn at(&self, lx: u32, ly: u32, aw: u32, ah: u32) -> u8 {
        let map = |l: u32, len: u32| -> u32 {
            let half = len / 2;
            if l < self.t.min(half) {
                l
            } else if len - l <= self.t.min(len - half) {
                self.n - (len - l)
            } else {
                self.t
            }
        };
        let (mx, my) = (map(lx, aw), map(ly, ah));
        self.a.get((my * self.n + mx) as usize).copied().unwrap_or(0)
    }
}

/// Sirky tri krabickovych filtru pro Gauss se smerodatnou odchylkou
/// `sigma` (W. Jarosz, "Fast Image Convolutions").
fn box_sizes(sigma: f32) -> [u32; 3] {
    let n = 3.0;
    let w_ideal = crate::sqrtf(12.0 * sigma * sigma / n + 1.0);
    let mut wl = w_ideal as u32;
    if wl.is_multiple_of(2) {
        wl = wl.saturating_sub(1);
    }
    let wl = wl.max(1);
    let wu = wl + 2;
    let m_ideal = (12.0 * sigma * sigma - n * (wl * wl) as f32 - 4.0 * n * wl as f32 - 3.0 * n) / (-4.0 * wl as f32 - 4.0);
    let m = (m_ideal + 0.5).max(0.0) as u32;
    let mut out = [wu; 3];
    for (i, o) in out.iter_mut().enumerate() {
        if (i as u32) < m {
            *o = wl;
        }
    }
    out
}

/// Jeden pruchod krabickoveho filtru sirky `w` (liche) po radcich nebo
/// sloupcich; mimo masku je nula.
fn box_pass(a: &mut [f32], n: u32, w: u32, rows: bool) {
    let r = (w / 2) as i32;
    let n = n as i32;
    let mut line = vec![0f32; n as usize];
    let inv = 1.0 / w as f32;
    for k in 0..n {
        let idx = |i: i32| -> usize { if rows { (k * n + i) as usize } else { (i * n + k) as usize } };
        for i in 0..n {
            line[i as usize] = a[idx(i)];
        }
        // Klouzavy soucet.
        let mut acc = 0f32;
        for i in -r..=r {
            if (0..n).contains(&i) {
                acc += line[i as usize];
            }
        }
        for i in 0..n {
            a[idx(i)] = acc * inv;
            let (add, sub) = (i + r + 1, i - r);
            if (0..n).contains(&add) {
                acc += line[add as usize];
            }
            if (0..n).contains(&sub) {
                acc -= line[sub as usize];
            }
        }
    }
}

impl Canvas<'_> {
    /// Nakresli stin tvaru `shape` (uz posunuteho a rozsireneho podle
    /// vrstvy stinu) barvou `color`. `skip` je oblast, kterou stejne prekryje
    /// nepruhledne okno (nekresli se).
    pub fn shadow(&mut self, mask: &ShadowMask, shape: Rect, color: u32, skip: Rect) {
        self.shadow_layers(&[(mask, shape, color)], skip);
    }

    /// Vic vrstev stinu (odspodu nahoru): v kazdem pixelu se vrstvy nejdriv
    /// slozi mezi sebou a vysledek se prolne jednou. Proto je vysledek stejny,
    /// at se kresli primo do snimku, nebo do pruhledneho mezibufferu, ktery
    /// se prolne pozdeji (GPU backend).
    pub fn shadow_layers(&mut self, layers: &[(&ShadowMask, Rect, u32)], skip: Rect) {
        let areas: alloc::vec::Vec<Rect> = layers
            .iter()
            .map(|(m, s, _)| Rect::new(s.x - m.m as i32, s.y - m.m as i32, s.w + 2 * m.m, s.h + 2 * m.m))
            .collect();
        let all = areas.iter().fold(Rect::default(), |a, r| a.union(r));
        self.rows(all, |row, rx, y| {
            let skip_row = y >= skip.y && (y as i64) < skip.bottom();
            for (i, p) in row.iter_mut().enumerate() {
                let x = rx + i as i32;
                if skip_row && x >= skip.x && (x as i64) < skip.right() {
                    continue;
                }
                let mut s = 0u32;
                for ((mask, _, color), area) in layers.iter().zip(&areas) {
                    if !area.contains(x, y) {
                        continue;
                    }
                    let k = mask.at((x - area.x) as u32, (y - area.y) as u32, area.w, area.h) as u32;
                    if k > 0 {
                        s = over(s, scale(*color, k));
                    }
                }
                if s != 0 {
                    *p = over(*p, s);
                }
            }
        });
    }
}
