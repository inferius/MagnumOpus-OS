//! HPET - jen jako referencni hodiny pro kalibraci TSC a APIC timeru.

use crate::mm::{self, PhysAddr};

const REG_CAPABILITIES: usize = 0x00;
const REG_CONFIG: usize = 0x10;
const REG_COUNTER: usize = 0xF0;
const CONFIG_ENABLE: u64 = 1;

pub struct Hpet {
    base: *mut u64,
    /// Perioda jednoho tiku ve femtosekundach.
    period_fs: u64,
}

impl Hpet {
    /// # Safety
    ///
    /// `phys` musi byt adresa registru HPET z ACPI.
    pub unsafe fn new(phys: u64) -> Self {
        let base: *mut u64 = mm::map_mmio(PhysAddr::new(phys), 1024).as_mut_ptr();
        let hpet = Self { base, period_fs: 0 };
        let period_fs = hpet.read(REG_CAPABILITIES) >> 32;
        assert!(
            period_fs != 0 && period_fs <= 100_000_000,
            "neplatna perioda HPET"
        );
        let hpet = Self { base, period_fs };
        hpet.write(REG_CONFIG, hpet.read(REG_CONFIG) | CONFIG_ENABLE);
        hpet
    }

    fn read(&self, reg: usize) -> u64 {
        // SAFETY: registr lezi v namapovanem 1 KiB bloku HPET (viz `new`).
        unsafe { self.base.byte_add(reg).read_volatile() }
    }

    fn write(&self, reg: usize, value: u64) {
        // SAFETY: viz `read`.
        unsafe { self.base.byte_add(reg).write_volatile(value) }
    }

    /// Zmeri frekvenci (Hz) libovolneho citace: `read` se precte na zacatku
    /// a na konci okna o delce `window_ns`.
    pub fn measure_hz(&self, window_ns: u64, mut read: impl FnMut() -> u64) -> u64 {
        let ticks = window_ns * 1_000_000 / self.period_fs;
        let start = self.read(REG_COUNTER);
        let a = read();
        while self.read(REG_COUNTER).wrapping_sub(start) < ticks {
            core::hint::spin_loop();
        }
        let b = read();
        let elapsed_fs = self.read(REG_COUNTER).wrapping_sub(start) * self.period_fs;
        ((b.wrapping_sub(a)) as u128 * 1_000_000_000_000_000 / elapsed_fs as u128) as u64
    }
}
