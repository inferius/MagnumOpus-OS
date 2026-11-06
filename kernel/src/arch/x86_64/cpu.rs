//! Tenke obaly nad privilegovanymi instrukcemi x86_64.

use core::arch::asm;
use core::arch::x86_64::{__cpuid_count, CpuidResult};

pub const CR0_WP: u64 = 1 << 16;
pub const CR4_PGE: u64 = 1 << 7;

pub const MSR_EFER: u32 = 0xC000_0080;
pub const EFER_NXE: u64 = 1 << 11;

pub fn cpuid(leaf: u32, subleaf: u32) -> CpuidResult {
    __cpuid_count(leaf, subleaf)
}

/// Vlastnosti CPU, ktere jadro pouziva.
#[derive(Debug, Clone, Copy)]
pub struct Features {
    pub nx: bool,
    pub huge_1g: bool,
}

pub fn features() -> Features {
    let ext = cpuid(0x8000_0001, 0);
    Features {
        nx: ext.edx & (1 << 20) != 0,
        huge_1g: ext.edx & (1 << 26) != 0,
    }
}

/// # Safety
///
/// Zapis na I/O port muze mit libovolny vedlejsi efekt na hardware;
/// volajici ruci, ze port patri zarizeni, ktere obsluhuje.
pub unsafe fn outb(port: u16, value: u8) {
    // SAFETY: predano volajicim.
    unsafe {
        asm!("out dx, al", in("dx") port, in("al") value, options(nomem, nostack, preserves_flags))
    };
}

/// # Safety
///
/// Zapis do MSR meni chovani CPU; volajici ruci za platnost hodnoty.
pub unsafe fn wrmsr(msr: u32, value: u64) {
    // SAFETY: predano volajicim.
    unsafe {
        asm!("wrmsr", in("ecx") msr, in("eax") value as u32, in("edx") (value >> 32) as u32,
             options(nomem, nostack, preserves_flags))
    };
}

/// # Safety
///
/// Cteni neexistujiciho MSR vyvola #GP.
pub unsafe fn rdmsr(msr: u32) -> u64 {
    let (lo, hi): (u32, u32);
    // SAFETY: predano volajicim.
    unsafe {
        asm!("rdmsr", in("ecx") msr, out("eax") lo, out("edx") hi, options(nomem, nostack, preserves_flags))
    };
    ((hi as u64) << 32) | lo as u64
}

pub fn read_cr0() -> u64 {
    let v;
    // SAFETY: cteni CR0 nema vedlejsi efekty.
    unsafe { asm!("mov {}, cr0", out(reg) v, options(nomem, nostack, preserves_flags)) };
    v
}

/// # Safety
///
/// Meni zakladni chovani CPU (strankovani, ochrana zapisu, ...).
pub unsafe fn write_cr0(v: u64) {
    // SAFETY: predano volajicim.
    unsafe { asm!("mov cr0, {}", in(reg) v, options(nostack, preserves_flags)) };
}

pub fn read_cr2() -> u64 {
    let v;
    // SAFETY: cteni CR2 nema vedlejsi efekty.
    unsafe { asm!("mov {}, cr2", out(reg) v, options(nomem, nostack, preserves_flags)) };
    v
}

pub fn read_cr3() -> u64 {
    let v;
    // SAFETY: cteni CR3 nema vedlejsi efekty.
    unsafe { asm!("mov {}, cr3", out(reg) v, options(nomem, nostack, preserves_flags)) };
    v
}

/// # Safety
///
/// `v` musi ukazovat na platnou PML4, ktera mapuje prave bezici kod,
/// zasobnik a vsechna data, ktera se budou dal pouzivat.
pub unsafe fn write_cr3(v: u64) {
    // SAFETY: predano volajicim.
    unsafe { asm!("mov cr3, {}", in(reg) v, options(nostack, preserves_flags)) };
}

pub fn read_cr4() -> u64 {
    let v;
    // SAFETY: cteni CR4 nema vedlejsi efekty.
    unsafe { asm!("mov {}, cr4", out(reg) v, options(nomem, nostack, preserves_flags)) };
    v
}

/// # Safety
///
/// Meni chovani CPU; volajici ruci, ze nastavene bity CPU podporuje.
pub unsafe fn write_cr4(v: u64) {
    // SAFETY: predano volajicim.
    unsafe { asm!("mov cr4, {}", in(reg) v, options(nostack, preserves_flags)) };
}

/// Aktualni hodnota RBP (pro pruchod ramci zasobniku).
#[inline(always)]
pub fn read_rbp() -> u64 {
    let v;
    // SAFETY: jen cteni registru.
    unsafe { asm!("mov {}, rbp", out(reg) v, options(nomem, nostack, preserves_flags)) };
    v
}
