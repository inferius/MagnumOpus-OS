//! Kontext vlakna pro `THREAD_STATE`: obecne registry a stav FPU/SIMD.
//!
//! Struktury jsou pro kazdou architekturu zvlast (jadro prijme jen svou);
//! vsechny little endian, bez vyplne. Pouzivaji je debuggery, doruceni
//! vyjimek (SEH ve Windows vrstve, signaly v cstd) a JIT.

/// Operace `THREAD_STATE`.
pub mod op {
    pub const READ: u32 = 0;
    pub const WRITE: u32 = 1;
}

/// Druh stavu `THREAD_STATE`.
pub mod kind {
    /// Obecne registry, PC, SP, priznaky a baze TLS
    /// ([`X64General`] / [`A64General`]).
    pub const GENERAL: u32 = 0;
    /// FPU/SIMD: x86_64 obraz FXSAVE (512 B), aarch64 [`A64Fp`] (528 B).
    pub const FP: u32 = 1;
}

/// Obecne registry x86_64 (160 B). Pri zapisu: `rip`, `rsp` a `fs_base`
/// musi lezet v pameti procesu (kanonicke, pod `vm::USER_MAX`), z `rflags`
/// se prevezmou jen uzivatelske bity (CF, PF, AF, ZF, SF, TF, DF, OF, AC,
/// ID); IF je vzdy 1, IOPL 0.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct X64General {
    pub rax: u64,
    pub rbx: u64,
    pub rcx: u64,
    pub rdx: u64,
    pub rsi: u64,
    pub rdi: u64,
    pub rbp: u64,
    pub rsp: u64,
    pub r8: u64,
    pub r9: u64,
    pub r10: u64,
    pub r11: u64,
    pub r12: u64,
    pub r13: u64,
    pub r14: u64,
    pub r15: u64,
    pub rip: u64,
    pub rflags: u64,
    pub fs_base: u64,
    /// Rezerva (GS baze pro TEB Windows vrstvy - zatim 0, zapis ignorovan).
    pub gs_base: u64,
}

impl X64General {
    pub const SIZE: usize = 160;

    fn fields(&self) -> [u64; 20] {
        [
            self.rax, self.rbx, self.rcx, self.rdx, self.rsi, self.rdi, self.rbp, self.rsp, self.r8, self.r9,
            self.r10, self.r11, self.r12, self.r13, self.r14, self.r15, self.rip, self.rflags, self.fs_base,
            self.gs_base,
        ]
    }

    pub fn encode(&self) -> [u8; Self::SIZE] {
        let mut b = [0u8; Self::SIZE];
        for (i, v) in self.fields().iter().enumerate() {
            b[i * 8..i * 8 + 8].copy_from_slice(&v.to_le_bytes());
        }
        b
    }

    pub fn decode(b: &[u8; Self::SIZE]) -> Self {
        let v = |i: usize| u64::from_le_bytes(b[i * 8..i * 8 + 8].try_into().unwrap());
        Self {
            rax: v(0),
            rbx: v(1),
            rcx: v(2),
            rdx: v(3),
            rsi: v(4),
            rdi: v(5),
            rbp: v(6),
            rsp: v(7),
            r8: v(8),
            r9: v(9),
            r10: v(10),
            r11: v(11),
            r12: v(12),
            r13: v(13),
            r14: v(14),
            r15: v(15),
            rip: v(16),
            rflags: v(17),
            fs_base: v(18),
            gs_base: v(19),
        }
    }
}

/// Obraz FXSAVE x86_64 (512 B): x87, MMX, XMM0-15, MXCSR. Pri zapisu nesmi
/// mit MXCSR nastavene rezervovane bity (16-31).
pub const X64_FP_SIZE: usize = 512;
/// Offset MXCSR v obrazu FXSAVE.
pub const X64_FP_MXCSR: usize = 24;

/// Obecne registry aarch64 (280 B). Pri zapisu: `pc`, `sp` a `tpidr` musi
/// lezet v pameti procesu (pod `vm::USER_MAX`), z `pstate` se prevezmou jen
/// priznaky NZCV (bity 28-31); rezim zustava EL0.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct A64General {
    pub x: [u64; 31],
    pub sp: u64,
    pub pc: u64,
    pub pstate: u64,
    pub tpidr: u64,
}

impl Default for A64General {
    fn default() -> Self {
        Self { x: [0; 31], sp: 0, pc: 0, pstate: 0, tpidr: 0 }
    }
}

impl A64General {
    pub const SIZE: usize = 280;

    pub fn encode(&self) -> [u8; Self::SIZE] {
        let mut b = [0u8; Self::SIZE];
        let mut put = |i: usize, v: u64| b[i * 8..i * 8 + 8].copy_from_slice(&v.to_le_bytes());
        for (i, v) in self.x.iter().enumerate() {
            put(i, *v);
        }
        put(31, self.sp);
        put(32, self.pc);
        put(33, self.pstate);
        put(34, self.tpidr);
        b
    }

    pub fn decode(b: &[u8; Self::SIZE]) -> Self {
        let v = |i: usize| u64::from_le_bytes(b[i * 8..i * 8 + 8].try_into().unwrap());
        let mut x = [0u64; 31];
        for (i, r) in x.iter_mut().enumerate() {
            *r = v(i);
        }
        Self { x, sp: v(31), pc: v(32), pstate: v(33), tpidr: v(34) }
    }
}

/// Stav FPU/SIMD aarch64 (528 B): V0-V31 (po 16 B), pak FPCR a FPSR (u64).
pub const A64_FP_SIZE: usize = 528;
/// Bity FPCR, ktere smi program nastavit (AHP, DN, FZ, RMode, FZ16, IDE,
/// IXE, UFE, OFE, DZE, IOE); ostatni musi byt 0.
pub const A64_FPCR_MASK: u64 = 0x07C8_9F00;
/// Bity FPSR (N, Z, C, V, QC, IDC, IXC, UFC, OFC, DZC, IOC).
pub const A64_FPSR_MASK: u64 = 0xF800_009F;
