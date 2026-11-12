//! Celociselne kresleni znaku v klidove poloze (meritko 1, bez posunu).
//!
//! Programy na x86_64 bezi bez SSE (plovouci carka v softwaru): pocitat
//! vzdalenost, uhel a prolinani ve float pro kazdy pixel znaku by stalo
//! desitky ms na snimek. Geometrie znaku se ale v klidu nemeni - meni se
//! jen zare (dychani) a prstenec. Tabulka proto drzi pro kazdy pixel
//! vzdalenost od stredu, uhel, pokryti prstence a loga; snimek pak
//! pocita jen celociselne `a - b * d` a prolinani.

use alloc::vec::Vec;

use crate::art::Mask;
use crate::geom::{self, Rect};
use crate::{Layout, size};

/// Tabulka znaku pro obdelnik `rect` (radky po `rect.w`).
pub struct MarkCache {
    pub rect: Rect,
    /// Vzdalenost od stredu v 1/16 px.
    d16: Vec<u16>,
    /// Uhel od vrcholu po smeru hodin (0..65536 = 0..2pi), jen kde je prstenec.
    ang: Vec<u16>,
    /// Pokryti prstence a loga 0..=255.
    ring: Vec<u8>,
    logo: Vec<u8>,
}

/// Hodnoty jednoho snimku (spocitane jednou, float).
pub struct FrameInt {
    /// Zare: alfa (x 256) = `(glow_a - glow_q * d16) >> 16`.
    glow_a: i64,
    glow_q: i64,
    /// Konec oblouku (0..=65536), 0 = prazdny, `full` = cely kruh.
    end: u32,
    full: bool,
    accent: u32,
}

impl MarkCache {
    pub fn build(l: &Layout, logo: &Mask, logo_x0: f32, logo_y0: f32) -> Self {
        let rect = l.mark_rect(1.0, 0.0, 0.0);
        let n = (rect.w * rect.h) as usize;
        let (mut d16, mut ang, mut ring, mut lg) = (Vec::with_capacity(n), Vec::with_capacity(n), Vec::with_capacity(n), Vec::with_capacity(n));
        let ring_r = size::RING_R * l.s;
        let hw = size::RING_W * l.s / 2.0;
        for y in rect.y..rect.bottom() {
            for x in rect.x..rect.right() {
                let (px, py) = (x as f32 + 0.5 - l.cx, y as f32 + 0.5 - l.cy);
                let d = geom::sqrt(px * px + py * py);
                d16.push((d * 16.0).min(65535.0) as u16);
                let r = (hw + 0.5 - (d - ring_r).abs()).clamp(0.0, 1.0);
                ring.push((r * 255.0 + 0.5) as u8);
                ang.push(if r > 0.0 { (geom::clockwise_angle(px, py) / core::f32::consts::TAU * 65536.0).min(65535.0) as u16 } else { 0 });
                lg.push(logo.get((x as f32 - logo_x0) as i32, (y as f32 - logo_y0) as i32));
            }
        }
        Self { rect, d16, ang, ring, logo: lg }
    }

    pub fn frame(l: &Layout, glow: f32, glow_scale: f32, progress: f32, accent: u32) -> FrameInt {
        let gr16 = size::GLOW_R * l.s * glow_scale * 16.0;
        let a = (size::GLOW_ALPHA * glow * 256.0 * 65536.0) as i64;
        let q = if gr16 > 0.0 { (size::GLOW_ALPHA * glow * 256.0 * 65536.0 / gr16) as i64 } else { 0 };
        let p = progress.clamp(0.0, 1.0);
        FrameInt { glow_a: a, glow_q: q, end: (p * 65536.0) as u32, full: p >= 1.0, accent }
    }

    /// Pixel `(x, y)` (uvnitr `rect`) nad podkladem `c`.
    pub fn shade(&self, x: i32, y: i32, c: u32, f: &FrameInt) -> u32 {
        let j = ((y - self.rect.y) as u32 * self.rect.w + (x - self.rect.x) as u32) as usize;
        let mut c = c;
        if f.glow_a > 0 {
            let a = (f.glow_a - f.glow_q * self.d16[j] as i64) >> 16;
            if a > 0 {
                c = geom::lerp(c, f.accent, a.min(256) as u32);
            }
        }
        let r = self.ring[j] as u32;
        if r > 0 {
            // Podklad prstence: akcent 22 %.
            c = geom::lerp(c, f.accent, (r * 256 * 22 / 100 + 127) / 255);
            if f.full || (f.end > 0 && (self.ang[j] as u32) <= f.end) {
                c = geom::lerp(c, f.accent, (r * 256 + 127) / 255);
            }
        }
        let lc = self.logo[j] as u32;
        if lc > 0 {
            c = geom::lerp(c, f.accent, (lc * 256 + 127) / 255);
        }
        c
    }
}
