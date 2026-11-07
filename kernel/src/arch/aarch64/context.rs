//! Prepinani kontextu vlaken jadra a prechod do EL0.
//!
//! `switch` uklada registry, ktere podle AAPCS64 musi zachovat volany
//! (x19-x28, x29, x30, sp), a cely stav FPU/SIMD programu (v0-v31, FPCR,
//! FPSR, 528 B). Jadro se preklada bez FPU (`-softfloat`), takze registry
//! FPU mezi vstupem do jadra a prepnutim patri porad programu - staci je
//! ukladat jen tady. Nove vlakno zacina s vynulovanym stavem FPU.

use alloc::boxed::Box;
use core::arch::{asm, naked_asm};

use crate::arch::api::ThreadContext;

/// Stav FPU/SIMD: q0-q31 (512 B), FPCR a FPSR.
#[repr(C, align(16))]
pub struct FpuArea([u8; 528]);

/// Ulozeny kontext vlakna: ukazatel zasobniku (registry jsou na zasobniku)
/// a stav FPU. Poradi poli pouziva `switch` (offset 0 a 8).
#[repr(C)]
pub struct Context {
    sp: u64,
    fpu: Box<FpuArea>,
}

impl Default for Context {
    fn default() -> Self {
        Self {
            sp: 0,
            fpu: Box::new(FpuArea([0; 528])),
        }
    }
}

impl core::fmt::Debug for Context {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Context")
            .field("sp", &self.sp)
            .finish_non_exhaustive()
    }
}

/// Velikost ramce na zasobniku pro `switch` (x19-x30, 12 slov).
const FRAME: usize = 12 * 8;

impl ThreadContext for Context {
    unsafe fn new(stack_top: u64, start: extern "C" fn() -> !) -> Self {
        let mut frame = [0u64; 12];
        // Poradi jako v `switch`: x19..x28, x29 (0 = konec retezu ramcu),
        // x30 (navratova adresa pro `ret`).
        frame[11] = start as usize as u64;
        let sp = stack_top - FRAME as u64;
        // SAFETY: zapis 96 B pod vrchol zasobniku, ktery vlastnime.
        unsafe { core::ptr::copy_nonoverlapping(frame.as_ptr(), sp as *mut u64, frame.len()) };
        Self {
            sp,
            fpu: Box::new(FpuArea([0; 528])),
        }
    }
}

/// Ulozi aktualni kontext do `from` a prepne na `to`.
///
/// # Safety
///
/// Viz `api::Threads::switch`.
#[unsafe(naked)]
pub unsafe extern "C" fn switch(from: *mut Context, to: *const Context) {
    naked_asm!(
        ".arch_extension fp",
        ".arch_extension simd",
        // Stav FPU/SIMD (x9-x12 smi volany prepsat).
        "ldr x9, [x0, #8]",
        "stp q0, q1, [x9, #0]",
        "stp q2, q3, [x9, #32]",
        "stp q4, q5, [x9, #64]",
        "stp q6, q7, [x9, #96]",
        "stp q8, q9, [x9, #128]",
        "stp q10, q11, [x9, #160]",
        "stp q12, q13, [x9, #192]",
        "stp q14, q15, [x9, #224]",
        "stp q16, q17, [x9, #256]",
        "stp q18, q19, [x9, #288]",
        "stp q20, q21, [x9, #320]",
        "stp q22, q23, [x9, #352]",
        "stp q24, q25, [x9, #384]",
        "stp q26, q27, [x9, #416]",
        "stp q28, q29, [x9, #448]",
        "stp q30, q31, [x9, #480]",
        "mrs x10, fpcr",
        "mrs x11, fpsr",
        "add x12, x9, #512",
        "stp x10, x11, [x12]",
        "ldr x9, [x1, #8]",
        "ldp q0, q1, [x9, #0]",
        "ldp q2, q3, [x9, #32]",
        "ldp q4, q5, [x9, #64]",
        "ldp q6, q7, [x9, #96]",
        "ldp q8, q9, [x9, #128]",
        "ldp q10, q11, [x9, #160]",
        "ldp q12, q13, [x9, #192]",
        "ldp q14, q15, [x9, #224]",
        "ldp q16, q17, [x9, #256]",
        "ldp q18, q19, [x9, #288]",
        "ldp q20, q21, [x9, #320]",
        "ldp q22, q23, [x9, #352]",
        "ldp q24, q25, [x9, #384]",
        "ldp q26, q27, [x9, #416]",
        "ldp q28, q29, [x9, #448]",
        "ldp q30, q31, [x9, #480]",
        "add x12, x9, #512",
        "ldp x10, x11, [x12]",
        "msr fpcr, x10",
        "msr fpsr, x11",
        // Registry zachovavane volanym.
        "sub sp, sp, #96",
        "stp x19, x20, [sp, #0]",
        "stp x21, x22, [sp, #16]",
        "stp x23, x24, [sp, #32]",
        "stp x25, x26, [sp, #48]",
        "stp x27, x28, [sp, #64]",
        "stp x29, x30, [sp, #80]",
        "mov x9, sp",
        "str x9, [x0]",
        "ldr x9, [x1]",
        "mov sp, x9",
        "ldp x19, x20, [sp, #0]",
        "ldp x21, x22, [sp, #16]",
        "ldp x23, x24, [sp, #32]",
        "ldp x25, x26, [sp, #48]",
        "ldp x27, x28, [sp, #64]",
        "ldp x29, x30, [sp, #80]",
        "add sp, sp, #96",
        "ret",
    )
}

