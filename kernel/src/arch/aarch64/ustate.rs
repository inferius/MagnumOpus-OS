//! Kontext uzivatelskeho vlakna aarch64 (`api::UserState`).
//!
//! Ramec registru je na vrcholu zasobniku jadra vlakna (vstup z EL0
//! odecte `TrapFrame` od SP_EL1 = vrchol). Stav FPU/SIMD (q0-q31, FPCR,
//! FPSR) je v `Context`.

use core::arch::asm;

use abi::thread::fp::{HEADER, Header, format};
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

/// Hlavicka stavu FPU (pevny format NEON).
const FP_HEADER: Header = Header { format: format::A64_NEON, len: A64_FP_SIZE as u32, features: 0 };

/// FPCR a FPSR z dat stavu FPU (bez hlavicky).
fn fp_regs(buf: &[u8]) -> (u64, u64) {
    let v = |i: usize| u64::from_le_bytes(buf[i..i + 8].try_into().unwrap());
    (v(512), v(520))
}

impl UserState for Aarch64 {
    const GENERAL_SIZE: usize = A64General::SIZE;
    fn fp_size() -> usize {
        HEADER + A64_FP_SIZE
    }

    fn fp_header() -> Header {
        FP_HEADER
    }

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
        out[..HEADER].copy_from_slice(&FP_HEADER.encode());
        out[HEADER..].copy_from_slice(ctx.fpu_bytes());
    }

    fn check_fp(buf: &[u8]) -> bool {
        if buf.len() != Self::fp_size() || Header::decode(buf) != Some(FP_HEADER) {
            return false;
        }
        let (fpcr, fpsr) = fp_regs(&buf[HEADER..]);
        fpcr & !A64_FPCR_MASK == 0 && fpsr & !A64_FPSR_MASK == 0
    }

    const FRAME_FP: usize = abi::exception::frame::A64_FP;

    unsafe fn save_fp(out: &mut [u8]) {
        let mut b = [0u8; A64_FP_SIZE];
        let (fpcr, fpsr): (u64, u64);
        // SAFETY: buffer 528 B; jen cteni registru FPU/SIMD vlakna.
        unsafe {
            asm!(
                ".arch_extension fp",
                ".arch_extension simd",
                "stp q0, q1, [{p}, #0]",
                "stp q2, q3, [{p}, #32]",
                "stp q4, q5, [{p}, #64]",
                "stp q6, q7, [{p}, #96]",
                "stp q8, q9, [{p}, #128]",
                "stp q10, q11, [{p}, #160]",
                "stp q12, q13, [{p}, #192]",
                "stp q14, q15, [{p}, #224]",
                "stp q16, q17, [{p}, #256]",
                "stp q18, q19, [{p}, #288]",
                "stp q20, q21, [{p}, #320]",
                "stp q22, q23, [{p}, #352]",
                "stp q24, q25, [{p}, #384]",
                "stp q26, q27, [{p}, #416]",
                "stp q28, q29, [{p}, #448]",
                "stp q30, q31, [{p}, #480]",
                "mrs {cr}, fpcr",
                "mrs {sr}, fpsr",
                p = in(reg) b.as_mut_ptr(),
                cr = out(reg) fpcr,
                sr = out(reg) fpsr,
                options(nostack)
            )
        };
        b[512..520].copy_from_slice(&fpcr.to_le_bytes());
        b[520..528].copy_from_slice(&fpsr.to_le_bytes());
        out[..HEADER].copy_from_slice(&FP_HEADER.encode());
        out[HEADER..].copy_from_slice(&b);
    }

    fn general_sp_ret(buf: &[u8]) -> (u64, u64) {
        let g = general(buf).expect("velikost overena");
        (g.sp, g.x[0])
    }

    fn prepare_handler_call(buf: &mut [u8], entry: u64, frame: u64) -> Option<u64> {
        let mut g = general(buf)?;
        g.pc = entry;
        g.x[0] = frame;
        g.x[30] = 0;
        g.sp = frame;
        buf.copy_from_slice(&g.encode());
        None
    }

    unsafe fn load_fp(buf: &[u8]) {
        let buf = &buf[HEADER..];
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
