//! driverkit: sada pro autory ovladacu MagnumOpus-OS (DDK).
//!
//! Ovladac v userspace dela porad dokola totez: najde BARy a namapuje
//! registry, zapne MSI-X, alokuje DMA pamet pres IOMMU, ceka na preruseni
//! a zpravy od klienta, pri konci zastavi zarizeni. Tahle sada to dela
//! jednou a poradne:
//!
//! - [`regs`]: typovane registry MMIO a jejich pole (zapis hodnoty, ktera
//!   se do pole nevejde, je chyba, ne tichy prepis sousednich bitu),
//! - [`dma`]: DMA buffery s kontrolou mezi (adresa pro zarizeni jen
//!   uvnitr prideleneho bufferu),
//! - [`ring`]: kruhy deskriptoru (producent/konzument jako e1000, NVMe SQ)
//!   a kruhy dokonceni s fazovym bitem (NVMe CQ, xHCI),
//! - [`Platform`]: co ovladac od systemu potrebuje (registry, DMA,
//!   preruseni, cas, kanal ke klientovi). Stejny ovladac bezi nad jadrem
//!   ([`os`], feature `os`) i v simulaci na hostiteli ([`sim`], feature
//!   `sim`),
//! - [`Driver`] a [`run`]: zivotni cyklus (probe, start, udalosti, stop)
//!   a smycka udalosti,
//! - [`manifest`]: popis ovladace (PCI ID, tridy, BARy, kvoty preruseni
//!   a DMA, obnova po padu) v souboru `<jmeno>.driver.toml`,
//! - [`DriverError`]: citelne chyby (co ovladac nesmi, chybejici pravo,
//!   DMA mimo pridelenou pamet) s radou, co s tim.
//!
//! Prirucka: `docs/cs/ovladace.md`, `docs/en/drivers.md`.

#![cfg_attr(not(test), no_std)]

extern crate alloc;

pub mod config;
pub mod dma;
mod driver;
mod error;
pub mod io;
pub mod manifest;
mod platform;
pub mod proto;
pub mod regs;
pub mod ring;

#[cfg(feature = "os")]
pub mod os;
#[cfg(feature = "sim")]
pub mod sim;

#[cfg(test)]
mod tests;

pub use dma::{DmaBuffer, DmaMemory};
pub use driver::{Driver, Exit, Runner, run};
pub use error::{DriverError, Result};
pub use io::{Io, Width};
pub use platform::{DeviceInfo, Event, IrqId, Platform, Sent, poll_until};
pub use regs::{Field, Reg};

/// Zaloguje zpravu ovladace s jeho jmenem a adresou zarizeni
/// (`edu 00:04.0: ...`).
///
/// ```ignore
/// driverkit::log!(p, "faktorial {n} = {v}");
/// ```
#[macro_export]
macro_rules! log {
    ($p:expr, $($arg:tt)*) => {
        $crate::Platform::log($p, format_args!($($arg)*))
    };
}
