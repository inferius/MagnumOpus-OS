//! Poskozene oblasti: seznam obdelniku, ktere je treba znovu slozit.
//!
//! Prekryvajici se nebo blizke obdelniky se slevaji, aby jich nebylo moc
//! (kazdy obdelnik stoji jeden pruchod skladani a jeden prenos do GPU).

use alloc::vec::Vec;

use crate::Rect;

/// Nejvic obdelniku; vic = jeden obalujici.
pub const MAX: usize = 16;

#[derive(Debug, Clone, Default)]
pub struct Damage {
    rects: Vec<Rect>,
}

fn area(r: &Rect) -> u64 {
    r.w as u64 * r.h as u64
}

impl Damage {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_empty(&self) -> bool {
        self.rects.is_empty()
    }

    pub fn rects(&self) -> &[Rect] {
        &self.rects
    }

    pub fn clear(&mut self) {
        self.rects.clear();
    }

    /// Prida obdelnik orezany na `bounds`.
    pub fn add(&mut self, r: Rect, bounds: Rect) {
        let mut r = r.intersect(&bounds);
        if r.is_empty() {
            return;
        }
        // Slevat, dokud se neco slit da (obalujici neni o moc vetsi nez
        // soucet ploch).
        loop {
            let mut merged = false;
            let mut i = 0;
            while i < self.rects.len() {
                let o = self.rects[i];
                let u = r.union(&o);
                if area(&u) <= (area(&r) + area(&o)) * 5 / 4 || !o.intersect(&r).is_empty() {
                    r = u;
                    self.rects.swap_remove(i);
                    merged = true;
                } else {
                    i += 1;
                }
            }
            if !merged {
                break;
            }
        }
        self.rects.push(r);
        if self.rects.len() > MAX {
            let all = self.bounding();
            self.rects.clear();
            self.rects.push(all);
        }
    }

    /// Obalujici obdelnik vseho.
    pub fn bounding(&self) -> Rect {
        self.rects.iter().fold(Rect::default(), |a, r| a.union(r))
    }

    pub fn take(&mut self) -> Vec<Rect> {
        core::mem::take(&mut self.rects)
    }
}
