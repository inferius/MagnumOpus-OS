//! virtio-input: ovladac klavesnice, mysi a dalsich vstupnich zarizeni
//! virtio 1.x pres PCI (v QEMU `virtio-keyboard-pci`, `virtio-mouse-pci`).
//!
//! Spousti ho spravce zarizeni. Startovni zprava (kind USER, v tomto poradi):
//! 1. omezeny Resource (jen BARy a konfigurace tohoto zarizeni, kvoty
//!    preruseni a DMA),
//! 2. stranka konfiguracniho prostoru PCI teto funkce,
//! 3. kanal ke sluzbe vstupu (protokol `inputproto::device`).
//!
//! Prvni zprava na bootstrap kanalu od spravce: `pci <sbernice> <zarizeni>
//! <funkce>` (desitkove). Ovladac posle sluzbe predstaveni (`IDEV`: druh
//! zarizeni a jmeno) a pak preposila udalosti zarizeni (`IEVT`), dokud
//! sluzba kanal nezavre. Udalosti virtio-input maji uz tvar evdev, ovladac
//! je jen prenese z fronty do kanalu.
//!
//! Specifikace: VIRTIO 1.2, kapitola 5.8 (Input Device).

#![no_std]
#![no_main]

extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;

use inputproto::code::{EV_KEY, EV_REL, KEY_A};
use inputproto::device::{self, RawEvent, caps};
use libos::abi::vm::{MAP_READ, MAP_WRITE};
use libos::hw::{Interrupt, Resource};
use libos::{Channel, Error, Mapping, Result, Signals, Startup, Vmo, println};
use pci::msix::MsixTable;
use pci::{Address, Ecam};
use virtio::{Buffer, Device, F_VERSION_1, Virtqueue};

/// Vlastnost: DMA pres preklad platformy (IOMMU), viz virtio-blk.
const F_ACCESS_PLATFORM: u64 = 1 << 33;

// Konfigurace virtio-input: select (u8), subsel (u8), size (u8), rezerva,
// od offsetu 8 data (retezec nebo bitmapa, 128 B).
const CFG_SELECT: u64 = 0;
const CFG_SUBSEL: u64 = 1;
const CFG_SIZE: u64 = 2;
const CFG_DATA: u64 = 8;
const CFG_ID_NAME: u8 = 0x01;
const CFG_EV_BITS: u8 = 0x11;

/// Nejvic bufferu udalosti ve fronte (kazdy 8 B).
const QUEUE_MAX: u16 = 64;

libos::entry!(main);

struct InputDevice {
    device: Device,
    queue: Virtqueue,
    irq: Interrupt,
    _msix: MsixTable,
    _dma: Vmo,
    buf: Mapping,
    buf_phys: u64,
    caps: u16,
    name: String,
    /// Prvni udalosti uz byly zalogovane.
    logged: bool,
}

fn main(startup: Startup) -> i64 {
    let mut handles = startup.handles.into_iter();
    let (Some(res), Some(ecam_vmo), Some(service)) =
        (handles.next(), handles.next(), handles.next())
    else {
        println!("virtio-input: chybi handly ve startovni zprave");
        return 1;
    };
    let resource = Resource::from_handle(res);
    let ecam_vmo = Vmo::from_handle(ecam_vmo);
    let service = Channel::from_handle(service);

    let Some(address) = read_config(&startup.bootstrap) else {
        println!("virtio-input: neplatna konfigurace od spravce zarizeni");
        return 2;
    };
    let dev = Ecam::single(&ecam_vmo, address)
        .and_then(|ecam| InputDevice::open(&ecam, address, &resource));
    let mut dev = match dev {
        Ok(d) => d,
        Err(e) => {
            println!("virtio-input {address}: inicializace selhala: {e}");
            return 3;
        }
    };
    println!(
        "virtio-input {address}: '{}' ({})",
        dev.name,
        match (dev.caps & caps::KEYBOARD != 0, dev.caps & caps::POINTER != 0) {
            (true, true) => "klavesnice a mys",
            (true, false) => "klavesnice",
            (false, true) => "mys",
            (false, false) => "nezname zarizeni",
        }
    );
    if service
        .write(&device::encode_hello(dev.caps, &dev.name), Vec::new())
        .is_err()
    {
        return 0;
    }
    let r = dev.serve(&service);
    // Zarizeni zastavit, aby uz nepouzivalo DMA buffery.
    let _ = dev.device.reset();
    match r {
        Ok(()) => 0,
        Err(e) => {
            println!("virtio-input {address}: {e}");
            4
        }
    }
}

fn read_config(bootstrap: &Channel) -> Option<Address> {
    let msg = bootstrap.read(None).ok()?;
    let text = core::str::from_utf8(&msg.data).ok()?;
    let mut parts = text.split_whitespace();
    if parts.next()? != "pci" {
        return None;
    }
    let mut num = || parts.next()?.parse::<u8>().ok();
    Some(Address {
        bus: num()?,
        device: num()?,
        function: num()?,
    })
}

