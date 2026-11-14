//! `setjmp.h`: `setjmp`, `longjmp`, `sigsetjmp`, `siglongjmp`.
//!
//! `jmp_buf` ma 320 B na obou architekturach: registry, ktere musi
//! volany zachovat (x86_64: rbx, rbp, r12-r15, rsp, navratova adresa;
//! aarch64: x19-x30, sp, d8-d15), od offsetu `MASK_FLAG` priznak a od
//! `MASK` maska signalu (`sigsetjmp`). Maska se uklada pred `setjmp`
//! (makro `sigsetjmp` v hlavicce vola `__cstd_sigsave`), proto staci
//! asembler jen pro registry.

#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
use core::arch::naked_asm;

use crate::c_int;

/// Velikost `jmp_buf` v bajtech.
pub const JMP_BUF: usize = 320;
/// Offset priznaku ulozene masky.
pub const MASK_FLAG: usize = 176;
/// Offset ulozene masky (128 B `sigset_t`).
pub const MASK: usize = 184;

/// # Safety
///
/// Volano z C s platnym `jmp_buf`; vraci se dvakrat (C semantika).
#[cfg(target_arch = "x86_64")]
#[unsafe(naked)]
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn setjmp(_buf: *mut u8) -> c_int {
    naked_asm!(
        "mov [rdi], rbx",
        "mov [rdi + 8], rbp",
        "mov [rdi + 16], r12",
        "mov [rdi + 24], r13",
        "mov [rdi + 32], r14",
        "mov [rdi + 40], r15",
        "lea rdx, [rsp + 8]",
        "mov [rdi + 48], rdx",
        "mov rdx, [rsp]",
        "mov [rdi + 56], rdx",
        "xor eax, eax",
        "ret",
    )
}

/// # Safety
///
/// `buf` naplnil `setjmp` ve funkci, ktera jeste neskoncila.
#[cfg(target_arch = "x86_64")]
#[unsafe(naked)]
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn longjmp(_buf: *const u8, _val: c_int) -> ! {
    naked_asm!(
        "mov eax, esi",
        "test eax, eax",
        "jnz 2f",
        "inc eax",
        "2:",
        "mov rbx, [rdi]",
        "mov rbp, [rdi + 8]",
        "mov r12, [rdi + 16]",
        "mov r13, [rdi + 24]",
        "mov r14, [rdi + 32]",
        "mov r15, [rdi + 40]",
        "mov rsp, [rdi + 48]",
        "jmp qword ptr [rdi + 56]",
    )
}

/// # Safety
///
/// Volano z C s platnym `jmp_buf`; vraci se dvakrat (C semantika).
#[cfg(target_arch = "aarch64")]
#[unsafe(naked)]
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn setjmp(_buf: *mut u8) -> c_int {
    naked_asm!(
        "stp x19, x20, [x0, #0]",
        "stp x21, x22, [x0, #16]",
        "stp x23, x24, [x0, #32]",
        "stp x25, x26, [x0, #48]",
        "stp x27, x28, [x0, #64]",
        "stp x29, x30, [x0, #80]",
        "mov x2, sp",
        "str x2, [x0, #96]",
        "stp d8, d9, [x0, #104]",
        "stp d10, d11, [x0, #120]",
        "stp d12, d13, [x0, #136]",
        "stp d14, d15, [x0, #152]",
        "mov w0, #0",
        "ret",
    )
}

/// # Safety
///
/// `buf` naplnil `setjmp` ve funkci, ktera jeste neskoncila.
#[cfg(target_arch = "aarch64")]
#[unsafe(naked)]
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn longjmp(_buf: *const u8, _val: c_int) -> ! {
    naked_asm!(
        "ldp x19, x20, [x0, #0]",
        "ldp x21, x22, [x0, #16]",
        "ldp x23, x24, [x0, #32]",
        "ldp x25, x26, [x0, #48]",
        "ldp x27, x28, [x0, #64]",
        "ldp x29, x30, [x0, #80]",
        "ldr x2, [x0, #96]",
        "mov sp, x2",
        "ldp d8, d9, [x0, #104]",
        "ldp d10, d11, [x0, #120]",
        "ldp d12, d13, [x0, #136]",
        "ldp d14, d15, [x0, #152]",
        "cmp w1, #0",
        "csinc w0, w1, wzr, ne",
        // Navrat na adresu ze setjmp (x30); `ret` neni nepryma vetev pro BTI.
        "ret",
    )
}

/// Ulozi masku signalu do `buf` pred `setjmp` (makro `sigsetjmp`).
///
/// # Safety
///
/// `buf` je `sigjmp_buf` (320 B).
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn __cstd_sigsave(buf: *mut u8, save: c_int) {
    // SAFETY: buffer ma JMP_BUF bajtu.
    unsafe {
        buf.add(MASK_FLAG).cast::<u64>().write_unaligned(u64::from(save != 0));
        if save != 0 {
            let m = crate::signal::current_mask();
            buf.add(MASK).cast::<[u64; 16]>().write_unaligned(m);
        }
    }
}

/// `siglongjmp`: obnovi masku (byla-li ulozena) a skoci.
///
/// # Safety
///
/// Jako `longjmp`.
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn siglongjmp(buf: *const u8, val: c_int) -> ! {
    // SAFETY: buffer ma JMP_BUF bajtu.
    unsafe {
        if buf.add(MASK_FLAG).cast::<u64>().read_unaligned() != 0 {
            crate::signal::set_mask(buf.add(MASK).cast::<[u64; 16]>().read_unaligned());
        }
        longjmp(buf, val)
    }
}

/// `_setjmp`/`_longjmp` (bez masky) = `setjmp`/`longjmp`.
///
/// # Safety
///
/// Jako `longjmp`.
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn _longjmp(buf: *const u8, val: c_int) -> ! {
    // SAFETY: kontrakt C.
    unsafe { longjmp(buf, val) }
}
