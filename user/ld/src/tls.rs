//! TLS modulu: `__tls_get_addr` (x86_64 general/local-dynamic) pres DTV
//! a resitele deskriptoru TLSDESC (aarch64 vychozi, x86_64 GNU2).
//!
//! **DTV** (vektor dynamickeho TLS, model glibc): kazde vlakno ma pole
//! `[generace, adresa bloku modulu 1, 2, ...]`, ukazatel na nej lezi v TCB
//! (x86_64 `TP + 8`, aarch64 `TP + 0`). DTV ma pevnou velikost
//! ([`MAX_MODULES`]) a cisla modulu se neopakuji (moduly se neodstranuji),
//! takze prazdna polozka vzdy znamena "blok jeste neni". Moduly ve
//! statickem TLS (ze startu a z `dlopen` s initial-exec v rezerve) maji
//! polozku vyplnenou od zalozeni bloku (`iface::new_block`) nebo od
//! `dlopen` (vsechna zijici vlakna). Moduly z `dlopen` bez initial-exec
//! maji blok **liny**: alokuje se pri prvnim pristupu vlakna
//! ([`tls_get_addr`], pomala cesta) a uvolni pri konci vlakna.

use core::arch::naked_asm;
use core::sync::atomic::AtomicU64;

/// Nejvic modulu s cislem TLS (cislo modulu od 1).
pub const MAX_MODULES: usize = 256;

/// Velikost DTV v bajtech (generace a polozka na modul).
pub const DTV_BYTES: u64 = ((MAX_MODULES + 1) * 8) as u64;

/// Generace seznamu modulu s TLS (zvysi ji `dlopen` modulu s PT_TLS).
pub static GENERATION: AtomicU64 = AtomicU64::new(1);

/// Posun ukazatele na DTV v TCB.
pub const TCB_DTV: u64 = if cfg!(target_arch = "x86_64") { 8 } else { 0 };

/// Ukazatel vlakna.
pub fn tp() -> u64 {
    let v: u64;
    // SAFETY: cteni ukazatele vlakna; na x86_64 je na TP ukazatel sam na
    // sebe (TCB), nastavuje ho zavadec.
    #[cfg(target_arch = "x86_64")]
    unsafe {
        core::arch::asm!("mov {}, fs:[0]", out(reg) v, options(nostack, readonly, preserves_flags))
    };
    // SAFETY: cteni systemoveho registru EL0.
    #[cfg(target_arch = "aarch64")]
    unsafe {
        core::arch::asm!("mrs {}, tpidr_el0", out(reg) v, options(nostack, nomem, preserves_flags))
    };
    v
}

/// DTV vlakna s ukazatelem `tp`.
///
/// # Safety
///
/// `tp` je TP zijiciho bloku TLS (`iface::new_block`).
pub unsafe fn dtv_of(tp: u64) -> *mut u64 {
    // SAFETY: TCB bloku (predano volajicim).
    unsafe { ((tp + TCB_DTV) as *const u64).read() as *mut u64 }
}

/// `__tls_get_addr(&tls_index { modul, posun })`: blok modulu z DTV
/// vlakna + posun; prazdna polozka = lina alokace bloku (pomala cesta).
pub extern "C" fn tls_get_addr(ti: *const [u64; 2]) -> *mut u8 {
    // SAFETY: argument je `tls_index` z GOT modulu nebo deskriptoru
    // (2 slova, zapsal je zavadec); DTV vlakna zalozil `new_block`.
    unsafe {
        let [m, off] = ti.read();
        let dtv = dtv_of(tp());
        let mut block = if m != 0 && (m as usize) <= MAX_MODULES { dtv.add(m as usize).read() } else { 0 };
        if block == 0 {
            block = crate::iface::dynamic_block(m, dtv);
        }
        (block + off) as *mut u8
    }
}

