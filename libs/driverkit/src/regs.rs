//! Typovane registry a jejich pole.
//!
//! Misto `io.write32(0x98, io.read32(0x98) | 1 << 2)` pise ovladac
//! `DMA_CMD::IRQ.set(&io)`. Registr zna svou sirku a offset, pole sve bity;
//! zapis hodnoty, ktera se do pole nevejde, vrati
//! [`DriverError::FieldOverflow`] misto tiche zmeny sousednich bitu.
//!
//! Registry se popisuji makrem [`registers!`](crate::registers):
//!
//! ```
//! driverkit::registers! {
//!     /// Stav karty.
//!     pub STATUS @ 0x20: u32 {
//!         /// Probiha vypocet (jen cteni).
//!         BUSY: 0,
//!         /// Vyvolat preruseni po vypoctu.
//!         IRQ: 7,
//!     }
//!     pub FACTORIAL @ 0x08: u32;
//! }
//! assert_eq!(STATUS.off, 0x20);
//! assert_eq!(STATUS::IRQ.mask(), 0x80);
//! ```
//!
//! Konstanta `STATUS` je registr, modul `STATUS` jeho pole (jmena
//! registru a modulu ziji v ruznych jmennych prostorech).

use core::marker::PhantomData;

use crate::io::{Io, Width};
use crate::{DriverError, Result};

/// Registr sirky `W` na offsetu `off` okna.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Reg<W> {
    pub name: &'static str,
    pub off: u64,
    _w: PhantomData<W>,
}

impl<W: Width> Reg<W> {
    pub const fn new(name: &'static str, off: u64) -> Self {
        Self {
            name,
            off,
            _w: PhantomData,
        }
    }

    pub fn read<I: Io + ?Sized>(&self, io: &I) -> W {
        W::read(io, self.off)
    }

    pub fn write<I: Io + ?Sized>(&self, io: &I, v: W) {
        W::write(io, self.off, v)
    }

    /// Precte, zmeni a zapise (read-modify-write). Pozor na registry, kde
    /// cteni nebo zapis neco dela (W1C, fronty): tam [`write`](Self::write).
    pub fn modify<I: Io + ?Sized>(&self, io: &I, f: impl FnOnce(W) -> W) {
        let v = self.read(io);
        self.write(io, f(v));
    }
}

/// Pole registru: `bits` bitu od bitu `shift`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Field<W> {
    pub reg: Reg<W>,
    pub name: &'static str,
    pub shift: u32,
    pub bits: u32,
}

impl<W: Width> Field<W> {
    /// Pole bitu `lo..=hi` registru `reg`. Pole mimo sirku registru je
    /// chyba prekladu (vyhodnoceni konstanty skonci panikou).
    pub const fn new(reg: Reg<W>, name: &'static str, lo: u32, hi: u32) -> Self {
        assert!(lo <= hi && hi < W::BITS, "pole mimo sirku registru");
        Self {
            reg,
            name,
            shift: lo,
            bits: hi - lo + 1,
        }
    }

    /// Maska pole na sve pozici v registru.
    pub const fn mask(&self) -> u64 {
        let ones = if self.bits >= 64 {
            u64::MAX
        } else {
            (1u64 << self.bits) - 1
        };
        ones << self.shift
    }

    /// Hodnota pole z hodnoty registru.
    pub fn get(&self, v: W) -> u64 {
        (v.to_u64() & self.mask()) >> self.shift
    }

    /// Hodnota registru `v` s polem nastavenym na `x`. `x`, ktere se do
    /// pole nevejde, je chyba.
    pub fn with(&self, v: W, x: u64) -> Result<W> {
        if self.bits < 64 && x >> self.bits != 0 {
            return Err(DriverError::FieldOverflow {
                reg: self.reg.name,
                field: self.name,
                value: x,
                bits: self.bits,
            });
        }
        Ok(W::from_u64(
            (v.to_u64() & !self.mask()) | (x << self.shift),
        ))
    }

    /// Precte pole z registru zarizeni.
    pub fn read<I: Io + ?Sized>(&self, io: &I) -> u64 {
        self.get(self.reg.read(io))
    }

    /// Zapise pole (read-modify-write, ostatni bity registru zustanou).
    pub fn write<I: Io + ?Sized>(&self, io: &I, x: u64) -> Result<()> {
        let v = self.with(self.reg.read(io), x)?;
        self.reg.write(io, v);
        Ok(())
    }

    /// Nastavi vsechny bity pole (u jednobitoveho pole "zapnout").
    pub fn set<I: Io + ?Sized>(&self, io: &I) {
        let m = self.mask();
        self.reg
            .modify(io, |v| W::from_u64(v.to_u64() | m));
    }

    /// Vynuluje bity pole.
    pub fn clear<I: Io + ?Sized>(&self, io: &I) {
        let m = self.mask();
        self.reg
            .modify(io, |v| W::from_u64(v.to_u64() & !m));
    }

    /// Je pole nenulove?
    pub fn is_set<I: Io + ?Sized>(&self, io: &I) -> bool {
        self.read(io) != 0
    }

    /// Hodnota registru jen s timto polem (pro registry, kam se zapisuje
    /// cela hodnota najednou, napr. prikaz nebo W1C).
    pub fn value(&self, x: u64) -> Result<W> {
        self.with(W::from_u64(0), x)
    }
}

/// Popis registru a jejich poli. Pole je `JMENO: bit` nebo
/// `JMENO: od..=do`. Priklad v dokumentaci modulu [`regs`](crate::regs).
#[macro_export]
macro_rules! registers {
    () => {};
    (
        $(#[$m:meta])*
        $vis:vis $name:ident @ $off:literal : $w:ty ;
        $($rest:tt)*
    ) => {
        $(#[$m])*
        #[allow(non_upper_case_globals)]
        $vis const $name: $crate::Reg<$w> = $crate::Reg::new(stringify!($name), $off);
        $crate::registers! { $($rest)* }
    };
    (
        $(#[$m:meta])*
        $vis:vis $name:ident @ $off:literal : $w:ty {
            $(
                $(#[$fm:meta])*
                $field:ident : $lo:literal $(..= $hi:literal)?
            ),* $(,)?
        }
        $($rest:tt)*
    ) => {
        $(#[$m])*
        #[allow(non_upper_case_globals)]
        $vis const $name: $crate::Reg<$w> = $crate::Reg::new(stringify!($name), $off);
        $(#[$m])*
        #[allow(non_snake_case, dead_code)]
        $vis mod $name {
            #[allow(unused_imports)]
            use super::*;
            $(
                $(#[$fm])*
                #[allow(dead_code)]
                pub const $field: $crate::Field<$w> = $crate::Field::new(
                    $crate::Reg::new(stringify!($name), $off),
                    stringify!($field),
                    $lo,
                    $crate::registers!(@hi $lo $(, $hi)?),
                );
            )*
        }
        $crate::registers! { $($rest)* }
    };
    (@hi $lo:literal) => { $lo };
    (@hi $lo:literal, $hi:literal) => { $hi };
}
