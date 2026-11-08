//! Sinusovy ton v celych cislech (testy zvuku, prikaz `audio tone`).
//!
//! Faze je u32 (cela perioda = 2^32), sinus z Taylorovy rady 7. radu na
//! ctvrtperiode: chyba < 2e-4 (harmonicke pod -70 dB), bez tabulky
//! a bez FPU.

use crate::{Frame, Q23_MAX};

/// pi/2 v Q30.
const HALF_PI_Q30: i64 = 1_686_629_713;
const ONE_Q30: i64 = 1 << 30;

/// sin(2 pi * phase / 2^32) v Q30.
pub fn sin_q30(phase: u32) -> i64 {
    let quadrant = phase >> 30;
    let mut r = (phase & 0x3FFF_FFFF) as i64;
    if quadrant & 1 == 1 {
        r = ONE_Q30 - r;
    }
    let x = r * HALF_PI_Q30 >> 30;
    let x2 = x * x >> 30;
    // x * (1 - x2/6 * (1 - x2/20 * (1 - x2/42)))
    let a = ONE_Q30 - x2 / 42;
    let b = ONE_Q30 - (x2 * a >> 30) / 20;
    let c = ONE_Q30 - (x2 * b >> 30) / 6;
    let s = x * c >> 30;
    if quadrant >= 2 { -s } else { s }
}

/// Generator tonu dane frekvence.
pub struct Tone {
    phase: u32,
    step: u32,
    /// Amplituda Q23.
    amp: i64,
}

impl Tone {
    /// `percent` = amplituda v procentech plneho rozsahu.
    pub fn new(freq: u32, rate: u32, percent: u8) -> Self {
        let step = ((freq as u64) << 32) / rate as u64;
        Self { phase: 0, step: step as u32, amp: Q23_MAX as i64 * percent.min(100) as i64 / 100 }
    }

    pub fn next_frame(&mut self) -> Frame {
        let v = (sin_q30(self.phase) * self.amp >> 30) as i32;
        self.phase = self.phase.wrapping_add(self.step);
        [v, v]
    }

    /// Dalsich `frames` snimku jako s16 stereo LE.
    pub fn fill_s16(&mut self, out: &mut [u8]) {
        for o in out.chunks_exact_mut(4) {
            let [l, r] = self.next_frame();
            o[0..2].copy_from_slice(&((l >> 8) as i16).to_le_bytes());
            o[2..4].copy_from_slice(&((r >> 8) as i16).to_le_bytes());
        }
    }
}
