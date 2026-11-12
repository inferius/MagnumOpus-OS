//! Tenke obaly nad privilegovanymi instrukcemi x86_64.

use core::arch::asm;
use core::arch::x86_64::{__cpuid_count, CpuidResult};

pub const CR0_WP: u64 = 1 << 16;
pub const CR4_PGE: u64 = 1 << 7;
pub const CR4_SMEP: u64 = 1 << 20;
pub const CR4_SMAP: u64 = 1 << 21;

pub const MSR_EFER: u32 = 0xC000_0080;
/// Baze segmentu FS (ukazatel vlakna userspace pro TLS).
pub const MSR_FS_BASE: u32 = 0xC000_0100;
pub const EFER_NXE: u64 = 1 << 11;

pub fn cpuid(leaf: u32, subleaf: u32) -> CpuidResult {
    __cpuid_count(leaf, subleaf)
}

/// Vlastnosti CPU, ktere jadro pouziva.
#[derive(Debug, Clone, Copy)]
pub struct Features {
    pub nx: bool,
    pub huge_1g: bool,
    pub smep: bool,
    pub smap: bool,
}

pub fn features() -> Features {
    let ext = cpuid(0x8000_0001, 0);
    let leaf7 = cpuid(7, 0);
    Features {
        nx: ext.edx & (1 << 20) != 0,
        huge_1g: ext.edx & (1 << 26) != 0,
        smep: leaf7.ebx & (1 << 7) != 0,
        smap: leaf7.ebx & (1 << 20) != 0,
    }
}

/// 64 bitu z RDSEED (primo ze zdroje entropie), jinak z RDRAND (vystup
/// DRBG v procesoru). Instrukce muze docasne selhat (CF = 0), proto par
/// pokusu. `None` = CPU instrukce nema nebo opakovane selhaly.
pub fn hw_random() -> Option<u64> {
    let rdseed = cpuid(7, 0).ebx & (1 << 18) != 0;
    let rdrand = cpuid(1, 0).ecx & (1 << 30) != 0;
    for _ in 0..16 {
        let mut v: u64;
        let ok: u8;
        if rdseed {
            // SAFETY: instrukci CPU podporuje (CPUID), nema vedlejsi ucinky.
            unsafe { asm!("rdseed {}", "setc {}", out(reg) v, out(reg_byte) ok, options(nomem, nostack)) };
        } else if rdrand {
            // SAFETY: jako vyse.
            unsafe { asm!("rdrand {}", "setc {}", out(reg) v, out(reg_byte) ok, options(nomem, nostack)) };
        } else {
            return None;
        }
        if ok == 1 {
            return Some(v);
        }
        core::hint::spin_loop();
    }
    None
}

/// # Safety
///
/// Cteni I/O portu muze mit vedlejsi efekt (napr. potvrzeni stavu);
/// volajici ruci, ze port patri zarizeni, ktere obsluhuje.
pub unsafe fn inb(port: u16) -> u8 {
    let value: u8;
    // SAFETY: predano volajicim.
    unsafe {
        asm!("in al, dx", in("dx") port, out("al") value, options(nomem, nostack, preserves_flags))
    };
    value
}

/// # Safety
///
/// Jako [`inb`] (16 bitu).
pub unsafe fn inw(port: u16) -> u16 {
    let value: u16;
    // SAFETY: predano volajicim.
    unsafe {
        asm!("in ax, dx", in("dx") port, out("ax") value, options(nomem, nostack, preserves_flags))
    };
    value
}

/// # Safety
///
/// Jako [`inb`] (32 bitu).
pub unsafe fn inl(port: u16) -> u32 {
    let value: u32;
    // SAFETY: predano volajicim.
    unsafe {
        asm!("in eax, dx", in("dx") port, out("eax") value, options(nomem, nostack, preserves_flags))
    };
    value
}

/// # Safety
///
/// Jako [`outb`] (16 bitu).
pub unsafe fn outw(port: u16, value: u16) {
    // SAFETY: predano volajicim.
    unsafe {
        asm!("out dx, ax", in("dx") port, in("ax") value, options(nomem, nostack, preserves_flags))
    };
}

/// # Safety
///
/// Jako [`outb`] (32 bitu).
pub unsafe fn outl(port: u16, value: u32) {
    // SAFETY: predano volajicim.
    unsafe {
        asm!("out dx, eax", in("dx") port, in("eax") value, options(nomem, nostack, preserves_flags))
    };
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

/// Zneplatni zaznam TLB pro jednu stranku na tomto CPU.
pub fn invlpg(addr: u64) {
    // SAFETY: invlpg jen zahodi zaznam z TLB, nemeni pamet.
    unsafe { asm!("invlpg [{}]", in(reg) addr, options(nostack, preserves_flags)) };
}
