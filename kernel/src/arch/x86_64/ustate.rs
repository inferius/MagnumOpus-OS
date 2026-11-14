//! Kontext uzivatelskeho vlakna x86_64 (`api::UserState`).
//!
//! Ramec registru je na vrcholu zasobniku jadra vlakna (syscall i preruseni
//! z userspace ukladaji stejny `TrapFrame`; TSS.RSP0 i zasobnik syscallu =
//! vrchol, zarovnany na 16 B). Stav FPU/SSE je obraz FXSAVE v `Context`.

use core::arch::asm;

use abi::thread::{X64_FP_MXCSR, X64_FP_SIZE, X64General};
use abi::vm::USER_MAX;

use super::X86_64;
use super::context::Context;
use super::idt::TrapFrame;
use crate::arch::api::UserState;

/// Uzivatelske bity RFLAGS: CF, PF, AF, ZF, SF, TF, DF, OF, AC, ID.
const RFLAGS_USER: u64 = 0x1 | 0x4 | 0x10 | 0x40 | 0x80 | 0x100 | 0x400 | 0x800 | 0x4_0000 | 0x20_0000;
/// Vzdy nastavene: rezervovany bit 1 a IF.
const RFLAGS_FIXED: u64 = 0x2 | 0x200;

fn frame(stack_top: u64) -> *mut TrapFrame {
    (stack_top - size_of::<TrapFrame>() as u64) as *mut TrapFrame
}

fn general(buf: &[u8]) -> Option<X64General> {
    Some(X64General::decode(buf.try_into().ok()?))
}

/// Zarovnany buffer pro FXRSTOR64.
#[repr(C, align(16))]
struct Fx([u8; X64_FP_SIZE]);

impl UserState for X86_64 {
    const GENERAL_SIZE: usize = X64General::SIZE;
    const FP_SIZE: usize = X64_FP_SIZE;

    unsafe fn read_general(stack_top: u64, tp: u64, out: &mut [u8]) {
        // SAFETY: predano volajicim (ramec stojiciho vlakna).
        let f = unsafe { &*frame(stack_top) };
        let g = X64General {
            rax: f.rax,
            rbx: f.rbx,
            rcx: f.rcx,
            rdx: f.rdx,
            rsi: f.rsi,
            rdi: f.rdi,
            rbp: f.rbp,
            rsp: f.rsp,
            r8: f.r8,
            r9: f.r9,
            r10: f.r10,
            r11: f.r11,
            r12: f.r12,
            r13: f.r13,
            r14: f.r14,
            r15: f.r15,
            rip: f.rip,
            rflags: f.rflags,
            fs_base: tp,
            gs_base: 0,
        };
        out.copy_from_slice(&g.encode());
    }

    fn check_general(buf: &[u8]) -> bool {
        // RIP, RSP a FS baze kanonicke v userspace: jinak by iretq nebo
        // zapis FS_BASE vyvolaly vyjimku jeste v jadre.
        general(buf).is_some_and(|g| g.rip < USER_MAX && g.rsp <= USER_MAX && g.fs_base < USER_MAX)
    }

    unsafe fn write_general(stack_top: u64, buf: &[u8]) -> u64 {
        let g = general(buf).expect("overeno v check_general");
        // SAFETY: predano volajicim (vlastni ramec, zakazana preruseni).
        let f = unsafe { &mut *frame(stack_top) };
        f.rax = g.rax;
        f.rbx = g.rbx;
        f.rcx = g.rcx;
        f.rdx = g.rdx;
        f.rsi = g.rsi;
        f.rdi = g.rdi;
        f.rbp = g.rbp;
        f.rsp = g.rsp;
        f.r8 = g.r8;
        f.r9 = g.r9;
        f.r10 = g.r10;
        f.r11 = g.r11;
        f.r12 = g.r12;
        f.r13 = g.r13;
        f.r14 = g.r14;
        f.r15 = g.r15;
        f.rip = g.rip;
        // CS a SS se nemeni; z priznaku jen uzivatelske bity.
        f.rflags = (g.rflags & RFLAGS_USER) | RFLAGS_FIXED;
        g.fs_base
    }

    fn read_fp(ctx: &Context, out: &mut [u8]) {
        out.copy_from_slice(ctx.fpu_bytes());
    }

    fn check_fp(buf: &[u8]) -> bool {
        // Rezervovane bity MXCSR = #GP ve FXRSTOR (v jadre).
        buf.len() == X64_FP_SIZE
            && u32::from_le_bytes(buf[X64_FP_MXCSR..X64_FP_MXCSR + 4].try_into().unwrap()) & 0xFFFF_0000 == 0
    }

    const FRAME_SIZE: usize = abi::exception::frame::X64_SIZE;
    const FRAME_FP: usize = abi::exception::frame::X64_FP;

    unsafe fn save_fp(out: &mut [u8]) {
        let mut fx = Fx([0; X64_FP_SIZE]);
        // SAFETY: zarovnany buffer 512 B; FXSAVE jen cte registry FPU/SSE.
        unsafe { asm!("fxsave64 [{}]", in(reg) fx.0.as_mut_ptr(), options(nostack)) };
        out.copy_from_slice(&fx.0);
    }

    fn general_sp_ret(buf: &[u8]) -> (u64, u64) {
        let g = general(buf).expect("velikost overena");
        (g.rsp, g.rax)
    }

    fn prepare_handler_call(buf: &mut [u8], entry: u64, frame: u64) -> Option<u64> {
        let mut g = general(buf)?;
        g.rip = entry;
        g.rdi = frame;
        // Jako po `call`: rsp % 16 == 8, na [rsp] navratova adresa 0.
        g.rsp = frame - 8;
        buf.copy_from_slice(&g.encode());
        Some(frame - 8)
    }

    unsafe fn load_fp(buf: &[u8]) {
        let mut fx = Fx([0; X64_FP_SIZE]);
        fx.0.copy_from_slice(buf);
        // SAFETY: obraz FXSAVE prosel `check_fp` (MXCSR bez rezervovanych
        // bitu), buffer zarovnany na 16 B. Jadro FPU/SSE nepouziva, stav
        // patri vlaknu, ktere se vraci do userspace.
        unsafe { asm!("fxrstor64 [{}]", in(reg) fx.0.as_ptr(), options(nostack, readonly)) };
    }
}