impl InputDevice {
    fn open(ecam: &Ecam, a: Address, resource: &Resource) -> Result<Self> {
        let device = Device::open(ecam, a, resource)?;
        device.begin_init()?;
        device.negotiate(F_VERSION_1 | F_ACCESS_PLATFORM)?;

        let irq = resource.interrupt()?;
        let msix = MsixTable::open(ecam, a, resource)?;
        msix.set(0, irq.msi())?;
        msix.enable(ecam);
        // Fronta 0 = udalosti (zarizeni -> ovladac). Fronta 1 (stav,
        // napr. kontrolky Caps Lock) zatim nepouzivame.
        let queue = device.setup_queue(0, QUEUE_MAX, 0, resource)?;
        let name = read_name(&device);
        let caps = read_caps(&device);
        device.driver_ok();

        let size = queue.size() as u64 * RawEvent::SIZE as u64;
        let dma = resource.dma_memory(size)?;
        let buf_phys = resource.phys_addr(&dma, 0)?;
        let buf = Mapping::new(&dma, 0, size, MAP_READ | MAP_WRITE)?;
        let mut dev = Self {
            device,
            queue,
            irq,
            _msix: msix,
            _dma: dma,
            buf,
            buf_phys,
            caps,
            name,
            logged: false,
        };
        for id in 0..dev.queue.size() {
            dev.post(id)?;
        }
        dev.device.notify(&dev.queue);
        Ok(dev)
    }

    /// Vrati buffer `id` zarizeni k naplneni.
    fn post(&mut self, id: u16) -> Result<()> {
        self.queue.post(
            id,
            Buffer {
                phys: self.buf_phys + id as u64 * RawEvent::SIZE as u64,
                len: RawEvent::SIZE as u32,
                device_writes: true,
            },
        )
    }

    /// Preposila udalosti, dokud sluzba vstupu nezavre kanal.
    fn serve(&mut self, service: &Channel) -> Result<()> {
        let mut dropped = 0u64;
        loop {
            let items = [
                (self.irq.handle(), Signals::SIGNALED),
                (service.handle(), Signals::PEER_CLOSED),
            ];
            let (_, observed) = libos::wait_many(&items, None)?;
            if observed[1].intersects(Signals::PEER_CLOSED) {
                return Ok(());
            }
            // Potvrdit driv nez vyber fronty: preruseni behem vyberu se
            // neztrati.
            self.irq.ack()?;
            let mut events = Vec::new();
            while let Some((id, len)) = self.queue.take_posted()? {
                if len as usize >= RawEvent::SIZE {
                    let mut b = [0u8; RawEvent::SIZE];
                    self.buf
                        .read_bytes(id as u64 * RawEvent::SIZE as u64, &mut b);
                    if let Some(e) = RawEvent::from_bytes(&b) {
                        events.push(e);
                    }
                }
                self.post(id)?;
            }
            self.device.notify(&self.queue);
            if !events.is_empty() && !self.logged {
                self.logged = true;
                println!("virtio-input: prvni udalosti od zarizeni ({})", events.len());
            }
            for chunk in events.chunks(device::MAX_EVENTS) {
                match service.write(&device::encode_events(chunk), Vec::new()) {
                    Ok(()) => {}
                    // Sluzba nestiha: udalosti zahodit, neblokovat zarizeni.
                    Err(Error::ShouldWait) => {
                        dropped += 1;
                        if dropped.is_power_of_two() {
                            println!("virtio-input: sluzba nestiha, zahozeno {dropped} zprav");
                        }
                    }
                    Err(Error::PeerClosed) => return Ok(()),
                    Err(e) => return Err(e),
                }
            }
        }
    }
}

/// Vybere polozku konfigurace a vrati jeji delku.
fn select(device: &Device, sel: u8, subsel: u8) -> u8 {
    device.set_config::<u8>(CFG_SELECT, sel);
    device.set_config::<u8>(CFG_SUBSEL, subsel);
    device.config::<u8>(CFG_SIZE).min(128)
}

/// Jmeno zarizeni (napr. "QEMU Virtio Keyboard"); neplatne UTF-8 se
/// nahradi.
fn read_name(device: &Device) -> String {
    let size = select(device, CFG_ID_NAME, 0);
    let bytes: Vec<u8> = (0..size as u64)
        .map(|i| device.config::<u8>(CFG_DATA + i))
        .take_while(|&b| b != 0)
        .collect();
    String::from_utf8(bytes).unwrap_or_else(|_| String::from("?"))
}

/// Druh zarizeni podle podporovanych udalosti: klavesy pismen =
/// klavesnice, relativni osy = mys.
fn read_caps(device: &Device) -> u16 {
    let mut c = 0;
    let key_bits = select(device, CFG_EV_BITS, EV_KEY as u8);
    if key_bits as u16 > KEY_A / 8 {
        let byte = device.config::<u8>(CFG_DATA + (KEY_A / 8) as u64);
        if byte & (1 << (KEY_A % 8)) != 0 {
            c |= caps::KEYBOARD;
        }
    }
    if select(device, CFG_EV_BITS, EV_REL as u8) > 0 {
        c |= caps::POINTER;
    }
    c
}
