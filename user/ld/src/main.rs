//! Zavadec sdilenych knihoven `ld.so` (`/system/lib/ld.so`, v initfs
//! `lib/ld.so`). Popis: `docs/cs/dynamicke-linkovani.md`.
//!
//! Program PIE s `PT_INTERP` spusti `spawn`: namapuje program i zavadec na
//! nahodne baze, na zasobnik da auxv (`abi::auxv`) a do bootstrap kanalu
//! pred startovni zpravu zpravu zavadece. Zavadec:
//!
//! 1. se sam zrelokuje (je static-PIE, [`start`]),
//! 2. precte zpravu zavadece (handle procesu, initfs jen pro cteni),
//! 3. nahraje `DT_NEEDED` do sirky z initfs (`lib/<soname>`) na nahodne
//!    baze, kod zapecetene (W^X),
//! 4. rozlozi staticke TLS vsech modulu a nastavi ukazatel vlakna,
//! 5. provede relokace (vse hned, `BIND_NOW`), RELRO prepne na jen pro
//!    cteni, zavola inicializatory (zavislosti napred),
//! 6. zavre sve handly, odmapuje initfs a skoci na vstup programu se stejnym
//!    bootstrap kanalem (startovni zpravu cte az program).
//!
//! Chyba = hlaska `ld.so: ...` v logu a konec procesu s kodem
//! `abi::auxv::EXIT_LOAD_FAILED`.

#![no_std]
#![no_main]

extern crate alloc;

mod bump;
mod iface;
mod load;
mod start;
mod tls;

use core::panic::PanicInfo;

/// Konec zavadece s hlaskou.
pub fn fail(msg: &str) -> ! {
    let mut buf = [0u8; 256];
    let prefix = b"ld.so: ";
    let n = msg.len().min(buf.len() - prefix.len());
    buf[..prefix.len()].copy_from_slice(prefix);
    buf[prefix.len()..prefix.len() + n].copy_from_slice(&msg.as_bytes()[..n]);
    libos::sys::debug_log(core::str::from_utf8(&buf[..prefix.len() + n]).unwrap_or("ld.so: chyba"));
    libos::sys::process_exit(abi::auxv::EXIT_LOAD_FAILED)
}

#[panic_handler]
fn panic(_: &PanicInfo) -> ! {
    fail("vnitrni chyba (panika)")
}
