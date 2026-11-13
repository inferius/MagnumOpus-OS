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
    let b = [255.0f32, (base >> 16 & 0xFF) as f32, (base >> 8 & 0xFF) as f32, (base & 0xFF) as f32];
    // Presna barva (premultiplied, float) v bode.
    let exact = |x: f32, y: f32| -> [f32; 4] {
        let mut p = b;
        for &(cx, cy, len, col) in geo.iter().rev() {
            let (dx, dy) = (x - cx, y - cy);
            let t = sqrt2(dx * dx + dy * dy) / len;
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
    // Body mrizky presne (float), pak uz jen cela cisla: programy bez FPU
    // (x86_64 bez SSE) by float interpolaci kazdeho pixelu pocitaly
    // stovky ms az sekundy. Kanal v mrizce je v 1/256 urovne.
    let mut grid: Vec<[i32; 3]> = Vec::with_capacity((gw * gh) as usize);
    for gy in 0..gh {
        for gx in 0..gw {
            let p = exact((gx * STEP) as f32 + 0.5, (gy * STEP) as f32 + 0.5);
            grid.push([(p[1] * 256.0 + 0.5) as i32, (p[2] * 256.0 + 0.5) as i32, (p[3] * 256.0 + 0.5) as i32]);
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

/// Odmocnina pro body mrizky: odhad z exponentu a dva kroky Newtonovy
/// metody (relativni chyba pod 1e-6; `crate::sqrtf` dela ctyri kroky,
/// v programech bez FPU je kazde deleni drahe).
fn sqrt2(x: f32) -> f32 {
    if !(x > 0.0) || !x.is_finite() {
        return 0.0;
    }
    let mut y = f32::from_bits((x.to_bits() >> 1) + 0x1FBB_4F2E);
    y = 0.5 * (y + x / y);
    0.5 * (y + x / y)
}
