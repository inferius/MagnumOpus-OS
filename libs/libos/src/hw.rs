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

/// Rozsah I/O portu pro [`Resource::restrict`]: `count` portu od `first`.
pub const fn io_ports(first: u16, count: u16) -> (u64, u64) {
    (abi::syscall::RANGE_IO_PORTS | first as u64, count as u64)
}

/// Rozsah starych linek preruseni (ISA IRQ) pro [`Resource::restrict`].
pub const fn irq_lines(first: u32, count: u32) -> (u64, u64) {
    (abi::syscall::RANGE_IRQ_LINES | first as u64, count as u64)
}

/// Preruseni zarizeni (MSI nebo stara linka).
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
    /// delka; I/O porty a linky preruseni pres [`io_ports`] a
    /// [`irq_lines`]), nejvys `irq_max` preruseni a `dma_max` bajtu DMA
    /// pameti za dobu zivota. Rozsahy i limity musi lezet v mezich tohoto prostredku.
    pub fn restrict(&self, ranges: &[(u64, u64)], irq_max: u32, dma_max: u64) -> Result<Resource> {
        sys::resource_create(self.0.raw(), ranges, irq_max, dma_max, 0)
            .map(|h| Resource::from_handle(Handle::from_raw(h)))
    }

    /// Jako `restrict`, navic pro konkretni zarizeni PCI: s IOMMU dostane
    /// zarizeni vlastni domenu a smi pres DMA jen do pameti vytvorene
    /// pres tento prostredek (`dma_memory`). Bez IOMMU stejne jako `restrict`.
    pub fn restrict_device(
        &self,
        ranges: &[(u64, u64)],
        irq_max: u32,
        dma_max: u64,
        (segment, bus, device, function): (u16, u8, u8, u8),
    ) -> Result<Resource> {
        let id = abi::syscall::pci_device(segment, bus, device, function);
        sys::resource_create(self.0.raw(), ranges, irq_max, dma_max, id)
            .map(|h| Resource::from_handle(Handle::from_raw(h)))
    }

    /// Nastavi realny cas systemu (vyzaduje pravo `SET_CLOCK`, ma ho jen
    /// sluzba casu). Viz `time::set`.
    pub fn set_clock(&self, req: &abi::time::SetRequest, flags: u64) -> Result<bool> {
        sys::clock_set(self.0.raw(), req, flags)
    }

    /// Preruseni na stare lince (ISA IRQ pres IOAPIC, napr. PS/2: 1 a 12).
    /// Prostredek musi linku povolovat ([`irq_lines`]). Zprava MSI je
    /// u takoveho objektu prazdna.
    pub fn interrupt_line(&self, line: u32) -> Result<Interrupt> {
        self.interrupt_line_with(line, 0)
    }

    /// Jako [`Resource::interrupt_line`] s priznaky spousteni
    /// (`abi::syscall::line_flags`; aarch64 SPI: hrana/uroven z ACPI _CRS).
    pub fn interrupt_line_with(&self, line: u32, flags: u64) -> Result<Interrupt> {
        let h = sys::interrupt_create_line(self.0.raw(), line, flags)?;
        Ok(Interrupt(Handle::from_raw(h), MsiMessage::default()))
    }

    /// Precte I/O port sirky 1, 2 nebo 4 bajty.
    pub fn port_read(&self, port: u16, width: u8) -> Result<u32> {
        sys::ioport_read(self.0.raw(), port, width)
    }

    /// Zapise I/O port sirky 1, 2 nebo 4 bajty.
    pub fn port_write(&self, port: u16, width: u8, value: u32) -> Result<()> {
        sys::ioport_write(self.0.raw(), port, width, value)
    }

    /// Vypne (`abi::syscall::power_op::OFF`) nebo restartuje pocitac pres
    /// firmware (pravo `POWER`). Pri uspechu se nevraci.
    pub fn system_power(&self, op: u64) -> Result<()> {
        sys::system_power(self.0.raw(), op)
    }

    /// Precte bajt z I/O portu (x86; prostredek musi port povolovat,
    /// [`io_ports`]).
    pub fn port_read8(&self, port: u16) -> Result<u8> {
        sys::ioport_read(self.0.raw(), port, 1).map(|v| v as u8)
    }

    /// Zapise bajt na I/O port.
    pub fn port_write8(&self, port: u16, value: u8) -> Result<()> {
        sys::ioport_write(self.0.raw(), port, 1, value as u32)
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

    /// Potvrdi preruseni bez cekani (po `wait_many`, ktere cekalo i na jine
    /// objekty).
    pub fn ack(&self) -> Result<()> {
        sys::interrupt_ack(self.0.raw())
    }

    pub fn handle(&self) -> &Handle {
        &self.0
    }
}
