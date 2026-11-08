//! Rozmazani oblasti (pozadi pod listou: CSS `backdrop-filter: blur(16px)`).
//!
//! Gaussovo rozmazani se smerodatnou odchylkou `sigma` jako tri pruchody
//! krabickoveho filtru po radcich a sloupcich, v polovicnim rozliseni
//! (ctyrikrat mene pixelu, rozmazany obraz to nepozna) a zpet bilinearne.
//! Okraje se prodlouzi (zadne tmave lemovani).

use alloc::vec;
use alloc::vec::Vec;

use crate::Rect;
use crate::shadow::box_sizes;

/// Rozmaze oblast `area` obrazu `src` (krok `stride`, premultiplied);
/// vraci `area.w * area.h` pixelu. Oblast musi lezet v obrazu.
pub fn blurred_region(src: &[u32], stride: u32, area: Rect, sigma: f32) -> Vec<u32> {
    let (w, h) = (area.w as usize, area.h as usize);
    if w == 0 || h == 0 {
        return Vec::new();
    }
    let (hw, hh) = (w.div_ceil(2), h.div_ceil(2));
    // Polovicni rozliseni: prumer 2x2, ctyri kanaly v f32.
    let mut ch = vec![[0f32; 4]; hw * hh];
    for y in 0..hh {
        for x in 0..hw {
            let mut acc = [0f32; 4];
            let mut n = 0.0;
            for dy in 0..2 {
                for dx in 0..2 {
                    let (sx, sy) = (2 * x + dx, 2 * y + dy);
                    if sx >= w || sy >= h {
                        continue;
                    }
                    let p = src[(area.y as usize + sy) * stride as usize + area.x as usize + sx];
                    for (k, a) in acc.iter_mut().enumerate() {
                        *a += (p >> (24 - 8 * k) & 0xFF) as f32;
                    }
                    n += 1.0;
                }
            }
            ch[y * hw + x] = acc.map(|v| v / n);
        }
    }
    for bw in box_sizes((sigma / 2.0).max(0.5)) {
        box_pass(&mut ch, hw, hh, bw, true);
        box_pass(&mut ch, hw, hh, bw, false);
    }
    // Zpet do plneho rozliseni (bilinearne).
    let mut out = Vec::with_capacity(w * h);
    for y in 0..h {
        let fy = (y as f32 - 0.5) / 2.0;
        let y0 = (fy.max(0.0) as usize).min(hh - 1);
        let y1 = (y0 + 1).min(hh - 1);
        let ty = (fy - y0 as f32).clamp(0.0, 1.0);
        for x in 0..w {
            let fx = (x as f32 - 0.5) / 2.0;
            let x0 = (fx.max(0.0) as usize).min(hw - 1);
            let x1 = (x0 + 1).min(hw - 1);
            let tx = (fx - x0 as f32).clamp(0.0, 1.0);
            let (a, b, c, d) = (ch[y0 * hw + x0], ch[y0 * hw + x1], ch[y1 * hw + x0], ch[y1 * hw + x1]);
            let mut px = 0u32;
            for k in 0..4 {
                let top = a[k] + (b[k] - a[k]) * tx;
                let bot = c[k] + (d[k] - c[k]) * tx;
                let v = (top + (bot - top) * ty + 0.5).clamp(0.0, 255.0) as u32;
                px |= v << (24 - 8 * k);
            }
            out.push(px);
        }
    }
    out
}

/// Krabickovy filtr sirky `bw` po radcich (`rows`) nebo sloupcich, okraje
/// prodlouzene.
fn box_pass(a: &mut [[f32; 4]], w: usize, h: usize, bw: u32, rows: bool) {
    let r = (bw / 2) as isize;
    let (n, lines) = if rows { (w, h) } else { (h, w) };
    let mut line = vec![[0f32; 4]; n];
    let inv = 1.0 / bw as f32;
    for l in 0..lines {
        let idx = |i: usize| if rows { l * w + i } else { i * w + l };
        for (i, v) in line.iter_mut().enumerate() {
            *v = a[idx(i)];
        }
        let at = |i: isize| line[i.clamp(0, n as isize - 1) as usize];
        let mut acc = [0f32; 4];
        for i in -r..=r {
            let v = at(i);
            for k in 0..4 {
                acc[k] += v[k];
            }
        }
        for i in 0..n as isize {
            a[idx(i as usize)] = acc.map(|v| v * inv);
            let (add, sub) = (at(i + r + 1), at(i - r));
            for k in 0..4 {
                acc[k] += add[k] - sub[k];
            }
        }
    }
}
