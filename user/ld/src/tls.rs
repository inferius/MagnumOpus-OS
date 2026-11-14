//! Staticke TLS modulu nahranych pri startu: `__tls_get_addr` (x86_64
//! general/local-dynamic) a resitel statickeho deskriptoru TLSDESC (aarch64
//! vychozi, x86_64 GNU2). Vsechny moduly lezi ve staticke casti, takze
//! adresa = TP + posun modulu + posun promenne. Dynamicke TLS (`dlopen`)
//! a TLS novych vlaken prijde v DL-3.

use core::arch::naked_asm;
use core::sync::atomic::{AtomicI64, Ordering};

/// Nejvic modulu s cislem TLS (cislo modulu od 1).
pub const MAX_MODULES: usize = 256;

/// Posun dat modulu od ukazatele vlakna (index = cislo modulu).
static TPOFF: [AtomicI64; MAX_MODULES + 1] = [const { AtomicI64::new(0) }; MAX_MODULES + 1];

pub fn set_offset(module: u64, off: i64) {
    TPOFF[module as usize].store(off, Ordering::Relaxed);
}

/// Ukazatel vlakna.
fn tp() -> u64 {
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

/// `__tls_get_addr(&tls_index { modul, posun })`.
pub extern "C" fn tls_get_addr(ti: *const [u64; 2]) -> *mut u8 {
    // SAFETY: argument je `tls_index` z GOT modulu (2 slova, zapsal je
    // zavadec relokacemi DTPMOD/DTPOFF).
    let [m, off] = unsafe { ti.read() };
    let base = TPOFF.get(m as usize).map_or(0, |a| a.load(Ordering::Relaxed));
    tp().wrapping_add_signed(base).wrapping_add(off) as *mut u8
}

/// Resitel statickeho deskriptoru TLS: vraci argument deskriptoru (posun
/// od TP). Vlastni ABI: meni jen `rax` / `x0`.
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
