//! virtio-rng: ovladac zdroje nahody (virtio 1.x pres PCI, "entropy device").
//!
//! Spousti ho spravce zarizeni; start, smycku udalosti a zastaveni
//! zarizeni pri konci dela driverkit (`driverkit::os`). Startovni zprava:
//! Resource zarizeni (s pravem `MANAGE`, smi tedy `RANDOM_ADD`), stranka
//! konfigurace PCI a kanal k rodici (zavreni = konec ovladace).
//!
//! Ovladac hned po startu precte [`START_BYTES`] bajtu a preda je jadru
//! (`RANDOM_ADD`), pak kazdych [`REFILL`] dalsich [`REFILL_BYTES`].
//!
//! Kredit: 4 bity na bajt (polovina). Zdroj je hostitel (v QEMU
//! `/dev/urandom` nebo `BCryptGenRandom`), jehoz kvalitu ovladac neoveri.
//! RIZIKO: vadne nebo podvrzene zarizeni muze dodat predvidatelna data -
//! jadro je primicha (neuskodi), ale kredit muze generator prohlasit za
//! nasazeny driv, nez skutecne je.

#![no_std]
#![no_main]

extern crate alloc;

use alloc::vec::Vec;
use core::time::Duration;

use driverkit::os::OsPlatform;
use driverkit::{DmaBuffer, Driver, DriverError, Platform, Result};
use libos::hw::Interrupt;
use libos::{Error, Startup};
use pci::msix::MsixTable;
use virtio::{Buffer, Device, F_VERSION_1, Virtqueue};

/// Vlastnost: DMA pres IOMMU (musi se prijmout, kdyz ji zarizeni nabidne).
const F_ACCESS_PLATFORM: u64 = 1 << 33;
/// Velikost jednoho pozadavku.
const CHUNK: usize = 64;
const START_BYTES: usize = 256;
const REFILL_BYTES: usize = 64;
const REFILL: Duration = Duration::from_secs(60);
/// Kredit na bajt dat od zarizeni.
const CREDIT_PER_BYTE: u32 = 4;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(5);

libos::entry!(main);

fn main(startup: Startup) -> i64 {
    driverkit::os::main::<Rng>(startup, "virtio-rng")
}

struct Rng {
    device: Device,
    queue: Virtqueue,
    /// Na preruseni se ceka primo (pozadavek je synchronni), ne ve smycce.
    irq: Interrupt,
    _msix: MsixTable,
    buf: DmaBuffer<driverkit::os::OsDma>,
    next_refill: u64,
}

impl Driver<OsPlatform> for Rng {
    fn probe(p: &OsPlatform) -> Result<Self> {
        let virtio::Started {
            device,
            queue,
            irq,
            msix,
            ..
        } = virtio::start(p.ecam(), p.address(), p.resource(), F_VERSION_1 | F_ACCESS_PLATFORM, 4)
            .map_err(DriverError::os("inicializace virtio"))?;
        let buf = p.alloc_dma("data", 4096)?;
        Ok(Self {
            device,
            queue,
            irq,
            _msix: msix,
            buf,
            next_refill: 0,
        })
    }

    fn start(&mut self, p: &OsPlatform) -> Result<()> {
        self.feed(p, START_BYTES)?;
        driverkit::log!(p, "{START_BYTES} B entropie predano jadru");
        self.next_refill = libos::time::deadline_after(REFILL);
        Ok(())
    }

    fn next_timer(&self) -> Option<u64> {
        Some(self.next_refill)
    }

    fn timer(&mut self, p: &OsPlatform) -> Result<()> {
        self.feed(p, REFILL_BYTES)?;
        self.next_refill = libos::time::deadline_after(REFILL);
        Ok(())
    }

    fn stop(&mut self, _p: &OsPlatform) {
        // Zarizeni zastavit (zadne DMA po konci ovladace).
        let _ = self.device.reset();
    }
}

impl Rng {
    /// Jeden pozadavek: zarizeni zapise az `CHUNK` bajtu do bufferu.
    fn read_chunk(&mut self, out: &mut Vec<u8>) -> Result<()> {
        let fail = |e: Error| DriverError::Device(alloc::format!("zarizeni neodpovida ({e})"));
        self.queue
            .submit(&[Buffer {
                phys: self.buf.iova_of(0, CHUNK as u64)?,
                len: CHUNK as u32,
                device_writes: true,
            }])
            .map_err(fail)?;
        self.device.notify(&self.queue);
        let deadline = libos::time::deadline_after(REQUEST_TIMEOUT);
        let len = loop {
            if let Some((_, len)) = self.queue.take_used().map_err(fail)? {
                break len as usize;
            }
            self.irq.wait(Some(deadline)).map_err(fail)?;
        };
        let mut b = [0u8; CHUNK];
        let n = len.min(CHUNK);
        self.buf.read_bytes(0, &mut b[..n])?;
        out.extend_from_slice(&b[..n]);
        Ok(())
    }

    /// Precte `bytes` bajtu a preda je jadru.
    fn feed(&mut self, p: &OsPlatform, bytes: usize) -> Result<()> {
        let mut data = Vec::with_capacity(bytes);
        let mut attempts = 0;
        while data.len() < bytes {
            self.read_chunk(&mut data)?;
            attempts += 1;
            if attempts > bytes {
                return Err(DriverError::Device(alloc::string::String::from(
                    "zarizeni vraci prazdne odpovedi",
                )));
            }
        }
        let credit = (data.len() as u32).saturating_mul(CREDIT_PER_BYTE);
        let r = libos::random::add(p.resource(), &data, credit);
        data.fill(0);
        r.map_err(DriverError::os("RANDOM_ADD"))
    }
}
