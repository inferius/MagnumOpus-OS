//! Testy `ucontext_t` (rozlozeni jako Linux, shoda s hlavickou, FP aarch64).

use core::mem::{offset_of, size_of};

use crate::ucontext::{a64, x64};

#[test]
fn x64_layout_like_glibc() {
    assert_eq!(size_of::<x64::ucontext_t>(), x64::SIZE);
    assert_eq!(offset_of!(x64::ucontext_t, uc_stack), 16);
    assert_eq!(offset_of!(x64::ucontext_t, uc_mcontext), 40);
    assert_eq!(size_of::<x64::mcontext_t>(), 256);
    assert_eq!(offset_of!(x64::mcontext_t, fpregs), 184);
    assert_eq!(offset_of!(x64::ucontext_t, uc_sigmask), 296);
    assert_eq!(offset_of!(x64::ucontext_t, __fpregs_mem), 424);
}

#[test]
fn a64_layout_like_glibc() {
    assert_eq!(offset_of!(a64::ucontext_t, uc_sigmask), 40);
    assert_eq!(offset_of!(a64::ucontext_t, uc_mcontext), 176);
    assert_eq!(offset_of!(a64::mcontext_t, regs), 8);
    assert_eq!(offset_of!(a64::mcontext_t, pc), 264);
    assert_eq!(offset_of!(a64::mcontext_t, __reserved), 288);
    assert_eq!(size_of::<a64::ucontext_t>(), a64::SIZE);
}

#[test]
fn header_matches() {
    let h = include_str!("../include/sys/ucontext.h");
    for (n, v) in [("REG_R8", x64::REG_R8), ("REG_RAX", x64::REG_RAX), ("REG_RSP", x64::REG_RSP), ("REG_RIP", x64::REG_RIP), ("REG_EFL", x64::REG_EFL), ("REG_CR2", x64::REG_CR2)] {
        assert!(h.contains(&format!("    {n} = {v}")), "{n}");
    }
    assert!(h.contains(&format!("#define NGREG {}", x64::NGREG)));
    assert!(h.contains(&format!("#define FPSIMD_MAGIC {:#x}", a64::FPSIMD_MAGIC)));
}

#[test]
fn fpsimd_roundtrip() {
    let mut fp = vec![0u8; 528];
    for (i, b) in fp.iter_mut().enumerate().take(512) {
        *b = i as u8;
    }
    fp[512..520].copy_from_slice(&0x0300_0000u64.to_le_bytes());
    fp[520..528].copy_from_slice(&0x9u64.to_le_bytes());
    let mut res = [0u8; 4096];
    crate::ucontext::write_fpsimd(&mut res, &fp);
    assert_eq!(&res[0..4], &a64::FPSIMD_MAGIC.to_le_bytes());
    assert_eq!(u32::from_le_bytes(res[4..8].try_into().unwrap()), a64::FPSIMD_SIZE);
    assert_eq!(u32::from_le_bytes(res[8..12].try_into().unwrap()), 0x9, "fpsr");
    assert_eq!(u32::from_le_bytes(res[12..16].try_into().unwrap()), 0x0300_0000, "fpcr");
    // Obsluha zmeni V0 a FPCR, zpet do ramce.
    res[16] = 0xEE;
    res[12..16].copy_from_slice(&0x0040_0000u32.to_le_bytes());
    let mut back = vec![0u8; 528];
    crate::ucontext::read_fpsimd(&res, &mut back);
    assert_eq!(back[0], 0xEE);
    assert_eq!(&back[1..512], &fp[1..512]);
    assert_eq!(u64::from_le_bytes(back[512..520].try_into().unwrap()), 0x0040_0000);
    assert_eq!(u64::from_le_bytes(back[520..528].try_into().unwrap()), 0x9);
}
