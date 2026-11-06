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
    // saturating: kdyby TSC tohoto CPU byl o kousek za BSP, cas nesmi
    // pretect do obrovske hodnoty (posun se opravuje pri startu CPU).
    let delta = rdtsc().saturating_sub(TSC_BASE.load(Ordering::Relaxed));
    ((delta as u128 * NS_MULT.load(Ordering::Relaxed) as u128) >> 32) as u64
}

pub fn busy_wait_ns(ns: u64) {
    let end = now_ns() + ns;
    while now_ns() < end {
        core::hint::spin_loop();
    }
}

const MSR_TSC_ADJUST: u32 = 0x3B;

/// Posune TSC tohoto CPU o `delta` tiku pres MSR IA32_TSC_ADJUST.
/// Vraci false, pokud to CPU nepodporuje.
pub fn adjust(delta: i64) -> bool {
    if cpu::cpuid(7, 0).ebx & (1 << 1) == 0 {
        return false;
    }
    // SAFETY: MSR existuje (overeno CPUID); zmena posune jen TSC tohoto CPU.
    unsafe {
        let v = cpu::rdmsr(MSR_TSC_ADJUST) as i64;
        cpu::wrmsr(MSR_TSC_ADJUST, v.wrapping_add(delta) as u64);
    }
    true
}
