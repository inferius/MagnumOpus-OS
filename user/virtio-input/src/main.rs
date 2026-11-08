//! virtio-input: ovladac klavesnice, mysi a dalsich vstupnich zarizeni
//! virtio 1.x pres PCI (v QEMU `virtio-keyboard-pci`, `virtio-mouse-pci`).
//!
//! Spousti ho spravce zarizeni; start, smycku udalosti a zastaveni
//! zarizeni pri konci dela driverkit (`driverkit::os`). Startovni zprava:
//! Resource zarizeni (jen jeho BARy a konfigurace, kvoty preruseni a DMA),
//! stranka konfigurace PCI a kanal ke sluzbe vstupu (protokol
//! `inputproto::device`).
//!
//! Ovladac posle sluzbe predstaveni (`IDEV`: druh zarizeni a jmeno) a pak
//! preposila udalosti zarizeni (`IEVT`). Na synchronizaci nove instance
//! sluzby (`ISYN`) odpovi a predstavi se znovu; kdyz sluzba kanal zavre,
//! nekonci (obnova sluzeb, viz `docs/cs/obnova-sluzeb.md`;
//! `Driver::client_closed`). Udalosti virtio-input maji uz tvar evdev,
//! ovladac je jen prenese z fronty do kanalu.
//!
//! Specifikace: VIRTIO 1.2, kapitola 5.8 (Input Device).

#![no_std]
#![no_main]

extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;

use driverkit::os::{OsDma, OsPlatform};
use driverkit::{DmaBuffer, Driver, DriverError, IrqId, Platform, Result, Sent};
use inputproto::code::{ABS_X, ABS_Y, EV_ABS, EV_KEY, EV_REL, KEY_A};
use inputproto::device::{self, AxisInfo, RawEvent, caps};
use libos::Startup;
use pci::msix::MsixTable;
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
const CFG_ABS_INFO: u8 = 0x12;

/// Nejvic bufferu udalosti ve fronte (kazdy 8 B).
const QUEUE_MAX: u16 = 64;

libos::entry!(main);

fn main(startup: Startup) -> i64 {
    driverkit::os::main::<InputDevice>(startup, "virtio-input")
}

struct InputDevice {
    device: Device,
    queue: Virtqueue,
    _msix: MsixTable,
    buf: DmaBuffer<OsDma>,
    caps: u16,
    name: String,
    /// Predstaveni (`IDEV`): pri startu a znovu na `ISYN`.
    hello: Vec<u8>,
    /// Rozsahy absolutnich os (`IAXS`) u tabletu; bez nich by sluzba
    /// vstupu pocitala s 0..65535 a ukazatel by se dostal jen do casti
    /// obrazovky (QEMU tablet ma 0..32767).
    axes: Option<Vec<u8>>,
    /// Prvni udalosti uz byly zalogovane.
    logged: bool,
    /// Zpravy zahozene, protoze sluzba nestihala.
    dropped: u64,
}

impl Driver<OsPlatform> for InputDevice {
    fn probe(p: &OsPlatform) -> Result<Self> {
        // Fronta 0 = udalosti (zarizeni -> ovladac). Fronta 1 (stav,
        // napr. kontrolky Caps Lock) zatim nepouzivame.
        let virtio::Started {
            device,
            queue,
            irq,
            msix,
            ..
        } = virtio::start(p.ecam(), p.address(), p.resource(), F_VERSION_1 | F_ACCESS_PLATFORM, QUEUE_MAX)
            .map_err(DriverError::os("inicializace virtio"))?;
        p.adopt_irq(irq);
        let name = read_name(&device);
        let mut caps = read_caps(&device);
        let axes = read_axes(&device);
        if !axes.is_empty() {
            caps |= caps::ABSOLUTE;
        }
        let axes = (!axes.is_empty()).then(|| device::encode_axes(&axes));
        let buf = p.alloc_dma("udalosti", queue.size() as u64 * RawEvent::SIZE as u64)?;
        let hello = device::encode_hello(caps, &name);
        let mut dev = Self {
            device,
            queue,
            _msix: msix,
            buf,
            caps,
            name,
            hello,
            axes,
            logged: false,
            dropped: 0,
        };
        for id in 0..dev.queue.size() {
            dev.post(id)?;
        }
        dev.device.notify(&dev.queue);
        Ok(dev)
    }

