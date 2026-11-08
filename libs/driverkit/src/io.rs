//! Pristup k registrum: okno MMIO (BAR) nebo konfiguracni prostor PCI.
//!
//! [`Io`] je jediny zpusob, jak ovladac sahne na registry. Nad jadrem je to
//! namapovany BAR (volatilni pristupy), v simulaci volani modelu zarizeni.
//! Offset je od zacatku okna; pristup za koncem okna se nevykona (cteni
//! vraci same jednicky jako neexistujici zarizeni na sbernici).

/// Okno registru.
pub trait Io {
    fn read8(&self, off: u64) -> u8;
    fn read16(&self, off: u64) -> u16;
    fn read32(&self, off: u64) -> u32;
    fn read64(&self, off: u64) -> u64;
    fn write8(&self, off: u64, v: u8);
    fn write16(&self, off: u64, v: u16);
    fn write32(&self, off: u64, v: u32);
    fn write64(&self, off: u64, v: u64);
    /// Delka okna v bajtech.
    fn len(&self) -> u64;
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Sirka registru: `u8`, `u16`, `u32`, `u64`.
pub trait Width: Copy + Eq + core::fmt::Debug + 'static {
    const BITS: u32;
    fn to_u64(self) -> u64;
    /// Orizne na sirku (vyssi bity se zahodi).
    fn from_u64(v: u64) -> Self;
    fn read<I: Io + ?Sized>(io: &I, off: u64) -> Self;
    fn write<I: Io + ?Sized>(io: &I, off: u64, v: Self);
}

macro_rules! width {
    ($t:ty, $bits:expr, $r:ident, $w:ident) => {
        impl Width for $t {
            const BITS: u32 = $bits;
            fn to_u64(self) -> u64 {
                self as u64
            }
            fn from_u64(v: u64) -> Self {
                v as $t
            }
            fn read<I: Io + ?Sized>(io: &I, off: u64) -> Self {
                io.$r(off)
            }
            fn write<I: Io + ?Sized>(io: &I, off: u64, v: Self) {
                io.$w(off, v)
            }
        }
    };
}

width!(u8, 8, read8, write8);
width!(u16, 16, read16, write16);
width!(u32, 32, read32, write32);
width!(u64, 64, read64, write64);

/// Pomocnik pro implementace [`Io`]: vejde se pristup `size` bajtu na
/// `off` do okna delky `len`?
pub fn fits(off: u64, size: u64, len: u64) -> bool {
    off.checked_add(size).is_some_and(|end| end <= len)
}
