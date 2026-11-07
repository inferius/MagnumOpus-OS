//! Rasterizer s plosnym vyhlazenim: kazda usecka pricte do bunek
//! akumulacniho pole svuj podil plochy se znamenkem (smer), prubezny
//! soucet radku pak dava pokryti pixelu (pravidlo nenuloveho vinuti,
//! pokryti nad 1 se orizne). Krivky se rozdeli na usecky podle odchylky.
//!
//! Stejny princip pouziva font-rs (Raph Levien); zde je vlastni
//! implementace, ktera nikdy nesaha mimo pole (cteni a zapis pres `get`).

use alloc::vec;
use alloc::vec::Vec;

use crate::math::{ceil, floor, sqrt};
use crate::{Glyph, Point, Segment};

/// Nejvetsi rozmer masky glyfu (ochrana pameti).
const MAX_DIM: u32 = 2048;

/// Akumulacni pole pokryti.
pub struct Rasterizer {
    w: usize,
    h: usize,
    acc: Vec<f32>,
}

impl Rasterizer {
    pub fn new(w: u32, h: u32) -> Self {
        let (w, h) = (w.min(MAX_DIM) as usize, h.min(MAX_DIM) as usize);
        Self {
            w,
            h,
            acc: vec![0.0; w * h + 4],
        }
    }

    fn add(&mut self, i: isize, v: f32) {
        if i >= 0
            && let Some(a) = self.acc.get_mut(i as usize)
        {
            *a += v;
        }
    }

    /// Usecka v pixelech masky (y dolu).
    pub fn line(&mut self, p0: Point, p1: Point) {
        if (p0.y - p1.y).abs() <= 1e-6 || !p0.x.is_finite() || !p1.x.is_finite() {
            return;
        }
        let (dir, p0, p1) = if p0.y < p1.y { (1.0f32, p0, p1) } else { (-1.0, p1, p0) };
        let maxx = self.w as f32 - 0.001;
        let clampx = |x: f32| x.clamp(0.0, maxx);
        let dxdy = (p1.x - p0.x) / (p1.y - p0.y);
        let mut x = p0.x;
        let y0 = if p0.y < 0.0 {
            x -= p0.y * dxdy;
            0usize
        } else {
            p0.y as usize
        };
        let yend = (ceil(p1.y).max(0.0) as usize).min(self.h);
        for y in y0..yend {
            let line = (y * self.w) as isize;
            let dy = ((y + 1) as f32).min(p1.y) - (y as f32).max(p0.y);
            let xnext = x + dxdy * dy;
            let d = dy * dir;
            let (xa, xb) = if x < xnext { (clampx(x), clampx(xnext)) } else { (clampx(xnext), clampx(x)) };
            let x0f = floor(xa);
            let x0i = x0f as isize;
            let x1c = ceil(xb);
            let x1i = x1c as isize;
            if x1i <= x0i + 1 {
                let xm = 0.5 * (xa + xb) - x0f;
                self.add(line + x0i, d - d * xm);
                self.add(line + x0i + 1, d * xm);
            } else {
                let s = 1.0 / (xb - xa);
                let fx0 = xa - x0f;
                let a0 = 0.5 * s * (1.0 - fx0) * (1.0 - fx0);
                let fx1 = xb - x1c + 1.0;
                let am = 0.5 * s * fx1 * fx1;
                self.add(line + x0i, d * a0);
                if x1i == x0i + 2 {
                    self.add(line + x0i + 1, d * (1.0 - a0 - am));
                } else {
                    let a1 = s * (1.5 - fx0);
                    self.add(line + x0i + 1, d * (a1 - a0));
                    for xi in x0i + 2..x1i - 1 {
                        self.add(line + xi, d * s);
                    }
                    let a2 = a1 + (x1i - x0i - 3) as f32 * s;
                    self.add(line + x1i - 1, d * (1.0 - a2 - am));
                }
                self.add(line + x1i, d * am);
            }
            x = xnext;
        }
    }

    /// Kvadraticka krivka rozdelena na usecky.
    pub fn quad(&mut self, p0: Point, p1: Point, p2: Point) {
        let ddx = p0.x - 2.0 * p1.x + p2.x;
        let ddy = p0.y - 2.0 * p1.y + p2.y;
        let dev = ddx * ddx + ddy * ddy;
        if dev < 0.333 {
            return self.line(p0, p2);
        }
        let n = (1.0 + floor(sqrt(sqrt(3.0 * dev)))).min(64.0) as u32;
        let mut prev = p0;
        for i in 1..=n {
            let t = i as f32 / n as f32;
            let mt = 1.0 - t;
            let p = Point {
                x: mt * mt * p0.x + 2.0 * mt * t * p1.x + t * t * p2.x,
                y: mt * mt * p0.y + 2.0 * mt * t * p1.y + t * t * p2.y,
            };
            self.line(prev, p);
            prev = p;
        }
    }

    /// Prubezny soucet -> maska pokryti 0..=255.
    pub fn finish(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.w * self.h);
        let mut acc = 0.0f32;
        for i in 0..self.w * self.h {
            acc += self.acc[i];
            let v = acc.abs().min(1.0);
            out.push((v * 255.0 + 0.5) as u8);
        }
        out
    }
}

/// Vykresli obrys (jednotky pisma, y nahoru) ve skale `s` (px na jednotku).
pub fn render(segs: &[Segment], s: f32) -> Glyph {
    let mut minx = f32::MAX;
    let mut miny = f32::MAX;
    let mut maxx = f32::MIN;
    let mut maxy = f32::MIN;
    let mut each = |p: &Point| {
        minx = minx.min(p.x);
        maxx = maxx.max(p.x);
        miny = miny.min(p.y);
        maxy = maxy.max(p.y);
    };
    for seg in segs {
        match seg {
            Segment::Line(a, b) => {
                each(a);
                each(b);
            }
            Segment::Quad(a, b, c) => {
                each(a);
                each(b);
                each(c);
            }
        }
    }
    if segs.is_empty() || !(minx <= maxx) || !(maxx - minx).is_finite() || !(maxy - miny).is_finite() {
        return Glyph::default();
    }
    // Obdelnik masky v pixelech (y dolu): x0..x1, y0..y1.
    let x0 = floor(minx * s);
    let x1 = ceil(maxx * s);
    let y0 = floor(-maxy * s);
    let y1 = ceil(-miny * s);
    let w = ((x1 - x0) as u32 + 1).min(MAX_DIM);
    let h = ((y1 - y0) as u32).clamp(1, MAX_DIM);
    let mut r = Rasterizer::new(w, h);
    let tr = |p: &Point| Point {
        x: p.x * s - x0,
        y: -p.y * s - y0,
    };
    for seg in segs {
        match seg {
            Segment::Line(a, b) => r.line(tr(a), tr(b)),
            Segment::Quad(a, b, c) => r.quad(tr(a), tr(b), tr(c)),
        }
    }
    Glyph {
        w: r.w as u32,
        h: r.h as u32,
        left: x0 as i32,
        top: y0 as i32,
        advance: 0.0,
        coverage: r.finish(),
    }
}
