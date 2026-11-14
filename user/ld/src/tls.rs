//! TLS modulu: `__tls_get_addr` (x86_64 general/local-dynamic) pres DTV
//! a resitel statickeho deskriptoru TLSDESC (aarch64 vychozi, x86_64 GNU2).
//!
//! **DTV** (vektor dynamickeho TLS, model glibc): kazde vlakno ma pole
//! `[generace, adresa bloku modulu 1, 2, ...]`, ukazatel na nej lezi v TCB
//! (x86_64 `TP + 8`, aarch64 `TP + 0`). Moduly ze startu maji blok ve
//! statickem TLS (polozky vyplni `iface::new_block`). Moduly z budouciho
//! `dlopen` (DL-4) zvysi globalni generaci; vlakno se starsi generaci nebo
//! s prazdnou polozkou pujde pomalou cestou (lina alokace bloku) - ta zatim
//! neni, protoze zadny takovy modul nevznika.

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

/// `__tls_get_addr(&tls_index { modul, posun })`: blok modulu z DTV
/// vlakna + posun.
pub extern "C" fn tls_get_addr(ti: *const [u64; 2]) -> *mut u8 {
    // SAFETY: argument je `tls_index` z GOT modulu (2 slova, zapsal je
    // zavadec relokacemi DTPMOD/DTPOFF); DTV vlakna zalozil `new_block`.
    unsafe {
        let [m, off] = ti.read();
        let dtv = ((tp() + TCB_DTV) as *const u64).read() as *const u64;
        let block = if (m as usize) <= MAX_MODULES { dtv.add(m as usize).read() } else { 0 };
        if block == 0 {
            // Pomala cesta (modul z dlopen ve vlakne se starsi generaci
            // DTV, lina alokace bloku) prijde s DL-4.
            crate::fail("TLS modulu, ktery neni ve statickem bloku (dlopen az v DL-4)");
        }
        (block + off) as *mut u8
    }
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
