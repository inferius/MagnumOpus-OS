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
///
/// Celociselne (programy v OS nemaji FPU, `f32` je volani funkce na
/// kazdou operaci): kanaly v pevne radove carce 8.8, klouzave soucty
/// v `u32`. Proti vypoctu v `f32` se lisi nejvys o jednu uroven barvy
/// (test `blur_int_matches_float`).
pub fn blurred_region(src: &[u32], stride: u32, area: Rect, sigma: f32) -> Vec<u32> {
    let (w, h) = (area.w as usize, area.h as usize);
    if w == 0 || h == 0 {
        return Vec::new();
    }
    let (hw, hh) = (w.div_ceil(2), h.div_ceil(2));
    // Polovicni rozliseni: prumer 2x2, ctyri kanaly v 8.8.
    let mut ch = vec![[0u32; 4]; hw * hh];
    for y in 0..hh {
        for x in 0..hw {
            let mut acc = [0u32; 4];
            let mut n = 0u32;
            for dy in 0..2 {
                for dx in 0..2 {
                    let (sx, sy) = (2 * x + dx, 2 * y + dy);
                    if sx >= w || sy >= h {
                        continue;
                    }
                    let p = src[(area.y as usize + sy) * stride as usize + area.x as usize + sx];
                    for (k, a) in acc.iter_mut().enumerate() {
                        *a += p >> (24 - 8 * k) & 0xFF;
                    }
                    n += 1;
                }
            }
            ch[y * hw + x] = acc.map(|v| (v * 256 + n / 2) / n);
        }
    }
    for bw in box_sizes((sigma / 2.0).max(0.5)) {
        box_pass(&mut ch, hw, hh, bw, true);
        box_pass(&mut ch, hw, hh, bw, false);
    }
    // Zpet do plneho rozliseni (bilinearne). Poloha ve zdroji je
    // `(x - 0,5) / 2`, tedy ve ctvrtinach `2x - 1`.
    let pos = |i: usize, n: usize| -> (usize, usize, u32) {
        let q = 2 * i as i64 - 1;
        let i0 = ((q.max(0) / 4) as usize).min(n - 1);
        let i1 = (i0 + 1).min(n - 1);
        let t = (q - 4 * i0 as i64).clamp(0, 4) as u32;
        (i0, i1, t)
    };
    let cols: Vec<(usize, usize, u32)> = (0..w).map(|x| pos(x, hw)).collect();
    let mut out = Vec::with_capacity(w * h);
    for y in 0..h {
        let (y0, y1, ty) = pos(y, hh);
        for &(x0, x1, tx) in &cols {
            let (a, b, c, d) = (ch[y0 * hw + x0], ch[y0 * hw + x1], ch[y1 * hw + x0], ch[y1 * hw + x1]);
            let mut px = 0u32;
            for k in 0..4 {
                let top = a[k] * (4 - tx) + b[k] * tx;
                let bot = c[k] * (4 - tx) + d[k] * tx;
                // 8.8 * 16 -> zaokrouhlit na celou uroven.
                let v = ((top * (4 - ty) + bot * ty + 2048) >> 12).min(255);
                px |= v << (24 - 8 * k);
            }
            out.push(px);
        }
    }
    out
}

/// Krabickovy filtr sirky `bw` po radcich (`rows`) nebo sloupcich, okraje
/// prodlouzene.
fn box_pass(a: &mut [[u32; 4]], w: usize, h: usize, bw: u32, rows: bool) {
    let r = (bw / 2) as isize;
    let (n, lines) = if rows { (w, h) } else { (h, w) };
    let mut line = vec![[0u32; 4]; n];
    for l in 0..lines {
        let idx = |i: usize| if rows { l * w + i } else { i * w + l };
        for (i, v) in line.iter_mut().enumerate() {
            *v = a[idx(i)];
        }
        let at = |i: isize| line[i.clamp(0, n as isize - 1) as usize];
        let mut acc = [0u32; 4];
        for i in -r..=r {
            let v = at(i);
            for k in 0..4 {
                acc[k] += v[k];
            }
        }
        for i in 0..n as isize {
            a[idx(i as usize)] = acc.map(|v| (v + bw / 2) / bw);
            let (add, sub) = (at(i + r + 1), at(i - r));
            for k in 0..4 {
                acc[k] = acc[k] + add[k] - sub[k];
            }
        }
    }
}
