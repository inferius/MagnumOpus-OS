//! Ladici seriova linka ARM PL011 (QEMU virt: 0x0900_0000, adresu da
//! loader z ACPI SPCR v `BootInfo::debug_serial`).
//!
//! Adresa registru se behem startu meni: nejdriv fyzicka adresa (loader ji
//! identicky namapoval jako zarizeni), po zavedeni tabulek jadra pevna
//! virtualni adresa `mmu::EARLY_UART_VA`. Proto je ulozena v atomicke
//! promenne a kazdy zapis si ji precte znovu. Rychlost a format linky
//! nastavil firmware; jadro je nemeni (frekvence hodin UARTu neni znama).

use core::fmt;
use core::sync::atomic::{AtomicU64, Ordering};

/// Virtualni adresa registru; 0 = linka neni (vystup se zahazuje).
static BASE: AtomicU64 = AtomicU64::new(0);
/// Fyzicka adresa registru (pro premapovani po zavedeni tabulek jadra).
static PHYS: AtomicU64 = AtomicU64::new(0);

const DR: u64 = 0x00;
const FR: u64 = 0x18;
const FR_TXFF: u32 = 1 << 5;
const CR: u64 = 0x30;
const CR_UARTEN: u32 = 1 << 0;
const CR_TXE: u32 = 1 << 8;

/// Nastavi fyzickou adresu linky od loaderu (zatim pristupnou identicky).
pub fn set_phys(phys: u64) {
    PHYS.store(phys, Ordering::Relaxed);
    BASE.store(phys, Ordering::Release);
}

pub fn phys() -> u64 {
    PHYS.load(Ordering::Relaxed)
}

/// Prepne na novou virtualni adresu registru (po zmene tabulek stranek).
pub fn set_virt(virt: u64) {
    BASE.store(virt, Ordering::Release);
}

/// Ladici linka; levna kopie, kazdy zapis cte aktualni adresu.
pub struct Pl011;

impl Pl011 {
    fn write_byte(base: u64, b: u8) {
        // SAFETY: `base` ukazuje na registry PL011 namapovane jako zarizeni
        // (identicky od loaderu nebo `mmu::EARLY_UART_VA`); cteni FR a zapis
        // DR nemaji jine ucinky nez vyslani znaku.
        unsafe {
            let fr = (base + FR) as *const u32;
            let mut spins = 0u32;
            while fr.read_volatile() & FR_TXFF != 0 && spins < 1_000_000 {
                core::hint::spin_loop();
                spins += 1;
            }
            ((base + DR) as *mut u32).write_volatile(b as u32);
        }
    }
}

impl fmt::Write for Pl011 {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        let base = BASE.load(Ordering::Acquire);
        if base == 0 {
            return Ok(());
        }
        for b in s.bytes() {
            if b == b'\n' {
                Self::write_byte(base, b'\r');
            }
            Self::write_byte(base, b);
        }
        Ok(())
    }
}

impl crate::arch::api::DebugSerial for Pl011 {
    /// Zapne vysilac (firmware ho obvykle uz zapnul).
    fn init(&mut self) {
        let base = BASE.load(Ordering::Acquire);
        if base == 0 {
            return;
        }
        // SAFETY: viz `write_byte`; zapnuti UARTu a vysilace.
        unsafe {
            let cr = (base + CR) as *mut u32;
            cr.write_volatile(cr.read_volatile() | CR_UARTEN | CR_TXE);
        }
    }
}
