//! Antialiasingovy filtr pred snizenim vzorkovaci frekvence.
//!
//! FIR dolni propust (okenkovany sinc, Hannovo okno, [`TAPS`] koeficientu)
//! s meznim kmitoctem 0,45 x nova frekvence: obsah nad novou Nyquistovou
//! frekvenci se potlaci drive, nez ho interpolace slozi zpet. Koeficienty
//! se spocitaji jednou pri otevreni streamu (pevna radova carka Q15,
//! soucet = 1); filtrace je v celych cislech. Zpozdeni je
//! (`TAPS` - 1) / 2 vstupnich snimku.

use crate::tone::sin_q30;
use crate::{Frame, Q23_MAX, Q23_MIN};

/// Pocet koeficientu (lichy: symetricky filtr se stredem).
pub const TAPS: usize = 47;
const Q: i64 = 1 << 15;

/// sin(pi * x) pro x v radiane/pi (pres celociselny sinus).
fn sin_pi(x: f64) -> f64 {
    // Faze u32: cela perioda (2 pi) = 2^32, tedy pi * x = x / 2 periody.
    let half = x / 2.0;
    let frac = half - (half as i64) as f64;
    let frac = if frac < 0.0 { frac + 1.0 } else { frac };
    sin_q30((frac * 4_294_967_296.0) as u64 as u32) as f64 / (1u64 << 30) as f64
}

/// cos(2 pi x).
fn cos_2pi(x: f64) -> f64 {
    sin_pi(2.0 * x + 0.5)
}

pub struct Lowpass {
    coef: [i32; TAPS],
    hist: [Frame; TAPS],
    pos: usize,
}

impl Lowpass {
    /// Filtr pro prevod `in_rate` -> `out_rate` (`out_rate` < `in_rate`).
    pub fn new(in_rate: u32, out_rate: u32) -> Self {
        let fc = 0.45 * out_rate as f64 / in_rate as f64; // v nasobcich vstupni frekvence
        let mid = (TAPS / 2) as f64;
        let mut raw = [0f64; TAPS];
        let mut sum = 0.0;
        for (i, r) in raw.iter_mut().enumerate() {
            let t = i as f64 - mid;
            let sinc = if t == 0.0 { 2.0 * fc } else { sin_pi(2.0 * fc * t) / (core::f64::consts::PI * t) };
            let w = 0.5 - 0.5 * cos_2pi(i as f64 / (TAPS - 1) as f64);
            *r = sinc * w;
            sum += *r;
        }
        let mut coef = [0i32; TAPS];
        for (c, r) in coef.iter_mut().zip(raw) {
            *c = (r / sum * Q as f64) as i32;
        }
        Self { coef, hist: [[0; 2]; TAPS], pos: 0 }
    }

    /// Prida vstupni snimek, vrati vyfiltrovany.
    pub fn push(&mut self, f: Frame) -> Frame {
        self.hist[self.pos] = f;
        self.pos = (self.pos + 1) % TAPS;
        let (mut l, mut r) = (0i64, 0i64);
        for (k, &c) in self.coef.iter().enumerate() {
            let h = self.hist[(self.pos + k) % TAPS];
            l += h[0] as i64 * c as i64;
            r += h[1] as i64 * c as i64;
        }
        let clamp = |v: i64| (v / Q).clamp(Q23_MIN as i64, Q23_MAX as i64) as i32;
        [clamp(l), clamp(r)]
    }

    pub fn reset(&mut self) {
        self.hist = [[0; 2]; TAPS];
        self.pos = 0;
    }

    /// Zpozdeni filtru (vstupni snimky).
    pub const fn delay() -> usize {
        TAPS / 2
    }
}
