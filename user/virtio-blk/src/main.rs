//! virtio-blk: ovladac virtualniho disku (virtio 1.x pres PCI).
//!
//! Spousti ho spravce zarizeni. Startovni zprava (kind USER, v tomto poradi):
//! 1. Resource (pristup k hardwaru),
//! 2. okno ECAM (konfigurace PCI),
//! 3. kanal pro klienta (protokol `blockproto`).
//!
//! Prvni zprava na bootstrap kanalu od spravce: `pci <arg_ecam> <sbernice>
//! <zarizeni> <funkce>` (desitkove). Ovladac pak obsluhuje pozadavky
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

/// Vlastnost: zarizeni podporuje FLUSH.
const F_FLUSH: u64 = 1 << 9;

const REQ_IN: u32 = 0;
const REQ_OUT: u32 = 1;
const REQ_FLUSH: u32 = 4;

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
}

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

    let Some((arg, address)) = read_config(&startup.bootstrap) else {
        println!("virtio-blk: neplatna konfigurace od spravce zarizeni");
        return 2;
    };
    let disk = Ecam::new(&ecam_vmo, arg).and_then(|ecam| Disk::open(&ecam, address, &resource));
    let mut disk = match disk {
        Ok(d) => d,
        Err(e) => {
            println!("virtio-blk {address}: inicializace selhala: {e}");
            return 3;
        }
    };
    println!(
        "virtio-blk {address}: {} MiB, flush {}",
        disk.capacity * SECTOR as u64 / (1024 * 1024),
        if disk.flush { "ano" } else { "ne" }
    );
    serve(&mut disk, &client);
    0
}

fn read_config(bootstrap: &Channel) -> Option<(u32, Address)> {
    let msg = bootstrap.read(None).ok()?;
    let text = core::str::from_utf8(&msg.data).ok()?;
    let mut parts = text.split_whitespace();
    if parts.next()? != "pci" {
        return None;
    }
    let mut num = || parts.next()?.parse::<u32>().ok();
    let arg = num()?;
    let address = Address {
        bus: num()? as u8,
        device: num()? as u8,
        function: num()? as u8,
    };
    Some((arg, address))
}

impl Disk {
    fn open(ecam: &Ecam, a: Address, resource: &Resource) -> Result<Self> {
        let device = Device::open(ecam, a, resource)?;
        device.begin_init();
        let features = device.negotiate(F_VERSION_1 | F_FLUSH)?;

        let irq = resource.interrupt()?;
        let msix = MsixTable::open(ecam, a, resource)?;
        msix.set(0, irq.msi())?;
        msix.enable(ecam);
        let queue = device.setup_queue(0, 16, 0, resource)?;
        device.driver_ok();

        let dma = resource.dma_memory(DMA_SIZE)?;
        let buf_phys = resource.phys_addr(&dma, 0)?;
        let buf = Mapping::new(&dma, 0, DMA_SIZE, MAP_READ | MAP_WRITE)?;
        // Kapacita v sektorech po 512 B je na zacatku konfigurace zarizeni.
        let capacity: u64 = device.config(0);
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
        })
    }

    /// Provede jeden pozadavek virtio-blk a pocka na dokonceni.
    fn request(&mut self, kind: u32, lba: u64, sectors: u32) -> Result<()> {
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
        if sectors > 0 {
            chain.push(Buffer {
                phys: self.buf_phys + DATA_OFF,
                len: sectors * SECTOR as u32,
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
        while self.queue.take_used().is_none() {
            self.irq.wait(None)?;
        }
        match self.buf.read::<u8>(STATUS_OFF) {
            0 => Ok(()),
            2 => Err(Error::NotSupported),
            _ => Err(Error::Corrupted),
        }
    }

    fn handle(&mut self, req: Request, data: &[u8]) -> (Result<u64>, Vec<u8>) {
        let range_ok = req.count <= MAX_SECTORS
            && req
                .lba
                .checked_add(req.count as u64)
                .is_some_and(|end| end <= self.capacity);
        match req.op {
            op::INFO => (Ok(self.capacity), Vec::new()),
            op::READ if range_ok => {
                let r = self.request(REQ_IN, req.lba, req.count);
                let mut out = blockproto::zeroed_sectors(req.count);
                if r.is_ok() {
                    self.buf.read_bytes(DATA_OFF, &mut out);
                }
                (r.map(|_| 0), out)
            }
            op::WRITE if range_ok && data.len() == req.count as usize * SECTOR => {
                self.buf.write_bytes(DATA_OFF, data);
                (
                    self.request(REQ_OUT, req.lba, req.count).map(|_| 0),
                    Vec::new(),
                )
            }
            op::FLUSH if self.flush => (self.request(REQ_FLUSH, 0, 0).map(|_| 0), Vec::new()),
            op::FLUSH => (Ok(0), Vec::new()),
            op::READ | op::WRITE => (Err(Error::OutOfRange), Vec::new()),
            _ => (Err(Error::NotSupported), Vec::new()),
        }
    }
}

fn serve(disk: &mut Disk, client: &Channel) {
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
            Some((req, payload)) => disk.handle(req, payload),
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
