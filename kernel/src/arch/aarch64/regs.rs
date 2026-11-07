//! Systemove registry, bariery a udrzba cache.

use core::arch::asm;

/// Precte systemovy registr: `mrs!("mpidr_el1")`.
macro_rules! mrs {
    ($reg:literal) => {{
        let v: u64;
        // SAFETY: cteni systemoveho registru v EL1 nema vedlejsi ucinky na
        // pamet (volajici pouziva jen registry citelne v EL1).
        #[allow(unused_unsafe)]
        unsafe { core::arch::asm!(concat!("mrs {}, ", $reg), out(reg) v, options(nomem, nostack, preserves_flags)) };
        v
    }};
}

/// Zapise systemovy registr: `msr!("vbar_el1", v)`. Neni `unsafe` samo
/// o sobe - volajici musi byt v `unsafe` bloku a rucit za dusledky.
macro_rules! msr {
    ($reg:literal, $v:expr) => {{
        let v: u64 = $v;
        core::arch::asm!(concat!("msr ", $reg, ", {}"), in(reg) v, options(nostack, preserves_flags));
    }};
}

pub(super) use {mrs, msr};

#[inline(always)]
pub fn isb() {
    // SAFETY: bariera nemeni stav pameti.
    unsafe { asm!("isb", options(nostack, preserves_flags)) };
}

#[inline(always)]
pub fn dsb_sy() {
    // SAFETY: viz `isb`.
    unsafe { asm!("dsb sy", options(nostack, preserves_flags)) };
}

#[inline(always)]
pub fn dsb_ish() {
    // SAFETY: viz `isb`.
    unsafe { asm!("dsb ish", options(nostack, preserves_flags)) };
}

#[inline(always)]
pub fn dsb_ishst() {
    // SAFETY: viz `isb`.
    unsafe { asm!("dsb ishst", options(nostack, preserves_flags)) };
}

/// Velikost radku datove cache v bajtech (CTR_EL0.DminLine).
pub fn dcache_line() -> u64 {
    4 << ((mrs!("ctr_el0") >> 16) & 0xF)
}

/// Zapise a zneplatni datovou cache pro rozsah virtualnich adres (PoC):
/// pamet i ostatni pozorovatele (zarizeni, CPU bez cache) pak vidi totez.
pub fn dcache_clean_invalidate(virt: u64, len: u64) {
    let line = dcache_line();
    let mut addr = virt & !(line - 1);
    while addr < virt + len {
        // SAFETY: `dc civac` jen zapise a zneplatni radek, data nemeni;
        // adresa je namapovana (volajici).
        unsafe { asm!("dc civac, {}", in(reg) addr, options(nostack, preserves_flags)) };
        addr += line;
    }
    dsb_sy();
}

/// Hardwarove ID z MPIDR: Aff3 << 24 | Aff2 << 16 | Aff1 << 8 | Aff0.
pub fn mpidr_to_hw_id(mpidr: u64) -> u32 {
    ((mpidr & 0xFF_FFFF) | ((mpidr >> 32) & 0xFF) << 24) as u32
}

/// Opak `mpidr_to_hw_id` (hodnota afinity pro PSCI a GIC).
pub fn hw_id_to_mpidr(id: u32) -> u64 {
    (id as u64 & 0xFF_FFFF) | ((id as u64 >> 24) & 0xFF) << 32
}
