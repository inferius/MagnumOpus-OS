//! Mix streamu: soucet v i64 se zesilenim kazdeho streamu, pak hlavni
//! hlasitost a orezani do 16 bitu zarizeni.
//!
//! Soucet se orezava az na konci (ne po kazdem streamu), takze dva
//! hlasite streamy, ktere se navzajem ruseji, nezkresli.

use crate::convert::q23_to_s16;
use crate::Frame;

/// Akumulator jedne periody (stereo).
pub struct Mixer {
    acc: alloc::vec::Vec<[i64; 2]>,
}

impl Mixer {
    pub fn new(frames: usize) -> Self {
        Self { acc: alloc::vec![[0; 2]; frames] }
    }

    pub fn frames(&self) -> usize {
        self.acc.len()
    }

    pub fn clear(&mut self) {
        self.acc.iter_mut().for_each(|a| *a = [0, 0]);
    }

    /// Pricte snimky od zacatku periody se zesilenim `gain` (Q16).
    pub fn add(&mut self, frames: &[Frame], gain: u32) {
        let g = gain as i64;
        for (a, f) in self.acc.iter_mut().zip(frames) {
            a[0] += f[0] as i64 * g >> 16;
            a[1] += f[1] as i64 * g >> 16;
        }
    }

    /// Vysledek jako s16 stereo LE do `out` (4 bajty na snimek) s hlavni
    /// hlasitosti `gain` (Q16). Vrati pocet orezanych vzorku (pro
    /// statistiku: hodne orezu = prilis hlasite).
    pub fn finish_s16(&self, gain: u32, out: &mut [u8]) -> u32 {
        let g = gain as i64;
        let mut clipped = 0;
        for (a, o) in self.acc.iter().zip(out.chunks_exact_mut(4)) {
            for c in 0..2 {
                let v = a[c] * g >> 16;
                let s = q23_to_s16(v);
                if (v >> 8) != s as i64 {
                    clipped += 1;
                }
                o[c * 2..c * 2 + 2].copy_from_slice(&s.to_le_bytes());
            }
        }
        clipped
    }
}