/// Resitel statickeho deskriptoru TLS: vraci argument deskriptoru (posun
/// od TP). Vlastni ABI: meni jen `rax` / `x0` (a priznaky).
#[cfg(target_arch = "x86_64")]
#[unsafe(naked)]
pub extern "C" fn tlsdesc_static() {
    naked_asm!("mov rax, [rax + 8]", "ret")
}

#[cfg(target_arch = "aarch64")]
#[unsafe(naked)]
pub extern "C" fn tlsdesc_static() {
    naked_asm!("ldr x0, [x0, #8]", "ret")
}

/// Resitel dynamickeho deskriptoru TLS (modul z `dlopen` bez initial-exec):
/// argument deskriptoru ukazuje na `[cislo modulu, posun]`; vraci adresu
/// promenne minus TP. Rychla cesta jen cte DTV; pomala zavola
/// [`tls_get_addr`] a ulozi pritom vsechny registry, ktere smi Rust menit
/// (obecne i vektorove: volajici o volani nevi, ABI deskriptoru meni jen
/// `rax` a priznaky). Zavadec je prelozeny bez AVX, takze horni poloviny
/// `ymm`/`zmm` nemeni (staci `xmm0-15`).
#[cfg(target_arch = "x86_64")]
#[unsafe(naked)]
pub extern "C" fn tlsdesc_dynamic() {
    naked_asm!(
        "mov rax, [rax + 8]",
        "push rcx",
        "push rdx",
        "mov rdx, fs:[8]",
        "mov rcx, [rax]",
        "mov rdx, [rdx + rcx * 8]",
        "test rdx, rdx",
        "jz 2f",
        "add rdx, [rax + 8]",
        "mov rax, rdx",
        "sub rax, fs:[0]",
        "pop rdx",
        "pop rcx",
        "ret",
        "2:",
        "pop rdx",
        "pop rcx",
        "push rbp",
        "mov rbp, rsp",
        "push rcx",
        "push rdx",
        "push rsi",
        "push rdi",
        "push r8",
        "push r9",
        "push r10",
        "push r11",
        "and rsp, -16",
        "sub rsp, 256",
        "movdqu [rsp], xmm0",
        "movdqu [rsp + 16], xmm1",
        "movdqu [rsp + 32], xmm2",
        "movdqu [rsp + 48], xmm3",
        "movdqu [rsp + 64], xmm4",
        "movdqu [rsp + 80], xmm5",
        "movdqu [rsp + 96], xmm6",
        "movdqu [rsp + 112], xmm7",
        "movdqu [rsp + 128], xmm8",
        "movdqu [rsp + 144], xmm9",
        "movdqu [rsp + 160], xmm10",
        "movdqu [rsp + 176], xmm11",
        "movdqu [rsp + 192], xmm12",
        "movdqu [rsp + 208], xmm13",
        "movdqu [rsp + 224], xmm14",
        "movdqu [rsp + 240], xmm15",
        "mov rdi, rax",
        "call {slow}",
        "movdqu xmm0, [rsp]",
        "movdqu xmm1, [rsp + 16]",
        "movdqu xmm2, [rsp + 32]",
        "movdqu xmm3, [rsp + 48]",
        "movdqu xmm4, [rsp + 64]",
        "movdqu xmm5, [rsp + 80]",
        "movdqu xmm6, [rsp + 96]",
        "movdqu xmm7, [rsp + 112]",
        "movdqu xmm8, [rsp + 128]",
        "movdqu xmm9, [rsp + 144]",
        "movdqu xmm10, [rsp + 160]",
        "movdqu xmm11, [rsp + 176]",
        "movdqu xmm12, [rsp + 192]",
        "movdqu xmm13, [rsp + 208]",
        "movdqu xmm14, [rsp + 224]",
        "movdqu xmm15, [rsp + 240]",
        "lea rsp, [rbp - 64]",
        "pop r11",
        "pop r10",
        "pop r9",
        "pop r8",
        "pop rdi",
        "pop rsi",
        "pop rdx",
        "pop rcx",
        "pop rbp",
        "sub rax, fs:[0]",
        "ret",
        slow = sym tls_get_addr,
    )
}

