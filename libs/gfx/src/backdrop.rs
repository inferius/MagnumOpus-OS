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

/// Bayer 4x4 jako rozptyl v 1/4096 urovne: `(B + 0,5) / 16` urovne.
const BAYER_I: [[i32; 4]; 4] = {
    const B: [[i32; 4]; 4] = [[0, 8, 2, 10], [12, 4, 14, 6], [3, 11, 1, 9], [15, 7, 13, 5]];
    let mut o = [[0i32; 4]; 4];
    let mut y = 0;
    while y < 4 {
        let mut x = 0;
        while x < 4 {
            o[y][x] = (B[y][x] * 2 + 1) * 128;
            x += 1;
        }
        y += 1;
    }
    o
};

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
    let (gw, gh) = (w / STEP + 2, h / STEP + 2);
    // Body mrizky celociselne (programy bez FPU by `f32` v kazdem bode
    // pocitaly stovky ms): souradnice v 1/16 px, `t` v 1/65536, kanal
    // v 1/256 urovne. Proti vypoctu v `f32` nejvys o jednu uroven
    // (test `backdrop_int_matches_float`).
    let geo16: Vec<(i64, i64, u64, [i64; 4])> = geo
        .iter()
        .map(|&(cx, cy, len, ch)| ((cx * 16.0 + 0.5) as i64, (cy * 16.0 + 0.5) as i64, ((len * 16.0 + 0.5) as u64).max(1), ch.map(|v| v as i64)))
        .collect();
    let base_ch = [(base >> 16 & 0xFF) as i64 * 256, (base >> 8 & 0xFF) as i64 * 256, (base & 0xFF) as i64 * 256];
    let mut grid: Vec<[i32; 3]> = Vec::with_capacity((gw * gh) as usize);
    for gy in 0..gh {
        for gx in 0..gw {
            // Stred pixelu (`* STEP + 0,5`) v 1/16 px.
            let (x16, y16) = ((gx * STEP) as i64 * 16 + 8, (gy * STEP) as i64 * 16 + 8);
            let mut p = base_ch;
            for &(cx, cy, len, col) in geo16.iter().rev() {
                let (dx, dy) = (x16 - cx, y16 - cy);
                let d = isqrt((dx * dx + dy * dy) as u64);
                if d >= len {
                    continue;
                }
                // k = 1 - d / len (1/65536).
                let k = 65536 - (d * 65536 / len) as i64;
                // src-over: s*k + d*(1 - a*k)
                let inv = 65536 - col[0] * k / 255;
                for i in 0..3 {
                    p[i] = (col[i + 1] * 256 * k + p[i] * inv + 32768) >> 16;
                }
            }
            grid.push([p[0] as i32, p[1] as i32, p[2] as i32]);
        }
    }
    // Bilinearni vahy (soucet STEP * STEP = 16) a rozptyl ve stejnych
    // jednotkach (1/4096 urovne): (B + 0,5) / 16 urovne.
    const S: i32 = STEP as i32;
    let mut out = Vec::with_capacity((w as usize) * (h as usize));
    for y in 0..h {
        let (gy, fy) = (y / STEP, (y % STEP) as i32);
        let row = (gy * gw) as usize;
        let brow = &BAYER_I[(y & 3) as usize];
        for x in 0..w {
            let (gx, fx) = (x / STEP, (x % STEP) as i32);
            let i = row + gx as usize;
            let (a, bb, c, d) = (&grid[i], &grid[i + 1], &grid[i + gw as usize], &grid[i + gw as usize + 1]);
            let (wa, wb, wc, wd) = ((S - fx) * (S - fy), fx * (S - fy), (S - fx) * fy, fx * fy);
            let dither = brow[(x & 3) as usize];
            let mut px = 0xFF00_0000u32;
            for (k, sh) in [(0usize, 16u32), (1, 8), (2, 0)] {
                let v = (a[k] * wa + bb[k] * wb + c[k] * wc + d[k] * wd + dither) >> 12;
                px |= (v.clamp(0, 255) as u32) << sh;
            }
            out.push(px);
        }
    }
    out
}

/// Cela cast odmocniny (bit po bitu, bez FPU).
fn isqrt(n: u64) -> u64 {
    let mut x = n;
    let mut r = 0u64;
    let mut bit = 1u64 << 62;
    while bit > n {
        bit >>= 2;
    }
    while bit != 0 {
        if x >= r + bit {
            x -= r + bit;
            r = (r >> 1) + bit;
        } else {
            r >>= 1;
        }
        bit >>= 2;
    }
    r
}
