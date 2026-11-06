//! Pristup k hardwaru pro ovladace: prostredky, preruseni, DMA pamet.

use abi::object::MsiMessage;
use abi::{Result, Signals};

use crate::handle::object_type;
use crate::{Handle, Vmo, sys, wait_one};

/// Opravneni k hardwaru (viz dokumentace: Resource). Korenovy prostredek je
/// bez omezeni; ovladace dostavaji omezene (`restrict`). Bez IOMMU muze
/// zarizeni pres DMA porad zapsat kamkoli.
#[derive(Debug)]
pub struct Resource(Handle);
object_type!(Resource);

/// Preruseni zarizeni (MSI).
#[derive(Debug)]
pub struct Interrupt(Handle, MsiMessage);

impl Resource {
    /// Fyzicky souvisla pamet pro DMA (max. 4 MiB).
    pub fn dma_memory(&self, size: u64) -> Result<Vmo> {
        sys::vmo_create_contiguous(self.0.raw(), size)
            .map(|h| Vmo::from_handle(Handle::from_raw(h)))
    }

    /// Okno na registry zarizeni (jen mimo RAM).
    pub fn device_memory(&self, phys: u64, size: u64) -> Result<Vmo> {
        sys::vmo_create_physical(self.0.raw(), phys, size)
            .map(|h| Vmo::from_handle(Handle::from_raw(h)))
    }

    /// Fyzicka adresa bajtu `offset` objektu pameti (pro DMA).
    pub fn phys_addr(&self, vmo: &Vmo, offset: u64) -> Result<u64> {
        sys::vmo_phys_addr(self.0.raw(), vmo.handle().raw(), offset)
    }

    /// Uzsi prostredek pro ovladac: jen fyzicke rozsahy `ranges` (zacatek,
    /// delka), nejvys `irq_max` preruseni a `dma_max` bajtu DMA pameti za
    /// dobu zivota. Rozsahy i limity musi lezet v mezich tohoto prostredku.
    pub fn restrict(&self, ranges: &[(u64, u64)], irq_max: u32, dma_max: u64) -> Result<Resource> {
        sys::resource_create(self.0.raw(), ranges, irq_max, dma_max)
            .map(|h| Resource::from_handle(Handle::from_raw(h)))
    }

    /// Novy objekt preruseni s volnym vektorem.
    pub fn interrupt(&self) -> Result<Interrupt> {
        let (h, msi) = sys::interrupt_create(self.0.raw())?;
        Ok(Interrupt(Handle::from_raw(h), msi))
    }
}

impl Interrupt {
    /// Zprava, kterou je treba zapsat do zarizeni (MSI/MSI-X).
    pub fn msi(&self) -> MsiMessage {
        self.1
    }

    /// Pocka na preruseni a hned ho potvrdi, aby se dalsi preruseni behem
    /// obsluhy neztratilo.
    pub fn wait(&self, deadline: Option<u64>) -> Result<()> {
        wait_one(&self.0, Signals::SIGNALED, deadline)?;
        sys::interrupt_ack(self.0.raw())
    }

    pub fn handle(&self) -> &Handle {
        &self.0
    }
}
