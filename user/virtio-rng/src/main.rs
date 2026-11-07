//! virtio-rng: ovladac zdroje nahody (virtio 1.x pres PCI, "entropy device").
//!
//! Spousti ho spravce zarizeni. Startovni zprava (kind USER, v tomto poradi):
//! 1. omezeny Resource tohoto zarizeni (s pravem `MANAGE`, smi tedy
//!    `RANDOM_ADD`),
//! 2. stranka konfiguracniho prostoru PCI,
//! 3. kanal k rodici - jen hlida konec (zavreni = konec ovladace).
//!
//! Prvni zprava na bootstrap kanalu: `pci <sbernice> <zarizeni> <funkce>`.
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

use libos::abi::vm::{MAP_READ, MAP_WRITE};
use libos::hw::{Interrupt, Resource};
use libos::{Channel, Error, Mapping, Result, Startup, Vmo, println};
use pci::msix::MsixTable;
use pci::{Address, Ecam};
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

struct Rng {
    device: Device,
    queue: Virtqueue,
    irq: Interrupt,
    _msix: MsixTable,
    _dma: Vmo,
    buf: Mapping,
    buf_phys: u64,
}

fn main(startup: Startup) -> i64 {
    let mut handles = startup.handles.into_iter();
    let (Some(res), Some(ecam_vmo), Some(parent)) =
        (handles.next(), handles.next(), handles.next())
    else {
        println!("virtio-rng: chybi handly ve startovni zprave");
        return 1;
    };
    let resource = Resource::from_handle(res);
    let ecam_vmo = Vmo::from_handle(ecam_vmo);
    let parent = Channel::from_handle(parent);
    let Some(address) = virtio::read_driver_config(&startup.bootstrap) else {
        println!("virtio-rng: neplatna konfigurace od spravce zarizeni");
        return 2;
    };
    let rng = Ecam::single(&ecam_vmo, address).and_then(|e| Rng::open(&e, address, &resource));
    let mut rng = match rng {
        Ok(r) => r,
        Err(e) => {
            println!("virtio-rng {address}: inicializace selhala: {e}");
            return 3;
        }
    };
    match rng.feed(&resource, START_BYTES) {
        Ok(()) => println!("virtio-rng {address}: {START_BYTES} B entropie predano jadru"),
        Err(e) => {
            let _ = rng.device.reset();
            println!("virtio-rng {address}: zarizeni neodpovida ({e}), koncim");
            return 4;
        }
    }
    loop {
        // Cekani na dalsi doplneni = cekani na konec rodice.
        match parent.read(Some(libos::time::deadline_after(REFILL))) {
            Err(Error::TimedOut) => {}
            Ok(_) => continue,
            Err(_) => {
                // Zarizeni zastavit (zadne DMA po konci ovladace).
                let _ = rng.device.reset();
                return 0;
            }
        }
        if let Err(e) = rng.feed(&resource, REFILL_BYTES) {
            let _ = rng.device.reset();
            println!("virtio-rng {address}: zarizeni neodpovida ({e}), koncim");
            return 4;
        }
    }
}

impl Rng {
    fn open(ecam: &Ecam, a: Address, resource: &Resource) -> Result<Self> {
        let device = Device::open(ecam, a, resource)?;
        device.begin_init()?;
        device.negotiate(F_VERSION_1 | F_ACCESS_PLATFORM)?;
        let irq = resource.interrupt()?;
        let msix = MsixTable::open(ecam, a, resource)?;
        msix.set(0, irq.msi())?;
        msix.enable(ecam);
        let queue = device.setup_queue(0, 4, 0, resource)?;
        device.driver_ok();
        let dma = resource.dma_memory(4096)?;
        let buf_phys = resource.phys_addr(&dma, 0)?;
        let buf = Mapping::new(&dma, 0, 4096, MAP_READ | MAP_WRITE)?;
        Ok(Self {
            device,
            queue,
            irq,
            _msix: msix,
            _dma: dma,
            buf,
            buf_phys,
        })
    }

    /// Jeden pozadavek: zarizeni zapise az `CHUNK` bajtu do bufferu.
    fn read_chunk(&mut self, out: &mut Vec<u8>) -> Result<()> {
        self.queue.submit(&[Buffer {
            phys: self.buf_phys,
            len: CHUNK as u32,
            device_writes: true,
        }])?;
        self.device.notify(&self.queue);
        let deadline = libos::time::deadline_after(REQUEST_TIMEOUT);
        let len = loop {
            if let Some((_, len)) = self.queue.take_used()? {
                break len as usize;
            }
            self.irq.wait(Some(deadline))?;
        };
        let mut b = [0u8; CHUNK];
        let n = len.min(CHUNK);
        self.buf.read_bytes(0, &mut b[..n]);
        out.extend_from_slice(&b[..n]);
        Ok(())
    }

    /// Precte `bytes` bajtu a preda je jadru.
    fn feed(&mut self, resource: &Resource, bytes: usize) -> Result<()> {
        let mut data = Vec::with_capacity(bytes);
        let mut attempts = 0;
        while data.len() < bytes {
            self.read_chunk(&mut data)?;
            attempts += 1;
            if attempts > bytes {
                // Zarizeni vraci prazdne odpovedi.
                return Err(Error::IoError);
            }
        }
        let credit = (data.len() as u32).saturating_mul(CREDIT_PER_BYTE);
        let r = libos::random::add(resource, &data, credit);
        data.fill(0);
        r
    }
}
