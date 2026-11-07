//! Omezeni rychlosti (token bucket).

use crate::Rate;

/// Kos s tokeny: plni se rychlosti `per_sec`, pojme nejvic `burst`.
/// Tokeny v tisicinach (plynule plneni i pri malem `per_sec`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Bucket {
    rate: Rate,
    milli: u64,
    /// Posledni doplneni (ns).
    pub last: u64,
}

impl Bucket {
    pub fn new(rate: Rate, now: u64) -> Self {
        Self {
            rate,
            milli: rate.burst as u64 * 1000,
            last: now,
        }
    }

    /// Vezme token; `false` = limit prekrocen.
    pub fn take(&mut self, now: u64) -> bool {
        let elapsed = now.saturating_sub(self.last);
        let add = (elapsed as u128 * self.rate.per_sec as u128 / 1_000_000) as u64;
        let max = self.rate.burst as u64 * 1000;
        if add > 0 {
            self.milli = self.milli.saturating_add(add).min(max);
            self.last = now;
        }
        if self.milli >= 1000 {
            self.milli -= 1000;
            true
        } else {
            false
        }
    }
}
