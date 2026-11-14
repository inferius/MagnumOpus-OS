//! Kontext uzivatelskeho vlakna aarch64 (`api::UserState`).
//!
//! Ramec registru je na vrcholu zasobniku jadra vlakna (vstup z EL0
//! odecte `TrapFrame` od SP_EL1 = vrchol). Stav FPU/SIMD (q0-q31, FPCR,
//! FPSR) je v `Context`.

use core::arch::asm;

use abi::thread::{A64_FP_SIZE, A64_FPCR_MASK, A64_FPSR_MASK, A64General};
use abi::vm::USER_MAX;

use super::Aarch64;
use super::context::Context;
use super::exceptions::TrapFrame;
use crate::arch::api::UserState;

/// Priznaky NZCV v PSTATE (jediny cast, kterou smi program zmenit; rezim
/// EL0t a masky DAIF zustavaji 0).
const PSTATE_NZCV: u64 = 0xF000_0000;

fn frame(stack_top: u64) -> *mut TrapFrame {
    (stack_top - size_of::<TrapFrame>() as u64) as *mut TrapFrame
}

fn general(buf: &[u8]) -> Option<A64General> {
    Some(A64General::decode(buf.try_into().ok()?))
}

fn fp_regs(buf: &[u8]) -> (u64, u64) {
    let v = |i: usize| u64::from_le_bytes(buf[i..i + 8].try_into().unwrap());
    (v(512), v(520))
}

impl UserState for Aarch64 {
    const GENERAL_SIZE: usize = A64General::SIZE;
    const FP_SIZE: usize = A64_FP_SIZE;

    unsafe fn read_general(stack_top: u64, tp: u64, out: &mut [u8]) {
        // SAFETY: predano volajicim (ramec stojiciho vlakna).
        let f = unsafe { &*frame(stack_top) };
        let g = A64General { x: f.x, sp: f.sp_el0, pc: f.elr, pstate: f.spsr, tpidr: tp };
        out.copy_from_slice(&g.encode());
    }

    fn check_general(buf: &[u8]) -> bool {
        general(buf).is_some_and(|g| g.pc < USER_MAX && g.sp <= USER_MAX && g.tpidr < USER_MAX)
    }

    unsafe fn write_general(stack_top: u64, buf: &[u8]) -> u64 {
        let g = general(buf).expect("overeno v check_general");
        // SAFETY: predano volajicim (vlastni ramec, zakazana preruseni).
        let f = unsafe { &mut *frame(stack_top) };
        f.x = g.x;
        f.sp_el0 = g.sp;
        f.elr = g.pc;
        // Rezim (EL0t) a masky preruseni se nemeni: jen NZCV.
        f.spsr = g.pstate & PSTATE_NZCV;
        g.tpidr
    }

    fn read_fp(ctx: &Context, out: &mut [u8]) {
        out.copy_from_slice(ctx.fpu_bytes());
    }

    fn check_fp(buf: &[u8]) -> bool {
        if buf.len() != A64_FP_SIZE {
            return false;
        }
        let (fpcr, fpsr) = fp_regs(buf);
        fpcr & !A64_FPCR_MASK == 0 && fpsr & !A64_FPSR_MASK == 0
    }

    unsafe fn load_fp(buf: &[u8]) {
        let (fpcr, fpsr) = fp_regs(buf);
        // SAFETY: buffer ma 528 B (overeno), FPCR/FPSR jen definovane bity.
        // Jadro registry FPU/SIMD nepouziva (cil softfloat), stav patri
        // vlaknu, ktere se vraci do userspace.
        unsafe {
            asm!(
                ".arch_extension fp",
                ".arch_extension simd",
                "ldp q0, q1, [{p}, #0]",
                "ldp q2, q3, [{p}, #32]",
                "ldp q4, q5, [{p}, #64]",
                "ldp q6, q7, [{p}, #96]",
                "ldp q8, q9, [{p}, #128]",
                "ldp q10, q11, [{p}, #160]",
                "ldp q12, q13, [{p}, #192]",
                "ldp q14, q15, [{p}, #224]",
                "ldp q16, q17, [{p}, #256]",
                "ldp q18, q19, [{p}, #288]",
                "ldp q20, q21, [{p}, #320]",
                "ldp q22, q23, [{p}, #352]",
                "ldp q24, q25, [{p}, #384]",
                "ldp q26, q27, [{p}, #416]",
                "ldp q28, q29, [{p}, #448]",
                "ldp q30, q31, [{p}, #480]",
                "msr fpcr, {cr}",
                "msr fpsr, {sr}",
                p = in(reg) buf.as_ptr(),
                cr = in(reg) fpcr,
                sr = in(reg) fpsr,
                options(nostack, readonly)
            )
        };
    }
}
