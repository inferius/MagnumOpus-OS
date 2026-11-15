//! `ucontext_t` pro obsluhy signalu se `SA_SIGINFO` (`sys/ucontext.h`):
//! registry v okamziku vyjimky z ramce obsluhy ve vlaknu (`osrt::exception`),
//! po navratu z obsluhy se zmenene registry pouziji (presmerovani PC, zmena
//! SP, registru, FPU - Wine, JIT, GC, ladici nastroje).
//!
//! Rozlozeni je jako Linux (glibc), aby prevzaty kod pristupoval k polim
//! beze zmen: x86_64 `uc_mcontext.gregs[REG_*]` a `fpregs` (obraz FXSAVE),
//! aarch64 `uc_mcontext.regs/sp/pc/pstate` a zaznam `fpsimd_context`
//! v `__reserved`.

use core::ffi::c_void;

use crate::signal::Set;

/// `stack_t` (24 B, jako `signal.h`).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct stack_t {
    pub ss_sp: usize,
    pub ss_flags: i32,
    pub _pad: i32,
    pub ss_size: usize,
}

#[cfg(target_arch = "x86_64")]
pub use x64::*;

#[cfg(target_arch = "aarch64")]
pub use a64::*;

/// Rozlozeni x86_64 (glibc `sys/ucontext.h`).
#[cfg_attr(not(target_arch = "x86_64"), allow(dead_code))]
pub mod x64 {
    use super::{Set, stack_t};

    pub const NGREG: usize = 23;
    pub const REG_R8: usize = 0;
    pub const REG_R9: usize = 1;
    pub const REG_R10: usize = 2;
    pub const REG_R11: usize = 3;
    pub const REG_R12: usize = 4;
    pub const REG_R13: usize = 5;
    pub const REG_R14: usize = 6;
    pub const REG_R15: usize = 7;
    pub const REG_RDI: usize = 8;
    pub const REG_RSI: usize = 9;
    pub const REG_RBP: usize = 10;
    pub const REG_RBX: usize = 11;
    pub const REG_RDX: usize = 12;
    pub const REG_RAX: usize = 13;
    pub const REG_RCX: usize = 14;
    pub const REG_RSP: usize = 15;
    pub const REG_RIP: usize = 16;
    pub const REG_EFL: usize = 17;
    pub const REG_CSGSFS: usize = 18;
    pub const REG_ERR: usize = 19;
    pub const REG_TRAPNO: usize = 20;
    pub const REG_OLDMASK: usize = 21;
    pub const REG_CR2: usize = 22;

    /// `mcontext_t` (256 B).
    #[repr(C)]
    #[derive(Clone, Copy, Debug)]
    pub struct mcontext_t {
        pub gregs: [i64; NGREG],
        /// Obraz FXSAVE (512 B, `struct _libc_fpstate`): ukazuje na
        /// `__fpregs_mem` tohoto kontextu.
        pub fpregs: *mut u8,
        pub _reserved1: [u64; 8],
    }

    /// `ucontext_t` (968 B).
    #[repr(C)]
    #[derive(Clone, Copy, Debug)]
    pub struct ucontext_t {
        pub uc_flags: u64,
        pub uc_link: *mut ucontext_t,
        pub uc_stack: stack_t,
        pub uc_mcontext: mcontext_t,
        pub uc_sigmask: Set,
        pub __fpregs_mem: [u8; 512],
        pub __ssp: [u64; 4],
    }

    pub const SIZE: usize = 968;
}

/// Rozlozeni aarch64 (glibc `sys/ucontext.h`, Linux `sigcontext`).
#[cfg_attr(not(target_arch = "aarch64"), allow(dead_code))]
pub mod a64 {
    use super::{Set, stack_t};

