//! Pozadi plochy: zakladni barva a dva mekke radialni prechody (jako
//! `.desk` v prototypu: `radial-gradient(circle at 18% 22%, wall_a,
//! transparent 52%)` a `circle at 82% 72%, wall_b, transparent 56%`).
//!
//! Spocita se jednou pro velikost obrazovky do obrazu; skladani pak jen
//! kopiruje. Prechody maji jemny rozptyl (Bayer 4x4, +/- pul urovne), aby
//! v malem rozsahu barev nebyly videt pruhy.

use alloc::vec::Vec;


/// Radialni prechod: stred (podil sirky a vysky), konec (podil delky
/// paprsku k nejvzdalenejsimu rohu, CSS `farthest-corner`), barva
/// (premultiplied).
#[derive(Debug, Clone, Copy)]
pub struct Radial {
    pub cx: f32,
    pub cy: f32,
    pub stop: f32,
    pub color: u32,
}

const BAYER: [[f32; 4]; 4] = [[0.0, 8.0, 2.0, 10.0], [12.0, 4.0, 14.0, 6.0], [3.0, 11.0, 1.0, 9.0], [15.0, 7.0, 13.0, 5.0]];

/// Krok mrizky, ve ktere se prechody pocitaji presne (mezi body linearne;
/// prechody jsou mekke, rozdil je pod jednu uroven barvy).
const STEP: u32 = 4;

/// Obraz plochy `w x h`: `base` a prechody (prvni je navrchu, jako v CSS).
pub fn render(w: u32, h: u32, base: u32, layers: &[Radial]) -> Vec<u32> {
    let (wf, hf) = (w as f32, h as f32);
    // Pro kazdy prechod: stred v px a delka paprsku na konec.
    let geo: Vec<(f32, f32, f32, [f32; 4])> = layers
        .iter()
        .map(|l| {
            let (cx, cy) = (l.cx * wf, l.cy * hf);
            let fx = cx.max(wf - cx);
            let fy = cy.max(hf - cy);
            let far = crate::sqrtf(fx * fx + fy * fy);
            let c = l.color;
            let ch = [(c >> 24 & 0xFF) as f32, (c >> 16 & 0xFF) as f32, (c >> 8 & 0xFF) as f32, (c & 0xFF) as f32];
            (cx, cy, (far * l.stop).max(1.0), ch)
        })
        .collect();
    let b = [255.0f32, (base >> 16 & 0xFF) as f32, (base >> 8 & 0xFF) as f32, (base & 0xFF) as f32];
    // Presna barva (premultiplied, float) v bode.
    let exact = |x: f32, y: f32| -> [f32; 4] {
        let mut p = b;
        for &(cx, cy, len, col) in geo.iter().rev() {
            let (dx, dy) = (x - cx, y - cy);
            let t = crate::sqrtf(dx * dx + dy * dy) / len;
            if t >= 1.0 {
                continue;
            }
            let k = 1.0 - t;
            // src-over: s*k + d*(1 - a*k)
            let inv = 1.0 - col[0] / 255.0 * k;
            for i in 0..4 {
                p[i] = col[i] * k + p[i] * inv;
            }
        }
        p
    };
    let (gw, gh) = (w / STEP + 2, h / STEP + 2);
    let mut grid = Vec::with_capacity((gw * gh) as usize);
    for gy in 0..gh {
        for gx in 0..gw {
            grid.push(exact((gx * STEP) as f32 + 0.5, (gy * STEP) as f32 + 0.5));
        }
    }
    let mut out = Vec::with_capacity((w as usize) * (h as usize));
    for y in 0..h {
        let (gy, fy) = (y / STEP, (y % STEP) as f32 / STEP as f32);
        for x in 0..w {
            let (gx, fx) = (x / STEP, (x % STEP) as f32 / STEP as f32);
            let i = (gy * gw + gx) as usize;
            let (a, bb, c, d) = (grid[i], grid[i + 1], grid[i + gw as usize], grid[i + gw as usize + 1]);
            let dither = (BAYER[(y & 3) as usize][(x & 3) as usize] + 0.5) / 16.0;
            let mut px = 0xFF00_0000u32;
            for (k, sh) in [(1usize, 16u32), (2, 8), (3, 0)] {
                let top = a[k] + (bb[k] - a[k]) * fx;
                let bot = c[k] + (d[k] - c[k]) * fx;
                let v = (top + (bot - top) * fy + dither).clamp(0.0, 255.0) as u32;
                px |= v << sh;
            }
            out.push(px);
        }
    }
    out
}
