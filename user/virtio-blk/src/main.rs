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
//! klienta, dokud klient kanal nezavre.

#![no_std]
#![no_main]

extern crate alloc;

use alloc::vec::Vec;

use blockproto::{MAX_SECTORS, Request, SECTOR, encode_reply, op};
use libos::abi::vm::{MAP_READ, MAP_WRITE};
use libos::hw::{Interrupt, Resource};
use libos::{Channel, Error, Mapping, Result, Startup, Vmo, println};
use pci::msix::MsixTable;
use pci::{Address, Ecam};
use virtio::{Buffer, Device, F_VERSION_1, Virtqueue};

/// Vlastnost: zarizeni je jen pro cteni.
const F_RO: u64 = 1 << 5;
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

/// Offset `max_discard_sectors` v konfiguraci virtio-blk.
const CFG_MAX_DISCARD_SECTORS: u64 = 36;

/// Rozlozeni DMA bufferu pozadavku: hlavicka 16 B, stav 1 B, data.
const HDR_OFF: u64 = 0;
const STATUS_OFF: u64 = 16;
const DATA_OFF: u64 = 512;
const DMA_SIZE: u64 = DATA_OFF + MAX_SECTORS as u64 * SECTOR as u64;

libos::entry!(main);

struct Disk {
    device: Device,
    queue: Virtqueue,
    irq: Interrupt,
    _msix: MsixTable,
    _dma: Vmo,
    buf: Mapping,
    buf_phys: u64,
    capacity: u64,
    flush: bool,
    /// Zarizeni je jen pro cteni (F_RO).
    read_only: bool,
    /// Nejvic sektoru jednoho DISCARD (0 = DISCARD neumi).
    max_discard: u32,
    /// Zarizeni neodpovedelo nebo vratilo nesmysl: dalsi pozadavky vraci
    /// `IO_ERROR` (DMA buffer muze zarizeni jeste prepisovat).
    failed: bool,
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
        "virtio-blk {address}: {} MiB, flush {}, discard {}",
        disk.capacity * SECTOR as u64 / (1024 * 1024),
        if disk.flush { "ano" } else { "ne" },
        if disk.max_discard > 0 { "ano" } else { "ne" }
    );
    serve(&mut disk, &client, &resource);
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
            F_VERSION_1 | F_RO | F_FLUSH | F_DISCARD | F_ACCESS_PLATFORM,
            16,
        )?;

        let dma = resource.dma_memory(DMA_SIZE)?;
        let buf_phys = resource.phys_addr(&dma, 0)?;
        let buf = Mapping::new(&dma, 0, DMA_SIZE, MAP_READ | MAP_WRITE)?;
        // Kapacita v sektorech po 512 B je na zacatku konfigurace zarizeni.
        let capacity: u64 = device.config(0);
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
            flush: features & F_FLUSH != 0,
            read_only: features & F_RO != 0,
            max_discard,
            failed: false,
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

    fn handle(&mut self, req: Request, data: &[u8], resource: &Resource) -> (Result<u64>, Vec<u8>) {
        let range_ok = req.count <= MAX_SECTORS
            && req
                .lba
                .checked_add(req.count as u64)
                .is_some_and(|end| end <= self.capacity);
        match req.op {
            op::INFO => (Ok(self.capacity), Vec::new()),
            op::READ if range_ok => {
                let r = self.request(REQ_IN, req.lba, req.count * SECTOR as u32);
                let mut out = blockproto::zeroed_sectors(req.count);
                if r.is_ok() {
                    self.buf.read_bytes(DATA_OFF, &mut out);
                }
                (r.map(|_| 0), out)
            }
            op::WRITE if range_ok && data.len() == req.count as usize * SECTOR => {
                self.buf.write_bytes(DATA_OFF, data);
                (
                    self.request(REQ_OUT, req.lba, req.count * SECTOR as u32)
                        .map(|_| 0),
                    Vec::new(),
                )
            }
            op::FEATURES => (Ok(self.features()), Vec::new()),
            op::FLUSH if self.flush => (self.request(REQ_FLUSH, 0, 0).map(|_| 0), Vec::new()),
            // Flush, ktery nic neudela, nesmi hlasit uspech.
            op::FLUSH => (Err(Error::NotSupported), Vec::new()),
            op::DISCARD if self.max_discard == 0 => (Err(Error::NotSupported), Vec::new()),
            op::DISCARD
                if req
                    .lba
                    .checked_add(req.count as u64)
                    .is_some_and(|end| end <= self.capacity) =>
            {
                (self.discard(req.lba, req.count).map(|_| 0), Vec::new())
            }
            op::DISCARD => (Err(Error::OutOfRange), Vec::new()),
            op::TEST_DMA_PROTECTION => (self.test_dma_protection(resource), Vec::new()),
            op::READ | op::WRITE => (Err(Error::OutOfRange), Vec::new()),
            _ => (Err(Error::NotSupported), Vec::new()),
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
        const PATTERN: u8 = 0xAA;
        let page = Vmo::create(4096)?;
        let map = Mapping::new(&page, 0, 4096, MAP_READ | MAP_WRITE)?;
        map.write_bytes(0, &[PATTERN; 512]);
        let phys = resource.phys_addr(&page, 0)?;
        // Vysledek pozadavku je vedlejsi (zarizeni muze chybu hlasit i ne).
        let _ = self.request_at(REQ_IN, 0, SECTOR as u32, phys);
        let mut back = [0u8; 512];
        map.read_bytes(0, &mut back);
        Ok(back.iter().all(|&b| b == PATTERN) as u64)
    }

    /// DISCARD po usecich podle limitu zarizeni. Popis useku (16 B):
    /// sektor u64, pocet u32, priznaky u32.
    fn discard(&mut self, lba: u64, count: u32) -> Result<()> {
        let mut done = 0u32;
        while done < count {
            let n = (count - done).min(self.max_discard);
            self.buf.write::<u64>(DATA_OFF, lba + done as u64);
            self.buf.write::<u32>(DATA_OFF + 8, n);
            self.buf.write::<u32>(DATA_OFF + 12, 0);
            self.request(REQ_DISCARD, 0, 16)?;
            done += n;
        }
        Ok(())
    }
}

fn serve(disk: &mut Disk, client: &Channel, resource: &Resource) {
    loop {
        let msg = match client.read(None) {
            Ok(m) => m,
            Err(Error::PeerClosed) => return,
            Err(e) => {
                println!("virtio-blk: chyba kanalu: {e}");
                return;
            }
        };
        let (result, data) = match Request::decode(&msg.data) {
            Some((req, payload)) => disk.handle(req, payload, resource),
            None => (Err(Error::InvalidArgs), Vec::new()),
        };
        if client
            .write(&encode_reply(result, &data), Vec::new())
            .is_err()
        {
            return;
        }
    }
}