/// aarch64: ABI deskriptoru meni jen `x0`, `x30` a priznaky; pomala cesta
/// ulozi `x1-x18` a vsechny `q0-q31`.
#[cfg(target_arch = "aarch64")]
#[unsafe(naked)]
pub extern "C" fn tlsdesc_dynamic() {
    naked_asm!(
        "ldr x0, [x0, #8]",
        "stp x1, x2, [sp, #-16]!",
        "mrs x1, tpidr_el0",
        "ldr x1, [x1]",
        "ldr x2, [x0]",
        "ldr x1, [x1, x2, lsl #3]",
        "cbz x1, 2f",
        "ldr x2, [x0, #8]",
        "add x0, x1, x2",
        "mrs x1, tpidr_el0",
        "sub x0, x0, x1",
        "ldp x1, x2, [sp], #16",
        "ret",
        "2:",
        "ldp x1, x2, [sp], #16",
        "stp x29, x30, [sp, #-16]!",
        "mov x29, sp",
        "sub sp, sp, #656",
        "stp x1, x2, [sp, #0]",
        "stp x3, x4, [sp, #16]",
        "stp x5, x6, [sp, #32]",
        "stp x7, x8, [sp, #48]",
        "stp x9, x10, [sp, #64]",
        "stp x11, x12, [sp, #80]",
        "stp x13, x14, [sp, #96]",
        "stp x15, x16, [sp, #112]",
        "stp x17, x18, [sp, #128]",
        "stp q0, q1, [sp, #144]",
        "stp q2, q3, [sp, #176]",
        "stp q4, q5, [sp, #208]",
        "stp q6, q7, [sp, #240]",
        "stp q8, q9, [sp, #272]",
        "stp q10, q11, [sp, #304]",
        "stp q12, q13, [sp, #336]",
        "stp q14, q15, [sp, #368]",
        "stp q16, q17, [sp, #400]",
        "stp q18, q19, [sp, #432]",
        "stp q20, q21, [sp, #464]",
        "stp q22, q23, [sp, #496]",
        "stp q24, q25, [sp, #528]",
        "stp q26, q27, [sp, #560]",
        "stp q28, q29, [sp, #592]",
        "stp q30, q31, [sp, #624]",
        "bl {slow}",
        "mrs x1, tpidr_el0",
        "sub x0, x0, x1",
        "ldp x1, x2, [sp, #0]",
        "ldp x3, x4, [sp, #16]",
        "ldp x5, x6, [sp, #32]",
        "ldp x7, x8, [sp, #48]",
        "ldp x9, x10, [sp, #64]",
        "ldp x11, x12, [sp, #80]",
        "ldp x13, x14, [sp, #96]",
        "ldp x15, x16, [sp, #112]",
        "ldp x17, x18, [sp, #128]",
        "ldp q0, q1, [sp, #144]",
        "ldp q2, q3, [sp, #176]",
        "ldp q4, q5, [sp, #208]",
        "ldp q6, q7, [sp, #240]",
        "ldp q8, q9, [sp, #272]",
        "ldp q10, q11, [sp, #304]",
        "ldp q12, q13, [sp, #336]",
        "ldp q14, q15, [sp, #368]",
        "ldp q16, q17, [sp, #400]",
        "ldp q18, q19, [sp, #432]",
        "ldp q20, q21, [sp, #464]",
        "ldp q22, q23, [sp, #496]",
        "ldp q24, q25, [sp, #528]",
        "ldp q26, q27, [sp, #560]",
        "ldp q28, q29, [sp, #592]",
        "ldp q30, q31, [sp, #624]",
        "mov sp, x29",
        "ldp x29, x30, [sp], #16",
        "ret",
        slow = sym tls_get_addr,
    )
}
