//! virtio-blk: ovladac virtualniho disku (virtio 1.x pres PCI).
//!
//! Spousti ho spravce zarizeni. Startovni zprava (kind USER, v tomto poradi):
//! 1. omezeny Resource (jen BARy a konfigurace tohoto zarizeni, kvoty
//!    preruseni a DMA),
//! 2. stranka konfiguracniho prostoru PCI teto funkce,
//! 3. kanal pro klienta (protokol `blockproto`).
//!
//! Prvni zprava na bootstrap kanalu od spravce: `pci <sbernice> <zarizeni>
//! <funkce>` (desitkove). Ovladac pak obsluhuje pozadavky
//! klienta, dokud klient kanal nezavre (`blockproto::server`, vcetne
//! ochrany cizich disku).

#![no_std]
#![no_main]

extern crate alloc;

use alloc::vec;
use alloc::vec::Vec;

use blockproto::server::{self, Disks};
use blockproto::SECTOR;
use libos::abi::vm::{MAP_READ, MAP_WRITE};
use libos::hw::{Interrupt, Resource};
use libos::{Channel, Error, Mapping, Result, Startup, Vmo, println};
use pci::msix::MsixTable;
use pci::{Address, Ecam};
use virtio::{Buffer, Device, F_VERSION_1, Virtqueue};

/// Vlastnost: zarizeni je jen pro cteni.
const F_RO: u64 = 1 << 5;
/// Vlastnost: zarizeni hlasi velikost logickeho bloku (`blk_size`).
const F_BLK_SIZE: u64 = 1 << 6;
/// Vlastnost: zarizeni podporuje FLUSH.
const F_FLUSH: u64 = 1 << 9;
/// Vlastnost: zarizeni podporuje DISCARD (TRIM).
const F_DISCARD: u64 = 1 << 13;
/// Vlastnost: zarizeni pouziva pri DMA preklad platformy (IOMMU). Kdyz ji
/// zarizeni nabidne, ovladac ji musi prijmout, jinak zarizeni odmitne start.
const F_ACCESS_PLATFORM: u64 = 1 << 33;

const REQ_IN: u32 = 0;
const REQ_OUT: u32 = 1;
const REQ_FLUSH: u32 = 4;
const REQ_DISCARD: u32 = 11;

/// Offset `blk_size` v konfiguraci virtio-blk.
const CFG_BLK_SIZE: u64 = 20;
/// Offset `max_discard_sectors` v konfiguraci virtio-blk.
const CFG_MAX_DISCARD_SECTORS: u64 = 36;

/// Rozlozeni DMA bufferu pozadavku: hlavicka 16 B, stav 1 B, data.
const HDR_OFF: u64 = 0;
const STATUS_OFF: u64 = 16;
const DATA_OFF: u64 = 512;
/// Nejvic bajtu jednoho cteni: server spoji navazujici cteni klienta
/// (po `MAX_SECTORS`) do jednoho prikazu az do teto delky: jedno preruseni
/// a jedno cekani na 1 MiB misto na kazdych 32 KiB.
const READ_MAX: usize = server::BATCH_MAX;
const DMA_SIZE: u64 = DATA_OFF + READ_MAX as u64;

libos::entry!(main);

struct Disk {
    device: Device,
    queue: Virtqueue,
    irq: Interrupt,
    _msix: MsixTable,
    _dma: Vmo,
    buf: Mapping,
    buf_phys: u64,
    /// Kapacita v sektorech po 512 B (adresovani virtio je vzdy po 512 B).
    capacity: u64,
    /// Logicky blok zarizeni (512 B, nebo vic u disku 4Kn); pozadavky
    /// musi byt na nej zarovnane.
    block: usize,
    flush: bool,
    /// Zarizeni je jen pro cteni (F_RO).
    read_only: bool,
    /// Nejvic sektoru jednoho DISCARD (0 = DISCARD neumi).
    max_discard: u32,
    /// Zarizeni neodpovedelo nebo vratilo nesmysl: dalsi pozadavky vraci
    /// `IO_ERROR` (DMA buffer muze zarizeni jeste prepisovat).
    failed: bool,
    /// Prostredek ovladace (test ochrany DMA potrebuje fyzickou adresu
    /// stranky, ktera neni DMA pamet).
    resource: Option<Resource>,
}

/// Jak dlouho cekat na dokonceni jednoho pozadavku.
const REQUEST_TIMEOUT: core::time::Duration = core::time::Duration::from_secs(10);

