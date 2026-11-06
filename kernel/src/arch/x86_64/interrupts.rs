//! Povoleni a zakazani preruseni na aktualnim CPU.

use core::arch::asm;

const RFLAGS_IF: u64 = 1 << 9;

pub fn are_enabled() -> bool {
    let flags: u64;
    // SAFETY: pushfq/pop jen cte RFLAGS.
    unsafe { asm!("pushfq", "pop {}", out(reg) flags, options(nomem, preserves_flags)) };
    flags & RFLAGS_IF != 0
}

/// Zakaze preruseni a vrati, zda byla predtim povolena.
pub fn disable() -> bool {
    let was = are_enabled();
    // SAFETY: cli jen zakaze preruseni na tomto CPU.
    unsafe { asm!("cli", options(nomem, nostack)) };
    was
}

/// Obnovi stav vraceny `disable`.
pub fn restore(was_enabled: bool) {
    if was_enabled {
        enable();
    }
}

pub fn enable() {
    // SAFETY: povoleni preruseni; IDT je nahrana pri startu CPU.
    unsafe { asm!("sti", options(nomem, nostack)) };
}

/// Atomicky povoli preruseni a uspi CPU do dalsiho preruseni
/// (sti ma zpozdeni jedne instrukce, takze preruseni mezi nimi neutece).
pub fn enable_and_halt() {
    // SAFETY: viz `enable`; hlt jen ceka na preruseni.
    unsafe { asm!("sti", "hlt", options(nomem, nostack)) };
}
