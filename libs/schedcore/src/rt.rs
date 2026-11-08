//! Realtime: pevny rozpocet v periode (server s konstantni sirkou pasma,
//! CBS - Abeni a Buttazzo 1998) a prijeti noveho vlakna.
//!
//! Vlakno dostane `budget` ns v kazde `period`. Dokud ma rozpocet, bezi
//! pred vsemi spravedlivymi tridami (mezi sebou EDF podle konce periody).
//! Po vycerpani do konce periody bezi jako Interactive (neni zastaveno -
//! mirne podhodnoceny rozpocet zvuku nezpusobi prasknuti, a pritom
//! nemuze zahltit system).

use core::sync::atomic::{AtomicU64, Ordering};

use abi::sched::{
    RT_BUDGET_MIN_US, RT_DEFAULT_BUDGET_US, RT_DEFAULT_PERIOD_US, RT_PERIOD_MAX_US,
    RT_PERIOD_MIN_US,
};

use crate::class::RT_MAX_PERMILLE;

/// Stav rozpoctu jednoho Realtime vlakna.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rt {
    pub budget: u64,
    pub period: u64,
    /// Zbytek rozpoctu v aktualni periode.
    pub remaining: u64,
    /// Konec aktualni periody (ns, cas systemu). 0 = jeste nezacala.
    pub period_end: u64,
}

impl Rt {
    pub const fn new(budget_ns: u64, period_ns: u64) -> Self {
        Self {
            budget: budget_ns,
            period: period_ns,
            remaining: 0,
            period_end: 0,
        }
    }

    /// Nova perioda, pokud ta aktualni skoncila.
    pub fn refresh(&mut self, now: u64) {
        if now >= self.period_end {
            self.remaining = self.budget;
            self.period_end = now + self.period;
        }
    }

    /// Probuzeni (pravidlo CBS): kdyby zbytek rozpoctu do konce periody
    /// dal vic nez `budget / period`, zacne nova perioda od ted. Vlakno
    /// tak spanim nenahromadi narazovy beh nad svou sirku pasma.
    pub fn wake(&mut self, now: u64) {
        let left = self.period_end.saturating_sub(now);
        // remaining / left > budget / period  <=>  remaining * period > left * budget
        if now >= self.period_end
            || (self.remaining as u128) * (self.period as u128) > (left as u128) * (self.budget as u128)
        {
            self.remaining = self.budget;
            self.period_end = now + self.period;
        }
    }

    pub fn has_budget(&self) -> bool {
        self.remaining > 0
    }

    /// Spotreba `ran` ns.
    pub fn charge(&mut self, ran: u64) {
        self.remaining = self.remaining.saturating_sub(ran);
    }

    /// Vyuziti v promile periody (zaokrouhleno nahoru).
    pub fn permille(&self) -> u64 {
        (self.budget * 1000).div_ceil(self.period)
    }
}

/// Overi a doplni parametry z ABI (us; 0 = vychozi). Vraci (budget, period)
/// v ns, nebo `None` pro neplatne.
pub fn params_from_us(budget_us: u32, period_us: u32) -> Option<(u64, u64)> {
    let (b, p) = match (budget_us, period_us) {
        (0, 0) => (RT_DEFAULT_BUDGET_US, RT_DEFAULT_PERIOD_US),
        (0, _) | (_, 0) => return None,
        bp => bp,
    };
    if !(RT_PERIOD_MIN_US..=RT_PERIOD_MAX_US).contains(&p) || b < RT_BUDGET_MIN_US {
        return None;
    }
    // Jedno vlakno nejvys polovina periody (zbytek CPU pro ostatni).
    if (b as u64) * 1000 > (p as u64) * RT_MAX_PERMILLE {
        return None;
    }
    Some((b as u64 * 1000, p as u64 * 1000))
}

/// Prijeti Realtime vlaken: soucet vyuziti vsech nesmi prekrocit
/// `RT_MAX_PERMILLE` kapacity vsech CPU.
pub struct Admission {
    used: AtomicU64,
}

impl Admission {
    pub const fn new() -> Self {
        Self {
            used: AtomicU64::new(0),
        }
    }

    /// Prijme vlakno s vyuzitim `permille` na systemu s `cpus` CPU.
    pub fn admit(&self, permille: u64, cpus: usize) -> bool {
        let limit = RT_MAX_PERMILLE * cpus as u64;
        let mut u = self.used.load(Ordering::Acquire);
        loop {
            if u + permille > limit {
                return false;
            }
            match self.used.compare_exchange_weak(u, u + permille, Ordering::AcqRel, Ordering::Acquire) {
                Ok(_) => return true,
                Err(now) => u = now,
            }
        }
    }

    /// Vlakno skoncilo nebo zmenilo tridu.
    pub fn release(&self, permille: u64) {
        self.used.fetch_sub(permille, Ordering::AcqRel);
    }

    pub fn used(&self) -> u64 {
        self.used.load(Ordering::Acquire)
    }
}

impl Default for Admission {
    fn default() -> Self {
        Self::new()
    }
}
