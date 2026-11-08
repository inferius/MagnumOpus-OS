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

    /// Jako [`Mixer::finish_s16`], ale 24 bitu v 32bitovem slove zarovnane
    /// nahoru (format HDA pro 20 a 24 bitu, 8 bajtu na snimek).
    pub fn finish_s32(&self, gain: u32, out: &mut [u8]) -> u32 {
        let g = gain as i64;
        let mut clipped = 0;
        for (a, o) in self.acc.iter().zip(out.chunks_exact_mut(8)) {
            for c in 0..2 {
                let v = a[c] * g >> 16;
                let s = v.clamp(crate::Q23_MIN as i64, crate::Q23_MAX as i64);
                if s != v {
                    clipped += 1;
                }
                o[c * 4..c * 4 + 4].copy_from_slice(&((s as i32) << 8).to_le_bytes());
            }
        }
        clipped
    }

    /// Vysledek ve formatu zarizeni (`S16`, nebo `S32` = 24 bitu nahore).
    pub fn finish(&self, sample: audioproto::Sample, gain: u32, out: &mut [u8]) -> u32 {
        match sample {
            audioproto::Sample::S32 | audioproto::Sample::S24 => self.finish_s32(gain, out),
            _ => self.finish_s16(gain, out),
        }
    }
}
