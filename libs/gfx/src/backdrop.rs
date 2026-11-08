//! Pozadi plochy: zakladni barva a dva mekke radialni prechody (jako
//! `.desk` v prototypu: `radial-gradient(circle at 18% 22%, wall_a,
//! transparent 52%)` a `circle at 82% 72%, wall_b, transparent 56%`).
//!
//! Spocita se jednou pro velikost obrazovky do obrazu; skladani pak jen
//! kopiruje. Prechody maji jemny rozptyl (Bayer 4x4, +/- pul urovne), aby
//! v malem rozsahu barev nebyly videt pruhy.

use alloc::vec::Vec;

use crate::{over, scale};

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

/// Obraz plochy `w x h`: `base` a prechody (prvni je navrchu, jako v CSS).
pub fn render(w: u32, h: u32, base: u32, layers: &[Radial]) -> Vec<u32> {
    let mut out = Vec::with_capacity((w as usize) * (h as usize));
    let (wf, hf) = (w as f32, h as f32);
    // Pro kazdy prechod: stred v px a delka paprsku na konec.
    let geo: Vec<(f32, f32, f32, &Radial)> = layers
        .iter()
        .map(|l| {
            let (cx, cy) = (l.cx * wf, l.cy * hf);
            let fx = cx.max(wf - cx);
            let fy = cy.max(hf - cy);
            let far = crate::sqrtf(fx * fx + fy * fy);
            (cx, cy, (far * l.stop).max(1.0), l)
        })
        .collect();
    for y in 0..h {
        for x in 0..w {
            let dither = (BAYER[(y & 3) as usize][(x & 3) as usize] + 0.5) / 16.0;
            let mut p = base;
            for &(cx, cy, len, l) in geo.iter().rev() {
                let (dx, dy) = (x as f32 + 0.5 - cx, y as f32 + 0.5 - cy);
                let t = crate::sqrtf(dx * dx + dy * dy) / len;
                if t >= 1.0 {
                    continue;
                }
                // Linearne v premultiplied prostoru (jako prohlizec).
                let k = ((1.0 - t) * 255.0 + dither) as u32;
                p = over(p, scale(l.color, k.min(255)));
            }
            out.push(p | 0xFF00_0000);
        }
    }
    out
}
