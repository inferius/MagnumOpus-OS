//! Prevod vzorkovaci frekvence (sample rate conversion).
//!
//! Kubicka interpolace Hermite (Catmull-Rom) ze 4 sousednich vzorku,
//! faze v pevne radove carce Q32. Pri stejne frekvenci je vystup presne
//! vstup (posunuty o jeden snimek). Pri snizovani frekvence (napr.
//! 96 -> 48 kHz) jde vstup nejdriv pres dolni propust
//! ([`crate::filter::Lowpass`]), jinak by se obsah nad novou Nyquistovou
//! frekvenci slozil zpet (aliasing).

use crate::filter::Lowpass;
use crate::{Frame, Q23_MAX, Q23_MIN};

const ONE: u64 = 1 << 32;

pub struct Resampler {
    /// Krok vstupu na jeden vystupni snimek (Q32).
    step: u64,
    /// Faze: kolik vstupu jeste "dluzime" pred dalsim vystupem (Q32).
    acc: u64,
    /// Posledni ctyri vstupni snimky (y0..y3); vystup lezi mezi y1 a y2.
    hist: [Frame; 4],
    /// Dolni propust pri snizovani frekvence.
    lowpass: Option<alloc::boxed::Box<Lowpass>>,
}

impl Resampler {
    pub fn new(in_rate: u32, out_rate: u32) -> Self {
        // Prvni vystup az po trech vstupech: y1 = prvni skutecny vzorek.
        let lowpass = (in_rate > out_rate).then(|| alloc::boxed::Box::new(Lowpass::new(in_rate, out_rate)));
        Self { step: ((in_rate as u64) << 32) / out_rate as u64, acc: 3 * ONE, hist: [[0; 2]; 4], lowpass }
    }

    /// Kolik vstupnich snimku ticha je potreba, aby vysel posledni
    /// skutecny vstup (historie interpolace a zpozdeni filtru).
    pub fn flush_frames(&self) -> u32 {
        2 + if self.lowpass.is_some() { Lowpass::delay() as u32 } else { 0 }
    }

    /// Kolik vstupnich snimku je potreba na `out` vystupnich (horni odhad).
    pub fn input_needed(&self, out: u32) -> u32 {
        if out == 0 {
            return 0;
        }
        let total = self.acc + (out as u64 - 1) * self.step;
        (total >> 32) as u32 + 1
    }

    /// Vyrobi az `out.len()` snimku; vstup bere z `input` (None = vstup
    /// dosel). Vrati pocet vyrobenych snimku. Stav zustane platny i pri
    /// nedostatku vstupu (pokracuje se pri dalsim volani).
    pub fn process(&mut self, out: &mut [Frame], mut input: impl FnMut() -> Option<Frame>) -> usize {
        for (i, o) in out.iter_mut().enumerate() {
            while self.acc >= ONE {
                let Some(mut f) = input() else { return i };
                if let Some(lp) = self.lowpass.as_mut() {
                    f = lp.push(f);
                }
                self.hist = [self.hist[1], self.hist[2], self.hist[3], f];
                self.acc -= ONE;
            }
            let t = (self.acc >> 16) as i64; // Q16
            *o = [interp(&self.hist, 0, t), interp(&self.hist, 1, t)];
            self.acc += self.step;
        }
        out.len()
    }

    /// Zapomene historii (po stop/start): dalsi vystup zacne cistym vstupem.
    pub fn reset(&mut self) {
        self.acc = 3 * ONE;
        self.hist = [[0; 2]; 4];
        if let Some(lp) = self.lowpass.as_mut() {
            lp.reset();
        }
    }
}

/// Catmull-Rom mezi y1 a y2, `t` v Q16 (0..65536).
#[inline]
fn interp(h: &[Frame; 4], c: usize, t: i64) -> i32 {
    let (y0, y1, y2, y3) = (h[0][c] as i64, h[1][c] as i64, h[2][c] as i64, h[3][c] as i64);
    if t == 0 {
        return y1 as i32;
    }
    // Dvojnasobne koeficienty (bez polovin): 2c1, 2c2, 2c3.
    let a1 = y2 - y0;
    let a2 = 2 * y0 - 5 * y1 + 4 * y2 - y3;
    let a3 = (y3 - y0) + 3 * (y1 - y2);
    let v = ((a3 * t >> 16) + a2) * t >> 16;
    let v = (v + a1) * t >> 17;
    (y1 + v).clamp(Q23_MIN as i64, Q23_MAX as i64) as i32
}
