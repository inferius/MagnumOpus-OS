mod serial;

pub use serial::SerialPort;

use core::arch::asm;

/// Port prvni seriove linky (COM1). Na UEFI PC je to stale standardni adresa
/// a QEMU ji vystavuje na `-serial`.
const COM1: u16 = 0x3F8;

/// Vrati port pro ladici vystup kernelu.
///
/// Port I/O na COM1 je neskodny i kdyz tam zadna linka neni (zapisy se zahodi),
/// proto je bezpecne ho vytvaret opakovane, napr. v panic handleru.
pub fn debug_serial() -> SerialPort {
    // SAFETY: COM1 v kernelu nepouziva nic jineho nez ladici vystup.
    unsafe { SerialPort::new(COM1) }
}

/// Zakaze preruseni a navzdy zastavi aktualni CPU.
pub fn halt_forever() -> ! {
    loop {
        // SAFETY: cli + hlt jen zastavi toto CPU, nemeni zadny stav pameti.
        unsafe { asm!("cli", "hlt", options(nomem, nostack)) };
    }
}
