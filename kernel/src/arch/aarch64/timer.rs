//! Generic timer: hodiny z citace CNTVCT_EL0 a jednorazovy casovac pres
//! virtualni casovac EL1 (CNTV_TVAL_EL0, preruseni PPI 27).
//!
//! Citac je podle architektury stejny na vsech CPU a jeho frekvence je
//! v CNTFRQ_EL0 (nastavi firmware), takze na rozdil od x86 neni potreba
//! kalibrace ani srovnani mezi CPU. Virtualni casovac je dostupny v EL1
//! vzdy (i pod hypervizorem), proto ho jadro pouziva misto fyzickeho.

use core::sync::atomic::{AtomicU64, Ordering};

use super::regs::{isb, mrs, msr};

/// Preruseni virtualniho casovace EL1 (PPI, INTID 27).
pub const INTID: u32 = 27;

static BASE: AtomicU64 = AtomicU64::new(0);
/// ns = (tiky * MULT) >> 32
static NS_MULT: AtomicU64 = AtomicU64::new(0);
static HZ: AtomicU64 = AtomicU64::new(0);

const CTL_ENABLE: u64 = 1 << 0;
const CTL_IMASK: u64 = 1 << 1;

/// Hodnota citace (isb: necist spekulativne driv nez predchozi instrukce).
pub fn counter() -> u64 {
    isb();
    mrs!("cntvct_el0")
}

/// Zacne merit cas od tohoto okamziku. Volat jednou, co nejdrive.
pub fn init() {
    let hz = mrs!("cntfrq_el0").max(1);
    HZ.store(hz, Ordering::Relaxed);
    NS_MULT.store(
        ((1_000_000_000u128 << 32) / hz as u128) as u64,
        Ordering::Relaxed,
    );
    BASE.store(counter(), Ordering::Relaxed);
}

pub fn hz() -> u64 {
    HZ.load(Ordering::Relaxed)
}

/// Nanosekundy od `init`.
pub fn now_ns() -> u64 {
    let delta = counter().saturating_sub(BASE.load(Ordering::Relaxed));
    ((delta as u128 * NS_MULT.load(Ordering::Relaxed) as u128) >> 32) as u64
}

/// Nanosekundy od hodnoty citace `c` do startu hodin (zaporne, kdyz `c` je
/// az po startu).
pub fn ns_from_counter_to_start(c: u64) -> i128 {
    let delta = BASE.load(Ordering::Relaxed) as i128 - c as i128;
    (delta * NS_MULT.load(Ordering::Relaxed) as i128) >> 32
}

/// Naplanuje preruseni za `ns` (nebo ho zastavi).
pub fn set_oneshot(ns: Option<u64>) {
    match ns {
        Some(ns) => {
            let ticks = (ns as u128 * hz() as u128 / 1_000_000_000).clamp(1, i32::MAX as u128);
            // SAFETY: registry virtualniho casovace tohoto CPU.
            unsafe {
                msr!("cntv_tval_el0", ticks as u64);
                msr!("cntv_ctl_el0", CTL_ENABLE);
            }
        }
        // SAFETY: viz vyse; maska zastavi preruseni.
        None => unsafe { msr!("cntv_ctl_el0", CTL_IMASK) },
    }
    isb();
}

/// Obsluha preruseni: casovac je jednorazovy, zamaskovat ho (dalsi
/// nastavi planovac pres `set_oneshot`).
pub fn on_interrupt() {
    // SAFETY: viz `set_oneshot`.
    unsafe { msr!("cntv_ctl_el0", CTL_IMASK) };
    isb();
}

pub fn busy_wait_ns(ns: u64) {
    let end = now_ns() + ns;
    while now_ns() < end {
        core::hint::spin_loop();
    }
}