    /// Zaznam FP v `__reserved` (Linux `FPSIMD_MAGIC`).
    pub const FPSIMD_MAGIC: u32 = 0x4650_8001;
    /// Velikost zaznamu `fpsimd_context` (hlavicka 8 + FPSR, FPCR 8 + V0-V31).
    pub const FPSIMD_SIZE: u32 = 528;

    /// `mcontext_t` = Linux `struct sigcontext` (zarovnani 16).
    #[repr(C, align(16))]
    #[derive(Clone, Copy, Debug)]
    pub struct mcontext_t {
        pub fault_address: u64,
        pub regs: [u64; 31],
        pub sp: u64,
        pub pc: u64,
        pub pstate: u64,
        /// Zarovnani `__reserved` na 16 (v glibc atribut `aligned(16)`).
        pub _pad: u64,
        /// Zaznamy rozsireni: `fpsimd_context`, pak koncovy zaznam (0, 0).
        pub __reserved: [u8; 4096],
    }

    /// `ucontext_t`.
    #[repr(C, align(16))]
    #[derive(Clone, Copy, Debug)]
    pub struct ucontext_t {
        pub uc_flags: u64,
        pub uc_link: *mut ucontext_t,
        pub uc_stack: stack_t,
        pub uc_sigmask: Set,
        pub uc_mcontext: mcontext_t,
    }

    pub const SIZE: usize = 4560;
}

/// Prazdny kontext (signal bez vyjimky: `raise`, casovac).
pub fn empty() -> Box<ucontext_t> {
    // SAFETY: vsechna pole jsou cisla nebo ukazatele, nuly jsou platne.
    let mut uc: Box<ucontext_t> = Box::new(unsafe { core::mem::zeroed() });
    #[cfg(target_arch = "x86_64")]
    {
        uc.uc_mcontext.fpregs = uc.__fpregs_mem.as_mut_ptr();
    }
    let _ = &mut uc;
    uc
}

/// Kontext z ramce vyjimky (registry, FPU, adresa vypadku).
#[cfg(target_os = "magnumopus")]
pub fn from_frame(f: &std::os::magnumopus::runtime::exception::Frame, mask: &Set) -> Box<ucontext_t> {
    let mut uc = empty();
    uc.uc_sigmask = *mask;
    let g = f.general();
    let fp = f.fp();
    #[cfg(target_arch = "x86_64")]
    {
        let r = &mut uc.uc_mcontext.gregs;
        for (i, v) in [
            (REG_R8, g.r8), (REG_R9, g.r9), (REG_R10, g.r10), (REG_R11, g.r11), (REG_R12, g.r12),
            (REG_R13, g.r13), (REG_R14, g.r14), (REG_R15, g.r15), (REG_RDI, g.rdi), (REG_RSI, g.rsi),
            (REG_RBP, g.rbp), (REG_RBX, g.rbx), (REG_RDX, g.rdx), (REG_RAX, g.rax), (REG_RCX, g.rcx),
            (REG_RSP, g.rsp), (REG_RIP, g.rip), (REG_EFL, g.rflags), (REG_CR2, f.report().addr),
        ] {
            r[i] = v as i64;
        }
        let n = fp.len().min(512);
        uc.__fpregs_mem[..n].copy_from_slice(&fp[..n]);
    }
    #[cfg(target_arch = "aarch64")]
    {
        let m = &mut uc.uc_mcontext;
        m.fault_address = f.report().addr;
        m.regs = g.x;
        m.sp = g.sp;
        m.pc = g.pc;
        m.pstate = g.pstate;
        write_fpsimd(&mut m.__reserved, fp);
    }
    uc
}