fn main(startup: Startup) -> i64 {
    let mut handles = startup.handles.into_iter();
    let (Some(res), Some(ecam_vmo), Some(client)) =
        (handles.next(), handles.next(), handles.next())
    else {
        println!("virtio-blk: chybi handly ve startovni zprave");
        return 1;
    };
    let resource = Resource::from_handle(res);
    let ecam_vmo = Vmo::from_handle(ecam_vmo);
    let client = Channel::from_handle(client);

    let Some(address) = virtio::read_driver_config(&startup.bootstrap) else {
        println!("virtio-blk: neplatna konfigurace od spravce zarizeni");
        return 2;
    };
    let disk =
        Ecam::single(&ecam_vmo, address).and_then(|ecam| Disk::open(&ecam, address, &resource));
    let mut disk = match disk {
        Ok(d) => d,
        Err(e) => {
            println!("virtio-blk {address}: inicializace selhala: {e}");
            return 3;
        }
    };
    println!(
        "virtio-blk {address}: {} MiB, blok {} B, flush {}, discard {}",
        disk.capacity * SECTOR as u64 / (1024 * 1024),
        disk.block,
        if disk.flush { "ano" } else { "ne" },
        if disk.max_discard > 0 { "ano" } else { "ne" }
    );
    let view = server::guard("virtio-blk", &mut disk, 0);
    disk.resource = Some(resource);
    server::serve(
        "virtio-blk",
        &mut disk,
        vec![server::Client {
            disk: 0,
            view,
            channel: client,
            // Disk virtio dostava kanal od spravce zarizeni; raw kanal pro
            // instalator ma jen NVMe, AHCI a USB (skutecne disky).
            raw_table: None,
        }],
    );
    0
}

impl Disk {
    fn open(ecam: &Ecam, a: Address, resource: &Resource) -> Result<Self> {
        let virtio::Started {
            device,
            queue,
            irq,
            msix,
            features,
        } = virtio::start(
            ecam,
            a,
            resource,
            F_VERSION_1 | F_RO | F_BLK_SIZE | F_FLUSH | F_DISCARD | F_ACCESS_PLATFORM,
            16,
        )?;

        let dma = resource.dma_memory(DMA_SIZE)?;
        let buf_phys = resource.phys_addr(&dma, 0)?;
        let buf = Mapping::new(&dma, 0, DMA_SIZE, MAP_READ | MAP_WRITE)?;
        // Kapacita v sektorech po 512 B je na zacatku konfigurace zarizeni.
        let capacity: u64 = device.config(0);
        let block = if features & F_BLK_SIZE != 0 {
            device.config::<u32>(CFG_BLK_SIZE) as usize
        } else {
            SECTOR
        };
        // Neplatna nebo prilis velka hodnota: zarizeni nepouzit (zarovnani
        // pozadavku by nesedelo a zapisy by mohly selhavat napul).
        if !block_ok(block) {
            println!("virtio-blk {a}: nepodporovany logicky blok {block} B");
            let _ = device.reset();
            return Err(Error::NotSupported);
        }
        // Pole max_discard_sectors (u32 na offsetu 36) musi zarizeni hlasit.
        let max_discard: u32 =
            if features & F_DISCARD != 0 && device.config_len() >= CFG_MAX_DISCARD_SECTORS + 4 {
                device.config::<u32>(CFG_MAX_DISCARD_SECTORS).max(1)
            } else {
                0
            };
        Ok(Self {
            device,
            queue,
            irq,
            _msix: msix,
            _dma: dma,
            buf,
            buf_phys,
            capacity,
            block,
            flush: features & F_FLUSH != 0,
            read_only: features & F_RO != 0,
            max_discard,
            failed: false,
            resource: None,
        })
    }

    /// Vlastnosti pro `op::FEATURES`. Bez F_FLUSH zarizeni nehlasi zadnou
    /// zaruku trvanlivosti: nevime jiste, ze nema mezipamet zapisu, a FS se
    /// pak radeji pripoji jen pro cteni, nez aby `fsync` lhal.
    fn features(&self) -> u64 {
        use blockproto::feature;
        let mut f = if self.flush { feature::FLUSH } else { 0 };
        if self.max_discard > 0 {
            f |= feature::DISCARD;
        }
        if self.read_only {
            f |= feature::READ_ONLY;
        }
        f
    }

    /// Provede jeden pozadavek virtio-blk a pocka na dokonceni. `data_len` =
    /// delka dat v bufferu (sektory, nebo popis useku u DISCARD).
    fn request(&mut self, kind: u32, lba: u64, data_len: u32) -> Result<()> {
        if self.failed {
            return Err(Error::IoError);
        }
        let r = self.request_inner(kind, lba, data_len);
        if matches!(r, Err(Error::TimedOut | Error::Corrupted)) {
            // Zarizeni zastavit, aby uz nepouzivalo DMA buffer.
            self.failed = true;
            let _ = self.device.reset();
            println!(
                "virtio-blk: zarizeni neodpovida spravne ({:?}), dalsi pozadavky odmitnuty",
                r
            );
            return Err(Error::IoError);
        }
        r
    }

    fn request_inner(&mut self, kind: u32, lba: u64, data_len: u32) -> Result<()> {
        self.request_at(kind, lba, data_len, self.buf_phys + DATA_OFF)
    }

