//! Lokalni APIC: preruseni od casovace, mezi-procesorova preruseni (IPI).
//!
//! Pokud CPU podporuje x2APIC, pouziva se (pristup pres MSR, 32bitova ID),
//! jinak xAPIC pres MMIO.

use core::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};

use super::cpu;
use crate::mm::{self, PhysAddr};

const MSR_APIC_BASE: u32 = 0x1B;
const APIC_BASE_ENABLE: u64 = 1 << 11;
const APIC_BASE_X2APIC: u64 = 1 << 10;

const REG_ID: u32 = 0x20;
const REG_TPR: u32 = 0x80;
const REG_EOI: u32 = 0xB0;
const REG_SVR: u32 = 0xF0;
const REG_ICR_LOW: u32 = 0x300;
const REG_ICR_HIGH: u32 = 0x310;
const REG_LVT_TIMER: u32 = 0x320;
const REG_TIMER_INITIAL: u32 = 0x380;
const REG_TIMER_CURRENT: u32 = 0x390;
const REG_TIMER_DIVIDE: u32 = 0x3E0;

const SVR_ENABLE: u32 = 1 << 8;
const LVT_MASKED: u32 = 1 << 16;
const ICR_DELIVERY_PENDING: u32 = 1 << 12;
const ICR_LEVEL_ASSERT: u32 = 1 << 14;
const ICR_INIT: u32 = 0b101 << 8;
const ICR_STARTUP: u32 = 0b110 << 8;
const TIMER_DIVIDE_BY_16: u32 = 0b0011;

pub const VECTOR_TIMER: u8 = 32;
pub const VECTOR_RESCHEDULE: u8 = 33;
pub const VECTOR_HALT: u8 = 34;
pub const VECTOR_SPURIOUS: u8 = 0xFF;

static X2APIC: AtomicBool = AtomicBool::new(false);
/// Virtualni adresa registru xAPIC (stejna fyzicka adresa na vsech CPU).
static XAPIC_BASE: AtomicUsize = AtomicUsize::new(0);
/// Tiky APIC timeru (po deleni 16) za sekundu; stejne na vsech CPU.
static TIMER_HZ: AtomicU64 = AtomicU64::new(0);

fn read(reg: u32) -> u32 {
    if X2APIC.load(Ordering::Relaxed) {
        // SAFETY: x2APIC MSR 0x800 + reg/16 existuji, kdyz je x2APIC zapnuty.
        unsafe { cpu::rdmsr(0x800 + (reg >> 4)) as u32 }
    } else {
        let base = XAPIC_BASE.load(Ordering::Relaxed);
        // SAFETY: registr lezi v namapovane strance xAPIC.
        unsafe { ((base + reg as usize) as *const u32).read_volatile() }
    }
}

fn write(reg: u32, value: u32) {
    if X2APIC.load(Ordering::Relaxed) {
        // SAFETY: viz `read`.
        unsafe { cpu::wrmsr(0x800 + (reg >> 4), value as u64) }
    } else {
        let base = XAPIC_BASE.load(Ordering::Relaxed);
        // SAFETY: viz `read`.
        unsafe { ((base + reg as usize) as *mut u32).write_volatile(value) }
    }
}

/// Zapne lokalni APIC na aktualnim CPU. Na bootovacim CPU zvoli rezim
/// (x2APIC / xAPIC), ostatni CPU ho prevezmou.
///
/// # Safety
///
/// Volat jednou na kazdem CPU, s nahranou IDT, pred povolenim preruseni.
pub unsafe fn init_cpu(is_bsp: bool, lapic_phys: u64) {
    if is_bsp {
        let x2 = cpu::cpuid(1, 0).ecx & (1 << 21) != 0;
        X2APIC.store(x2, Ordering::Relaxed);
        if !x2 {
            let base = mm::map_mmio(PhysAddr::new(lapic_phys), 4096);
            XAPIC_BASE.store(base.as_u64() as usize, Ordering::Relaxed);
        }
    }
    let mut base = APIC_BASE_ENABLE;
    if X2APIC.load(Ordering::Relaxed) {
        base |= APIC_BASE_X2APIC;
    }
    // SAFETY: MSR APIC_BASE existuje na kazdem x86_64 CPU; zapnuti x2APIC
    // jen pokud ho CPU podporuje. Fyzickou adresu nemenime.
    unsafe { cpu::wrmsr(MSR_APIC_BASE, cpu::rdmsr(MSR_APIC_BASE) | base) };

    write(REG_TPR, 0);
    write(REG_SVR, SVR_ENABLE | VECTOR_SPURIOUS as u32);
    write(REG_TIMER_DIVIDE, TIMER_DIVIDE_BY_16);
    write(REG_LVT_TIMER, LVT_MASKED | VECTOR_TIMER as u32);
    write(REG_TIMER_INITIAL, 0);
}

pub fn is_x2apic() -> bool {
    X2APIC.load(Ordering::Relaxed)
}

/// ID lokalniho APIC aktualniho CPU.
pub fn id() -> u32 {
    let raw = read(REG_ID);
    if X2APIC.load(Ordering::Relaxed) {
        raw
    } else {
        raw >> 24
    }
}

pub fn eoi() {
    write(REG_EOI, 0);
}

/// Zmeri frekvenci APIC timeru pomoci referencnich hodin.
pub fn calibrate_timer(measure: impl FnOnce(&mut dyn FnMut() -> u64) -> u64) {
    write(REG_TIMER_INITIAL, u32::MAX);
    let hz = measure(&mut || (u32::MAX - read(REG_TIMER_CURRENT)) as u64);
    write(REG_TIMER_INITIAL, 0);
    TIMER_HZ.store(hz, Ordering::Relaxed);
}

pub fn timer_hz() -> u64 {
    TIMER_HZ.load(Ordering::Relaxed)
}

/// Naplanuje jednorazove preruseni casovace za `ns` nanosekund,
/// `None` casovac zastavi.
pub fn set_oneshot(ns: Option<u64>) {
    match ns {
        None => write(REG_TIMER_INITIAL, 0),
        Some(ns) => {
            let ticks = (ns as u128 * TIMER_HZ.load(Ordering::Relaxed) as u128 / 1_000_000_000)
                .clamp(1, u32::MAX as u128) as u32;
            write(REG_LVT_TIMER, VECTOR_TIMER as u32); // one-shot, odmaskovano
            write(REG_TIMER_INITIAL, ticks);
        }
    }
}

fn send_icr(apic_id: u32, low: u32) {
    if X2APIC.load(Ordering::Relaxed) {
        // SAFETY: x2APIC ICR je jeden 64bitovy MSR 0x830.
        unsafe { cpu::wrmsr(0x830, ((apic_id as u64) << 32) | low as u64) };
    } else {
        write(REG_ICR_HIGH, apic_id << 24);
        write(REG_ICR_LOW, low);
        while read(REG_ICR_LOW) & ICR_DELIVERY_PENDING != 0 {
            core::hint::spin_loop();
        }
    }
}

pub fn send_ipi(apic_id: u32, vector: u8) {
    send_icr(apic_id, ICR_LEVEL_ASSERT | vector as u32);
}

pub fn send_init(apic_id: u32) {
    send_icr(apic_id, ICR_LEVEL_ASSERT | ICR_INIT);
}

pub fn send_startup(apic_id: u32, page: u8) {
    send_icr(apic_id, ICR_STARTUP | page as u32);
}