/// Zapise zmeny kontextu zpet do ramce (pouziji se navratem z obsluhy).
#[cfg(target_os = "magnumopus")]
pub fn to_frame(uc: &ucontext_t, f: &mut std::os::magnumopus::runtime::exception::Frame) {
    let mut g = f.general();
    #[cfg(target_arch = "x86_64")]
    {
        let r = &uc.uc_mcontext.gregs;
        let v = |i: usize| r[i] as u64;
        g.r8 = v(REG_R8);
        g.r9 = v(REG_R9);
        g.r10 = v(REG_R10);
        g.r11 = v(REG_R11);
        g.r12 = v(REG_R12);
        g.r13 = v(REG_R13);
        g.r14 = v(REG_R14);
        g.r15 = v(REG_R15);
        g.rdi = v(REG_RDI);
        g.rsi = v(REG_RSI);
        g.rbp = v(REG_RBP);
        g.rbx = v(REG_RBX);
        g.rdx = v(REG_RDX);
        g.rax = v(REG_RAX);
        g.rcx = v(REG_RCX);
        g.rsp = v(REG_RSP);
        g.rip = v(REG_RIP);
        g.rflags = v(REG_EFL);
        f.set_general(&g);
        // FPU: obsluha mohla zmenit obraz FXSAVE (pres `fpregs` nebo primo).
        let src = if uc.uc_mcontext.fpregs.is_null() {
            &uc.__fpregs_mem[..]
        } else {
            // SAFETY: `fpregs` ukazuje na obraz FXSAVE (512 B) - nas
            // `__fpregs_mem`, nebo pamet, kterou tam dosadila obsluha.
            unsafe { core::slice::from_raw_parts(uc.uc_mcontext.fpregs, 512) }
        };
        let fp = f.fp_mut();
        let n = fp.len().min(512);
        fp[..n].copy_from_slice(&src[..n]);
    }
    #[cfg(target_arch = "aarch64")]
    {
        let m = &uc.uc_mcontext;
        g.x = m.regs;
        g.sp = m.sp;
        g.pc = m.pc;
        g.pstate = m.pstate;
        f.set_general(&g);
        read_fpsimd(&m.__reserved, f.fp_mut());
    }
}

/// Zaznam `fpsimd_context` do `__reserved` z dat FP ramce (V0-V31 po 16 B,
/// pak FPCR a FPSR po 8 B, `osrt::exception::Frame::fp`).
pub fn write_fpsimd(res: &mut [u8; 4096], fp: &[u8]) {
    res[0..4].copy_from_slice(&a64::FPSIMD_MAGIC.to_le_bytes());
    res[4..8].copy_from_slice(&a64::FPSIMD_SIZE.to_le_bytes());
    let word = |o: usize| fp.get(o..o + 8).map_or(0u64, |b| u64::from_le_bytes(b.try_into().unwrap()));
    let (fpcr, fpsr) = (word(512) as u32, word(520) as u32);
    res[8..12].copy_from_slice(&fpsr.to_le_bytes());
    res[12..16].copy_from_slice(&fpcr.to_le_bytes());
    let n = fp.len().min(512);
    res[16..16 + n].copy_from_slice(&fp[..n]);
    // Koncovy zaznam (magic 0, size 0) hned za fpsimd.
    res[528..536].fill(0);
}

/// Opak [`write_fpsimd`]: zmeny V0-V31, FPCR, FPSR zpet do dat FP ramce.
pub fn read_fpsimd(res: &[u8; 4096], fp: &mut [u8]) {
    if res[0..4] != a64::FPSIMD_MAGIC.to_le_bytes() {
        return;
    }
    let n = fp.len().min(512);
    fp[..n].copy_from_slice(&res[16..16 + n]);
    let fpsr = u32::from_le_bytes(res[8..12].try_into().unwrap());
    let fpcr = u32::from_le_bytes(res[12..16].try_into().unwrap());
    if fp.len() >= 528 {
        fp[512..520].copy_from_slice(&u64::from(fpcr).to_le_bytes());
        fp[520..528].copy_from_slice(&u64::from(fpsr).to_le_bytes());
    }
}

/// Ukazatel na kontext pro obsluhu (`void *` treti argument).
pub fn as_arg(uc: &mut ucontext_t) -> *mut c_void {
    (uc as *mut ucontext_t).cast()
}
