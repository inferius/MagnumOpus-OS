//! Prubeh v realnem case: rychlost a odhad zbyvajiciho casu si okno
//! pocita samo z casu aktualizaci (sluzba posila jen `done` a `total`).

use alloc::string::String;
use alloc::vec::Vec;

/// Kolik vzorku drzet (klouzave okno pro rychlost).
const WINDOW: usize = 16;

#[derive(Debug, Clone, Default)]
pub struct Track {
    /// (cas v ns, hotovo).
    samples: Vec<(i64, u64)>,
}

impl Track {
    pub fn push(&mut self, now: i64, done: u64) {
        if self.samples.last().is_some_and(|(_, d)| *d > done) {
            // Novy beh (mensi hodnota): zacit znovu.
            self.samples.clear();
        }
        self.samples.push((now, done));
        if self.samples.len() > WINDOW {
            self.samples.remove(0);
        }
    }

    /// Jednotek za sekundu (z prvniho a posledniho vzorku okna).
    pub fn rate(&self) -> Option<f64> {
        let (a, b) = (self.samples.first()?, self.samples.last()?);
        let dt = (b.0 - a.0) as f64 / 1e9;
        if dt <= 0.0 || b.1 <= a.1 {
            return None;
        }
        Some((b.1 - a.1) as f64 / dt)
    }

    /// Odhad zbyvajiciho casu v ns.
    pub fn eta(&self, total: u64) -> Option<i64> {
        let done = self.samples.last()?.1;
        if total == 0 || done >= total {
            return None;
        }
        let r = self.rate()?;
        Some(((total - done) as f64 / r * 1e9) as i64)
    }
}

/// Lidsky cas `12 s`, `3 min 5 s`.
pub fn human_eta(ns: i64) -> String {
    let s = (ns / 1_000_000_000).max(0);
    match s {
        s if s < 60 => alloc::format!("{s} s"),
        s if s < 3600 => alloc::format!("{} min {} s", s / 60, s % 60),
        s => alloc::format!("{} h {} min", s / 3600, s % 3600 / 60),
    }
}

/// Rychlost s jednotkou podle druhu prubehu (`label` z interpretu).
pub fn human_rate(rate: f64, label: &str) -> String {
    match label {
        "sleep" => String::new(),
        _ if rate >= 10.0 => alloc::format!("{} pol./s", rate as u64),
        _ => alloc::format!("{rate:.1} pol./s"),
    }
}
