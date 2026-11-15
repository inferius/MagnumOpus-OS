//! Kontext uzivatelskeho vlakna x86_64 (`api::UserState`).
//!
//! Ramec registru je na vrcholu zasobniku jadra vlakna (syscall i preruseni
//! z userspace ukladaji stejny `TrapFrame`; TSS.RSP0 i zasobnik syscallu =
//! vrchol, zarovnany na 16 B). Stav FPU je obraz XSAVE (nebo FXSAVE na CPU
//! bez XSAVE) v `Context`; ven jde s hlavickou `abi::thread::fp::Header`.

use abi::thread::fp::{HEADER, Header, format};
use abi::thread::{X64_FP_MXCSR, X64_XSAVE_HEADER, X64General};
use abi::vm::USER_MAX;

use super::X86_64;
use super::context::{self, Context, FpuArea};
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

/// Hlavicka stavu FPU tohoto CPU.
fn header() -> Header {
    let xcr0 = context::xcr0();
    // Bez XSAVE (Nehalem, Westmere): obraz FXSAVE = casti x87 a SSE.
    let (format, features) = if xcr0 != 0 { (format::X64_XSAVE, xcr0) } else { (format::X64_FXSAVE, 0b11) };
    Header { format, len: context::fp_len() as u32, features }
}

/// Je obraz XSAVE/FXSAVE (bez hlavicky) bezpecny pro XRSTOR/FXRSTOR?
/// Rezervovane bity MXCSR, XSTATE_BV mimo XCR0, kompaktni format nebo
/// nenulova rezerva hlavicky XSAVE = #GP v jadre.
fn check_image(data: &[u8]) -> bool {
    let u64_at = |i: usize| u64::from_le_bytes(data[i..i + 8].try_into().unwrap());
    if u32::from_le_bytes(data[X64_FP_MXCSR..X64_FP_MXCSR + 4].try_into().unwrap()) & 0xFFFF_0000 != 0 {
        return false;
    }
    let xcr0 = context::xcr0();
    if xcr0 == 0 {
        return true;
    }
    let h = X64_XSAVE_HEADER;
    u64_at(h) & !xcr0 == 0 && u64_at(h + 8) == 0 && data[h + 16..h + 64].iter().all(|&b| b == 0)
}

impl UserState for X86_64 {
    const GENERAL_SIZE: usize = X64General::SIZE;
    fn fp_size() -> usize {
        HEADER + context::fp_len()
    }

    fn fp_header() -> Header {
        header()
    }

    unsafe fn read_general(stack_top: u64, tp: u64, tp2: u64, out: &mut [u8]) {
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
            gs_base: tp2,
        };
        out.copy_from_slice(&g.encode());
    }

    fn check_general(buf: &[u8]) -> bool {
        // RIP, RSP, baze FS a GS kanonicke v userspace: jinak by iretq nebo
        // zapis FS_BASE / KERNEL_GS_BASE vyvolaly vyjimku jeste v jadre.
        general(buf).is_some_and(|g| g.rip < USER_MAX && g.rsp <= USER_MAX && g.fs_base < USER_MAX && g.gs_base < USER_MAX)
    }

    unsafe fn write_general(stack_top: u64, buf: &[u8]) -> (u64, u64) {
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
        (g.fs_base, g.gs_base)
    }

    fn read_fp(ctx: &Context, out: &mut [u8]) {
        out[..HEADER].copy_from_slice(&header().encode());
        out[HEADER..].copy_from_slice(ctx.fpu_bytes());
    }

    fn check_fp(buf: &[u8]) -> bool {
        buf.len() == Self::fp_size() && Header::decode(buf) == Some(header()) && check_image(&buf[HEADER..])
    }

    const FRAME_FP: usize = abi::exception::frame::X64_FP;

    unsafe fn save_fp(out: &mut [u8]) {
        let mut a = FpuArea::zeroed(context::fp_len());
        // SAFETY: predano volajicim (registry FPU patri vlaknu).
        unsafe { a.save() };
        out[..HEADER].copy_from_slice(&header().encode());
        out[HEADER..].copy_from_slice(a.bytes());
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
        let mut a = FpuArea::zeroed(context::fp_len());
        a.bytes_mut().copy_from_slice(&buf[HEADER..]);
        // SAFETY: obraz prosel `check_fp` (hlavicka tohoto CPU, rezervovane
        // bity nulove), oblast zarovnana na 64 B. Jadro FPU nepouziva, stav
        // patri vlaknu, ktere se vraci do userspace.
        unsafe { a.load() };
    }
}
