//! Vstup zavadece a samorelokace (static-PIE).
//!
//! Zavadec je slinkovany jako PIE bez zavadece (`-pie --no-dynamic-linker`):
//! obsahuje jen relativni relokace (RELA `R_*_RELATIVE` a RELR). Nez se
//! provedou, kod nesmi sahnout na data s ukazateli (vtabulky, staticke
//! ukazatele) - proto vstup v asembleru a [`relocate_self`] jen
//! s cistymi cteni a zapisy ukazateli.

use core::arch::naked_asm;

/// Vstup: `rdi`/`x0` = handle bootstrapu, SP = argc a auxv (`abi::auxv`).
/// Spocita bazi (`__ehdr_start`) a adresu `_DYNAMIC` relativne k PC,
/// zrelokuje se, zavola [`crate::load::main`] a skoci na vstup programu se
/// stejnym SP a handlem bootstrapu.
#[cfg(target_arch = "x86_64")]
#[unsafe(naked)]
#[unsafe(no_mangle)]
extern "C" fn _start() -> ! {
    naked_asm!(
        "mov r12, rdi",
        "mov r13, rsp",
        "xor ebp, ebp",
        "lea rdi, [rip + __ehdr_start]",
        "lea rsi, [rip + _DYNAMIC]",
        "and rsp, -16",
        "call {reloc}",
        "mov rdi, r13",
        "mov rsi, r12",
        "call {main}",
        "mov rsp, r13",
        "mov rdi, r12",
        "xor ebp, ebp",
        "jmp rax",
        reloc = sym relocate_self,
        main = sym crate::load::main,
    )
}

#[cfg(target_arch = "aarch64")]
#[unsafe(naked)]
#[unsafe(no_mangle)]
extern "C" fn _start() -> ! {
    naked_asm!(
        "mov x19, x0",
        "mov x20, sp",
        "mov x29, xzr",
        "mov x30, xzr",
        "adrp x0, __ehdr_start",
        "add x0, x0, :lo12:__ehdr_start",
        "adrp x1, _DYNAMIC",
        "add x1, x1, :lo12:_DYNAMIC",
        "bl {reloc}",
        "mov x0, x20",
        "mov x1, x19",
        "bl {main}",
        "mov x16, x0",
        "mov sp, x20",
        "mov x0, x19",
        "mov x29, xzr",
        "mov x30, xzr",
        "br x16",
        reloc = sym relocate_self,
        main = sym crate::load::main,
    )
}

/// Typ relativni relokace teto architektury.
#[cfg(target_arch = "x86_64")]
const R_RELATIVE: u64 = 8;
#[cfg(target_arch = "aarch64")]
const R_RELATIVE: u64 = 1027;

/// Provede relativni relokace zavadece (RELA a RELR z `_DYNAMIC`).
///
/// # Safety
///
/// Vola jen `_start`: `base` je baze zavadece, `dynamic` jeho `_DYNAMIC`;
/// pamet zavadece je namapovana a data jsou zapisovatelna.
extern "C" fn relocate_self(base: u64, dynamic: *const u64) {
    let (mut rela, mut relasz, mut relr, mut relrsz) = (0u64, 0u64, 0u64, 0u64);
    let mut d = dynamic;
    // SAFETY: `_DYNAMIC` je pole (tag, hodnota) zakoncene DT_NULL
    // (vytvoril ho linker zavadece); ctou a pisou se jen adrese uvnitr
    // zavadece podle jeho vlastnich tabulek.
    unsafe {
        loop {
            let tag = d.read();
            let val = d.add(1).read();
            if tag == 0 {
                break;
            }
            if tag == 7 {
                rela = val;
            } else if tag == 8 {
                relasz = val;
            } else if tag == 36 {
                relr = val;
            } else if tag == 35 {
                relrsz = val;
            }
            d = d.add(2);
        }
        let mut p = base + rela;
        let end = p + relasz;
        while p < end {
            let off = (p as *const u64).read();
            let info = ((p + 8) as *const u64).read();
            let add = ((p + 16) as *const u64).read();
            if info & 0xFFFF_FFFF == R_RELATIVE {
                ((base + off) as *mut u64).write(base.wrapping_add(add));
            }
            p += 24;
        }
        // RELR: sude slovo = adresa, liche = bitmapa 63 dalsich slov.
        let mut q = base + relr;
        let qend = q + relrsz;
        let mut at = 0u64;
        while q < qend {
            let e = (q as *const u64).read();
            q += 8;
            if e & 1 == 0 {
                at = base + e;
                let w = at as *mut u64;
                w.write(w.read().wrapping_add(base));
                at += 8;
            } else {
                let mut bits = e >> 1;
                let mut i = 0u64;
                while bits != 0 {
                    if bits & 1 != 0 {
                        let w = (at + i * 8) as *mut u64;
                        w.write(w.read().wrapping_add(base));
                    }
                    bits >>= 1;
                    i += 1;
                }
                at += 63 * 8;
            }
        }
    }
}