    fn start(&mut self, p: &OsPlatform) -> Result<()> {
        let kind = match (self.caps & caps::KEYBOARD != 0, self.caps & caps::POINTER != 0) {
            _ if self.axes.is_some() => "tablet",
            (true, true) => "klavesnice a mys",
            (true, false) => "klavesnice",
            (false, true) => "mys",
            (false, false) => "nezname zarizeni",
        };
        driverkit::log!(p, "'{}' ({kind})", self.name);
        p.send(&self.hello)?;
        if let Some(axes) = &self.axes {
            p.send(axes)?;
        }
        Ok(())
    }

    /// Synchronizace nove instance sluzby vstupu (`ISYN`): odpoved
    /// a znovu predstaveni.
    fn message(&mut self, p: &OsPlatform, msg: &[u8]) -> Result<()> {
        device::answer_sync(msg, &self.hello, self.axes.as_deref(), |b| p.send(b).is_ok_and(|s| s == Sent::Sent));
        Ok(())
    }

    /// Sluzba vstupu zavrela kanal: ovladac nekonci (obnova sluzeb), dal
    /// obsluhuje zarizeni a udalosti zahazuje, az ho ukonci spravce.
    fn client_closed(&mut self, p: &OsPlatform) -> bool {
        driverkit::log!(p, "sluzba vstupu zavrela kanal, cekam");
        true
    }

    /// Preposle udalosti z fronty sluzbe vstupu.
    fn interrupt(&mut self, p: &OsPlatform, _irq: IrqId) -> Result<()> {
        let mut events = Vec::new();
        while let Some((id, len)) = self.queue.take_posted().map_err(corrupted)? {
            if len as usize >= RawEvent::SIZE {
                let mut b = [0u8; RawEvent::SIZE];
                self.buf.read_bytes(id as u64 * RawEvent::SIZE as u64, &mut b)?;
                if let Some(e) = RawEvent::from_bytes(&b) {
                    events.push(e);
                }
            }
            self.post(id)?;
        }
        self.device.notify(&self.queue);
        if !events.is_empty() && !self.logged {
            self.logged = true;
            driverkit::log!(p, "prvni udalosti od zarizeni ({})", events.len());
        }
        for chunk in events.chunks(device::MAX_EVENTS) {
            // Sluzba nestiha (nebo se restartuje): udalosti zahodit,
            // neblokovat zarizeni. Bez sluzby (`NoClient`) tise zahodit.
            if p.send(&device::encode_events(chunk))? == Sent::Full {
                self.dropped += 1;
                if self.dropped.is_power_of_two() {
                    driverkit::log!(p, "sluzba nestiha, zahozeno {} zprav", self.dropped);
                }
            }
        }
        Ok(())
    }

    fn stop(&mut self, _p: &OsPlatform) {
        // Zarizeni zastavit, aby uz nepouzivalo DMA buffery.
        let _ = self.device.reset();
    }
}

fn corrupted(e: libos::Error) -> DriverError {
    DriverError::Device(alloc::format!("fronta udalosti: {e}"))
}

impl InputDevice {
    /// Vrati buffer `id` zarizeni k naplneni.
    fn post(&mut self, id: u16) -> Result<()> {
        let size = RawEvent::SIZE as u64;
        let phys = self.buf.iova_of(id as u64 * size, size)?;
        self.queue
            .post(
                id,
                Buffer {
                    phys,
                    len: RawEvent::SIZE as u32,
                    device_writes: true,
                },
            )
            .map_err(corrupted)
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
/// Rozsahy os `ABS_X` a `ABS_Y` (`VIRTIO_INPUT_CFG_ABS_INFO`: min, max,
/// fuzz, flat, res jako u32), kdyz je zarizeni hlasi.
fn read_axes(device: &Device) -> Vec<AxisInfo> {
    let mut out = Vec::new();
    if select(device, CFG_EV_BITS, EV_ABS as u8) == 0 {
        return out;
    }
    let bits = device.config::<u8>(CFG_DATA);
    for code in [ABS_X, ABS_Y] {
        if bits & (1 << code) == 0 || select(device, CFG_ABS_INFO, code as u8) < 8 {
            continue;
        }
        let min = device.config::<u32>(CFG_DATA) as i32;
        let max = device.config::<u32>(CFG_DATA + 4) as i32;
        if max > min {
            out.push(AxisInfo { code, min, max });
        }
    }
    out
}

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