    /// Pozadavek s daty na fyzicke adrese `data_phys` (normalne DMA buffer
    /// ovladace; test ochrany DMA pouziva jinou stranku).
    fn request_at(&mut self, kind: u32, lba: u64, data_len: u32, data_phys: u64) -> Result<()> {
        self.buf.write::<u32>(HDR_OFF, kind);
        self.buf.write::<u32>(HDR_OFF + 4, 0);
        self.buf.write::<u64>(HDR_OFF + 8, lba);
        self.buf.write::<u8>(STATUS_OFF, 0xFF);
        let mut chain = Vec::with_capacity(3);
        chain.push(Buffer {
            phys: self.buf_phys + HDR_OFF,
            len: 16,
            device_writes: false,
        });
        if data_len > 0 {
            chain.push(Buffer {
                phys: data_phys,
                len: data_len,
                device_writes: kind == REQ_IN,
            });
        }
        chain.push(Buffer {
            phys: self.buf_phys + STATUS_OFF,
            len: 1,
            device_writes: true,
        });
        self.queue.submit(&chain)?;
        self.device.notify(&self.queue);
        let deadline = libos::time::deadline_after(REQUEST_TIMEOUT);
        while self.queue.take_used()?.is_none() {
            self.irq.wait(Some(deadline))?;
        }
        match self.buf.read::<u8>(STATUS_OFF) {
            0 => Ok(()),
            2 => Err(Error::NotSupported),
            _ => Err(Error::Corrupted),
        }
    }
}

impl Disk {
    /// Test ochrany DMA: necha zarizeni precist sektor 0 do bezne stranky
    /// ovladace, ktera neni DMA pamet (s IOMMU neni v domene zarizeni).
    /// Vraci 1, kdyz stranka zustala nezmenena (zapis zablokovan), 0 kdyz ji
    /// zarizeni prepsalo (bez IOMMU). Zapis jde jen do vlastni pameti
    /// ovladace, takze test nic neohrozi.
    fn test_dma_protection(&mut self, resource: &Resource) -> Result<u64> {
        let probe = server::DmaProbe::new(resource, self.block)?;
        // Vysledek pozadavku je vedlejsi (zarizeni muze chybu hlasit i ne).
        let _ = self.request_at(REQ_IN, 0, self.block as u32, probe.phys);
        Ok(probe.untouched())
    }

    /// DISCARD po usecich podle limitu zarizeni. Popis useku (16 B):
    /// sektor u64, pocet u32, priznaky u32.
    fn discard_range(&mut self, lba: u64, count: u64) -> Result<()> {
        let mut done = 0u64;
        while done < count {
            let n = (count - done).min(self.max_discard as u64) as u32;
            self.buf.write::<u64>(DATA_OFF, lba + done);
            self.buf.write::<u32>(DATA_OFF + 8, n);
            self.buf.write::<u32>(DATA_OFF + 12, 0);
            self.request(REQ_DISCARD, 0, 16)?;
            done += n as u64;
        }
        Ok(())
    }
}

/// Logicky blok, ktery ovladac umi: mocnina dvou 512 B..=32 KiB (buffer).
fn block_ok(block: usize) -> bool {
    block.is_power_of_two() && (SECTOR..=blockproto::server::IO_MAX).contains(&block)
}

impl Disk {
    /// Sektoru virtio (512 B) v jednom logickem bloku.
    fn ratio(&self) -> u64 {
        (self.block / SECTOR) as u64
    }
}

impl Disks for Disk {
    fn count(&self) -> usize {
        1
    }

    fn sectors(&self, _: usize) -> u64 {
        self.capacity / self.ratio()
    }

    fn sector_size(&self, _: usize) -> usize {
        self.block
    }

    fn features(&self, _: usize) -> u64 {
        Disk::features(self)
    }

    fn max_read(&self, _: usize) -> usize {
        READ_MAX
    }

    fn read(&mut self, _: usize, lba: u64, out: &mut [u8]) -> Result<()> {
        self.request(REQ_IN, lba * self.ratio(), out.len() as u32)?;
        self.buf.read_bytes(DATA_OFF, out);
        Ok(())
    }

    fn write(&mut self, _: usize, lba: u64, data: &[u8]) -> Result<()> {
        self.buf.write_bytes(DATA_OFF, data);
        self.request(REQ_OUT, lba * self.ratio(), data.len() as u32)
    }

    fn flush(&mut self, _: usize) -> Result<()> {
        // Flush, ktery nic neudela, nesmi hlasit uspech.
        if !self.flush {
            return Err(Error::NotSupported);
        }
        self.request(REQ_FLUSH, 0, 0)
    }

    fn discard(&mut self, _: usize, lba: u64, count: u64) -> Result<()> {
        if self.max_discard == 0 {
            return Err(Error::NotSupported);
        }
        let r = self.ratio();
        self.discard_range(lba * r, count * r)
    }

    fn test_dma_protection(&mut self, _: usize) -> Result<u64> {
        let resource = self.resource.take().ok_or(Error::BadState)?;
        let r = Disk::test_dma_protection(self, &resource);
        self.resource = Some(resource);
        r
    }
}
