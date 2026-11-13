//! Prubeh v realnem case: rychlost a odhad zbyvajiciho casu si okno
//! pocita samo z casu aktualizaci (sluzba posila jen `done` a `total`).

use alloc::string::String;

/// Vyhlazeni rychlosti (EMA): vaha noveho vzorku.
const ALPHA: f64 = 0.2;
/// Prvni chvili odhad neukazovat ("odhaduji...").
const WARMUP_NS: i64 = 2_000_000_000;
/// Kratsi zbyvajici cas odhad neukazuje (nema smysl).
const SHORT_NS: i64 = 4_000_000_000;
/// Obnova zobrazeneho odhadu: ~1/10 zbyvajiciho casu, v techto mezich.
const REFRESH_MIN_NS: i64 = 250_000_000;
const REFRESH_MAX_NS: i64 = 5_000_000_000;
/// Nahoru se odhad zmeni jen pri velke zmene (o polovinu).
const UP_FACTOR: f64 = 1.5;

/// Stopa prubehu: vyhlazena rychlost a citelny odhad zbyvajiciho casu.
#[derive(Debug, Clone, Default)]
pub struct Track {
    /// Posledni vzorek (cas v ns, hotovo).
    last: Option<(i64, u64)>,
    /// Zacatek mereni.
    first: Option<i64>,
    /// Vyhlazena rychlost (jednotek za sekundu).
    ema: Option<f64>,
    /// Zobrazeny odhad: (zbyvajici ns v case zobrazeni, cas zobrazeni).
    shown: core::cell::Cell<Option<(i64, i64)>>,
}

impl Track {
    pub fn push(&mut self, now: i64, done: u64) {
        if let Some((t0, d0)) = self.last {
            if done < d0 {
                // Novy beh (mensi hodnota): zacit znovu.
                *self = Track::default();
            } else {
                let dt = (now - t0) as f64 / 1e9;
                if dt > 0.0 {
                    let inst = (done - d0) as f64 / dt;
                    self.ema = Some(match self.ema {
                        Some(e) => e + ALPHA * (inst - e),
                        None => inst,
                    });
                }
            }
        }
        self.first.get_or_insert(now);
        self.last = Some((now, done));
    }

    /// Vyhlazena rychlost (jednotek za sekundu).
    pub fn rate(&self) -> Option<f64> {
        self.ema.filter(|r| *r > 0.0)
    }

    /// Surovy odhad zbyvajiciho casu v ns (z vyhlazene rychlosti).
    pub fn eta(&self, total: u64) -> Option<i64> {
        let (_, done) = self.last?;
        if total == 0 || done >= total {
            return None;
        }
        Some(((total - done) as f64 / self.rate()? * 1e9) as i64)
    }

    /// Text odhadu pro cloveka v case `now`: prvni chvili "odhaduji...",
    /// kratke ulohy nic, jinak zaokrouhleny cas, ktery se obnovuje umerne
    /// delce a jen klesa (nahoru az pri velke zmene rychlosti).
    pub fn eta_text(&self, now: i64, total: u64) -> Option<String> {
        let first = self.first?;
        if now - first < WARMUP_NS {
            return Some(String::from("odhaduji\u{2026}"));
        }
        let raw = self.eta(total)?;
        let value = match self.shown.get() {
            Some((val, at)) => {
                // Zobrazena hodnota mezitim ubyva s casem.
                let current = (val - (now - at)).max(0);
                let refresh = (current / 10).clamp(REFRESH_MIN_NS, REFRESH_MAX_NS);
                if now - at < refresh {
                    return round_eta(current);
                }
                if raw < current || raw as f64 > current as f64 * UP_FACTOR { raw } else { current }
            }
            None => raw,
        };
        self.shown.set(Some((value, now)));
        round_eta(value)
    }
}

/// Zaokrouhleni podle radu: pod 10 s po sekunde, do minuty po 5 s, pak
/// "~3 min", od hodiny "~1 h 20 min"; kratky zbytek nic.
fn round_eta(ns: i64) -> Option<String> {
    if ns < SHORT_NS {
        return None;
    }
    let s = (ns + 500_000_000) / 1_000_000_000;
    Some(match s {
        s if s < 10 => alloc::format!("{s} s"),
        s if s < 60 => alloc::format!("{} s", (s + 2) / 5 * 5),
        s if s < 3600 => alloc::format!("~{} min", (s + 30) / 60),
        s => {
            let m = (s + 150) / 300 * 5;
            if m % 60 == 0 { alloc::format!("~{} h", m / 60) } else { alloc::format!("~{} h {} min", m / 60, m % 60) }
        }
    })
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
