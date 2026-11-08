//! Co ovladac potrebuje od systemu: [`Platform`].
//!
//! Ovladac napsany proti `Platform` (genericky `P: Platform`) bezi beze
//! zmeny nad jadrem ([`crate::os::OsPlatform`]) i v simulaci na hostiteli
//! ([`crate::sim::SimPlatform`]). Kod, ktery potrebuje neco jen z jadra
//! (napr. knihovnu `virtio` nad `Ecam`), muze implementovat
//! `Driver<OsPlatform>` primo; simulace pak pro nej neni.

use alloc::vec::Vec;
use core::fmt;

use crate::dma::{DmaBuffer, DmaMemory};
use crate::io::Io;
use crate::{DriverError, Result};

/// Zarizeni, ke kteremu ovladac patri.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceInfo {
    /// Adresa na PCI (`sbernice:zarizeni.funkce`).
    pub bus: u8,
    pub device: u8,
    pub function: u8,
    pub vendor: u16,
    pub device_id: u16,
}

impl fmt::Display for DeviceInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:02x}:{:02x}.{}", self.bus, self.device, self.function)
    }
}

/// Cislo preruseni ovladace (poradi zapnuti: prvni = 0; u MSI-X polozka
/// tabulky).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct IrqId(pub u16);

/// Udalost pro ovladac.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// Zarizeni vyvolalo preruseni (backend ho uz potvrdil, dalsi se
    /// neztrati).
    Interrupt(IrqId),
    /// Zprava od klienta (sluzby nebo initu).
    Message(Vec<u8>),
    /// Uplynul termin z [`Driver::next_timer`](crate::Driver::next_timer).
    Timer,
    /// Klient zavrel kanal: ovladac ma skoncit.
    ClientClosed,
}

/// Vysledek odeslani zpravy klientovi.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sent {
    Sent,
    /// Fronta klienta je plna (klient nestiha). Ovladac rozhodne, jestli
    /// zpravu zahodit (udalosti vstupu) nebo zkusit pozdeji.
    Full,
}

/// Rozhrani ovladace k systemu.
pub trait Platform {
    /// Okno registru (BAR).
    type Mmio: Io;
    /// DMA pamet.
    type Dma: DmaMemory;

    fn info(&self) -> &DeviceInfo;

    /// Jmeno ovladace (pro logy).
    fn name(&self) -> &str;

    /// Namapuje pametovy BAR `index` cely. BAR, ktery ovladac nedostal
    /// (neni v `resources.bars` manifestu), je [`DriverError::MissingRight`].
    fn map_bar(&self, index: u8) -> Result<Self::Mmio>;

    /// Alokuje DMA buffer (`name` je pro hlasky). Nad kvotu
    /// `resources.dma_max` = [`DriverError::DmaQuota`].
    fn alloc_dma(&self, name: &'static str, size: u64) -> Result<DmaBuffer<Self::Dma>>;

    /// Zapne dalsi preruseni zarizeni (MSI-X polozka `n`, nebo MSI, kdyz
    /// MSI-X zarizeni nema) a vrati jeho cislo.
    fn enable_irq(&self) -> Result<IrqId>;

    /// Cas v ns od startu systemu (v simulaci virtualni).
    fn now(&self) -> u64;

    /// Kratke cekani pri dotazovani registru (v simulaci posune cas a da
    /// zarizeni prilezitost pracovat).
    fn relax(&self);

    /// Pocka na dalsi udalost; `deadline` (ns od startu) = nejpozdeji pak
    /// vrati [`Event::Timer`].
    fn wait(&self, deadline: Option<u64>) -> Result<Event>;

    /// Posle zpravu klientovi.
    fn send(&self, data: &[u8]) -> Result<Sent>;

    fn log(&self, args: fmt::Arguments);

    /// Bariera: zapisy do DMA pameti pred ni zarizeni uvidi driv nez
    /// zapisy za ni (deskriptory pred indexem).
    fn dma_wmb(&self);
    /// Bariera: precteny index od zarizeni pred ctenim jeho dat.
    fn dma_rmb(&self);
    /// Bariera: vsechny zapisy do pameti pred zapisem do registru
    /// ("doorbell").
    fn mmio_wmb(&self);
}

/// Opakuje `done()`, dokud nevrati `true`, nejvys `timeout_ms`. Pro
/// registry, ktere zarizeni zmeni "brzy" (reset, pripravenost). Na
/// preruseni se ceka pres [`Platform::wait`].
pub fn poll_until<P: Platform + ?Sized>(
    p: &P,
    what: &'static str,
    timeout_ms: u64,
    mut done: impl FnMut() -> bool,
) -> Result<()> {
    let deadline = p.now().saturating_add(timeout_ms * 1_000_000);
    loop {
        if done() {
            return Ok(());
        }
        if p.now() >= deadline {
            return Err(DriverError::Timeout {
                what,
                ms: timeout_ms,
            });
        }
        p.relax();
    }
}