/// Prepne na novy zasobnik a zavola `f(arg)`.
///
/// # Safety
///
/// Viz `api::Threads::switch_stack`.
pub unsafe fn switch_stack(top: u64, f: extern "C" fn(u64) -> !, arg: u64) -> ! {
    // SAFETY: predano volajicim; x29 = x30 = 0 ukonci retez ramcu.
    unsafe {
        asm!(
            "mov sp, {top}",
            "mov x29, xzr",
            "mov x30, xzr",
            "br {f}",
            top = in(reg) top,
            f = in(reg) f,
            in("x0") arg,
            options(noreturn)
        )
    }
}

/// Prvni vstup aktualniho vlakna do EL0: `entry(arg0, arg1)` se zasobnikem
/// `stack`, preruseni povolena. `kernel_top` = vrchol zasobniku jadra
/// vlakna (SP_EL1 pri pristi vyjimce z EL0).
///
/// # Safety
///
/// Viz `api::Threads::enter_user`.
pub unsafe fn enter_user(entry: u64, stack: u64, arg0: u64, arg1: u64, kernel_top: u64) -> ! {
    // SAFETY: SPSR = EL0t s povolenymi prerusenimi (DAIF = 0). Registry se
    // nuluji, aby do programu neunikla data jadra.
    unsafe {
        asm!(
            "msr daifset, #0xf",
            "msr sp_el0, {stack}",
            "msr elr_el1, {entry}",
            "msr spsr_el1, xzr",
            "mov sp, {ktop}",
            "mov x2, xzr", "mov x3, xzr", "mov x4, xzr", "mov x5, xzr",
            "mov x6, xzr", "mov x7, xzr", "mov x8, xzr", "mov x9, xzr",
            "mov x10, xzr", "mov x11, xzr", "mov x12, xzr", "mov x13, xzr",
            "mov x14, xzr", "mov x15, xzr", "mov x16, xzr", "mov x17, xzr",
            "mov x18, xzr", "mov x19, xzr", "mov x20, xzr", "mov x21, xzr",
            "mov x22, xzr", "mov x23, xzr", "mov x24, xzr", "mov x25, xzr",
            "mov x26, xzr", "mov x27, xzr", "mov x28, xzr", "mov x29, xzr",
            "mov x30, xzr",
            "eret",
            stack = in(reg) stack,
            entry = in(reg) entry,
            ktop = in(reg) kernel_top,
            in("x0") arg0,
            in("x1") arg1,
            options(noreturn)
        )
    }
}
