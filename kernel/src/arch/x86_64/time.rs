//! Monotonicke hodiny z TSC.
//!
//! Predpoklad: invariantni TSC synchronizovany mezi CPU (standard u
//! modernich x86_64). Frekvence se kalibruje proti HPET.

use core::arch::x86_64::_rdtsc;
use core::sync::atomic::{AtomicU64, Ordering};

use super::cpu;

static TSC_BASE: AtomicU64 = AtomicU64::new(0);
/// ns = (tiky * MULT) >> 32
static NS_MULT: AtomicU64 = AtomicU64::new(0);

pub fn rdtsc() -> u64 {
    // SAFETY: RDTSC je dostupna na kazdem x86_64 CPU (jadro nevypina CR4.TSD).
    unsafe { _rdtsc() }
}

pub fn is_invariant() -> bool {
    cpu::cpuid(0x8000_0007, 0).edx & (1 << 8) != 0
}

/// Nastavi frekvenci TSC; cas 0 = okamzik volani.
pub fn init(tsc_hz: u64) {
    NS_MULT.store(
        ((1_000_000_000u128 << 32) / tsc_hz as u128) as u64,
        Ordering::Relaxed,
    );
    TSC_BASE.store(rdtsc(), Ordering::Relaxed);
}

/// Nanosekundy od startu jadra.
pub fn now_ns() -> u64 {
    let delta = rdtsc().wrapping_sub(TSC_BASE.load(Ordering::Relaxed));
    ((delta as u128 * NS_MULT.load(Ordering::Relaxed) as u128) >> 32) as u64
}

pub fn busy_wait_ns(ns: u64) {
    let end = now_ns() + ns;
    while now_ns() < end {
        core::hint::spin_loop();
    }
}
