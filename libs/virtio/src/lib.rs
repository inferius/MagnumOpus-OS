//! Virtio 1.x pres PCI (moderni rozhrani): spolecna konfigurace, notifikace,
//! konfigurace zarizeni a fronty (split virtqueue).
//!
//! Specifikace: "Virtual I/O Device (VIRTIO) Version 1.2", kapitoly 2
//! (fronty), 4.1 (PCI transport).

#![no_std]

extern crate alloc;

mod queue;

pub use queue::{Buffer, Virtqueue};

use alloc::vec::Vec;

use libos::abi::vm::{MAP_READ, MAP_WRITE, PAGE_SIZE};
use libos::hw::Resource;
use libos::{Error, Mapping, Result};
use pci::{Address, CAP_VENDOR, Ecam};

pub const VENDOR: u16 = 0x1AF4;

/// Druhy capability virtio (cfg_type).
const CFG_COMMON: u8 = 1;
const CFG_NOTIFY: u8 = 2;
const CFG_ISR: u8 = 3;
const CFG_DEVICE: u8 = 4;

/// Stavove bity zarizeni.
pub const STATUS_ACKNOWLEDGE: u8 = 1;
pub const STATUS_DRIVER: u8 = 2;
pub const STATUS_DRIVER_OK: u8 = 4;
pub const STATUS_FEATURES_OK: u8 = 8;
pub const STATUS_FAILED: u8 = 128;

/// Vlastnost "virtio 1.0+" (bit 32) - bez ni zarizeni moderni rozhrani neumi.
pub const F_VERSION_1: u64 = 1 << 32;

// Offsety ve spolecne konfiguraci.
const DEVICE_FEATURE_SELECT: u64 = 0x00;
const DEVICE_FEATURE: u64 = 0x04;
const DRIVER_FEATURE_SELECT: u64 = 0x08;
const DRIVER_FEATURE: u64 = 0x0C;
const MSIX_CONFIG: u64 = 0x10;
const DEVICE_STATUS: u64 = 0x14;
const QUEUE_SELECT: u64 = 0x16;
const QUEUE_SIZE: u64 = 0x18;
const QUEUE_MSIX_VECTOR: u64 = 0x1A;
const QUEUE_ENABLE: u64 = 0x1C;
const QUEUE_NOTIFY_OFF: u64 = 0x1E;
const QUEUE_DESC: u64 = 0x20;
const QUEUE_DRIVER: u64 = 0x28;
const QUEUE_DEVICE: u64 = 0x30;

/// Hodnota "zadny vektor MSI-X".
pub const NO_VECTOR: u16 = 0xFFFF;

/// Okno do registru zarizeni (cast BARu podle capability).
struct Window {
    map: Mapping,
    base: u64,
    /// Delka struktury podle capability: pristup za ni je chyba zarizeni.
    len: u64,
}

impl Window {
    fn open(
        ecam: &Ecam,
        a: Address,
        resource: &Resource,
        bar: u8,
        offset: u32,
        len: u32,
    ) -> Result<Self> {
        let bar = ecam.memory_bar(a, bar).ok_or(Error::NotSupported)?;
        let phys = bar
            .phys
            .checked_add(offset as u64)
            .ok_or(Error::Corrupted)?;
        let page = phys & !(PAGE_SIZE - 1);
        let size = (phys - page + len as u64).next_multiple_of(PAGE_SIZE);
        let vmo = resource.device_memory(page, size)?;
        let map = Mapping::new(&vmo, 0, size, MAP_READ | MAP_WRITE)?;
        Ok(Self {
            map,
            base: phys - page,
            len: len as u64,
        })
    }

    fn fits<T>(&self, off: u64) -> bool {
        off.checked_add(core::mem::size_of::<T>() as u64)
            .is_some_and(|e| e <= self.len)
    }

    /// Cteni registru; mimo strukturu (vadne zarizeni) vraci 0.
    fn read<T: libos::Pod + Default>(&self, off: u64) -> T {
        if self.fits::<T>(off) {
            self.map.read(self.base + off)
        } else {
            T::default()
        }
    }

    /// Zapis registru; mimo strukturu se ignoruje.
    fn write<T: libos::Pod>(&self, off: u64, v: T) {
        if self.fits::<T>(off) {
            self.map.write(self.base + off, v)
        }
    }
}

/// Zarizeni virtio na PCI.
pub struct Device {
    common: Window,
    notify: Window,
    notify_multiplier: u32,
    device_cfg: Window,
    _isr: Window,
}

impl Device {
    /// Najde capability virtio a namapuje jejich registry.
    pub fn open(ecam: &Ecam, a: Address, resource: &Resource) -> Result<Self> {
        let mut found: [Option<(u8, u32, u32, u16)>; 5] = [None; 5];
        for (off, id) in ecam.capabilities(a) {
            if id != CAP_VENDOR {
                continue;
            }
            let cfg_type = ecam.read8(a, off + 3);
            if (1..=4).contains(&cfg_type) && found[cfg_type as usize].is_none() {
                let bar = ecam.read8(a, off + 4);
                let offset = ecam.read32(a, off + 8);
                let length = ecam.read32(a, off + 12);
                found[cfg_type as usize] = Some((bar, offset, length, off));
            }
        }
        let get = |t: u8| found[t as usize].ok_or(Error::NotSupported);
        let (cb, co, cl, _) = get(CFG_COMMON)?;
        let (nb, no, nl, ncap) = get(CFG_NOTIFY)?;
        let (ib, io, il, _) = get(CFG_ISR)?;
        let (db, dof, dl, _) = get(CFG_DEVICE)?;
        let notify_multiplier = ecam.read32(a, ncap + 16);
        ecam.enable(a);
        Ok(Self {
            common: Window::open(ecam, a, resource, cb, co, cl)?,
            notify: Window::open(ecam, a, resource, nb, no, nl)?,
            notify_multiplier,
            device_cfg: Window::open(ecam, a, resource, db, dof, dl)?,
            _isr: Window::open(ecam, a, resource, ib, io, il)?,
        })
    }

    pub fn status(&self) -> u8 {
        self.common.read(DEVICE_STATUS)
    }

    pub fn set_status(&self, s: u8) {
        self.common.write(DEVICE_STATUS, s)
    }

    /// Reset zarizeni: zastavi ho (prestane pouzivat fronty a DMA buffery).
    pub fn reset(&self) -> Result<()> {
        self.set_status(0);
        let deadline = libos::time::deadline_after(core::time::Duration::from_secs(1));
        while self.status() != 0 {
            if libos::time::now() > deadline {
                return Err(Error::TimedOut);
            }
            core::hint::spin_loop();
        }
        Ok(())
    }

    /// Reset a zahajeni inicializace (ACKNOWLEDGE | DRIVER). Zarizeni, ktere
    /// reset nedokonci do sekundy, je vadne (`TIMED_OUT`).
    pub fn begin_init(&self) -> Result<()> {
        self.reset()?;
        self.set_status(STATUS_ACKNOWLEDGE);
        self.set_status(STATUS_ACKNOWLEDGE | STATUS_DRIVER);
        Ok(())
    }

    pub fn device_features(&self) -> u64 {
        self.common.write::<u32>(DEVICE_FEATURE_SELECT, 0);
        let low = self.common.read::<u32>(DEVICE_FEATURE) as u64;
        self.common.write::<u32>(DEVICE_FEATURE_SELECT, 1);
        let high = self.common.read::<u32>(DEVICE_FEATURE) as u64;
        low | high << 32
    }

    /// Nastavi vlastnosti, ktere ovladac pouziva (prunik s nabidkou
    /// zarizeni), a overi, ze je zarizeni prijalo.
    pub fn negotiate(&self, wanted: u64) -> Result<u64> {
        let features = self.device_features() & wanted;
        if features & F_VERSION_1 == 0 {
            self.set_status(STATUS_FAILED);
            return Err(Error::NotSupported);
        }
        self.common.write::<u32>(DRIVER_FEATURE_SELECT, 0);
        self.common.write::<u32>(DRIVER_FEATURE, features as u32);
        self.common.write::<u32>(DRIVER_FEATURE_SELECT, 1);
        self.common
            .write::<u32>(DRIVER_FEATURE, (features >> 32) as u32);
        let s = self.status() | STATUS_FEATURES_OK;
        self.set_status(s);
        if self.status() & STATUS_FEATURES_OK == 0 {
            self.set_status(STATUS_FAILED);
            return Err(Error::NotSupported);
        }
        self.common.write::<u16>(MSIX_CONFIG, NO_VECTOR);
        Ok(features)
    }

    /// Nastavi frontu `index` (max. `max_size` polozek) s prerusenim na
    /// polozce MSI-X `vector`.
    pub fn setup_queue(
        &self,
        index: u16,
        max_size: u16,
        vector: u16,
        resource: &Resource,
    ) -> Result<Virtqueue> {
        self.common.write::<u16>(QUEUE_SELECT, index);
        let size = self.common.read::<u16>(QUEUE_SIZE).min(max_size);
        if size == 0 {
            return Err(Error::NotFound);
        }
        self.common.write::<u16>(QUEUE_SIZE, size);
        let notify_off =
            self.common.read::<u16>(QUEUE_NOTIFY_OFF) as u64 * self.notify_multiplier as u64;
        let queue = Virtqueue::new(size, resource, index, notify_off)?;
        let (desc, driver, device) = queue.phys_addresses();
        self.common.write::<u64>(QUEUE_DESC, desc);
        self.common.write::<u64>(QUEUE_DRIVER, driver);
        self.common.write::<u64>(QUEUE_DEVICE, device);
        self.common.write::<u16>(QUEUE_MSIX_VECTOR, vector);
        if self.common.read::<u16>(QUEUE_MSIX_VECTOR) != vector {
            return Err(Error::NoResources);
        }
        self.common.write::<u16>(QUEUE_ENABLE, 1);
        Ok(queue)
    }

    /// Dokonceni inicializace.
    pub fn driver_ok(&self) {
        let s = self.status() | STATUS_DRIVER_OK;
        self.set_status(s);
    }

    /// Upozorni zarizeni na nove pozadavky ve fronte.
    pub fn notify(&self, queue: &Virtqueue) {
        self.notify
            .write::<u16>(queue.notify_offset(), queue.index());
    }

    /// Cteni konfigurace zarizeni (vyznam podle typu zarizeni); za koncem
    /// konfigurace, kterou zarizeni hlasi, vraci 0.
    pub fn config<T: libos::Pod + Default>(&self, off: u64) -> T {
        self.device_cfg.read(off)
    }

    /// Delka konfigurace zarizeni (volitelna pole za ni zarizeni nema).
    pub fn config_len(&self) -> u64 {
        self.device_cfg.len
    }
}

/// Seznam funkci, ktere jsou zarizeni virtio daneho typu (ID zarizeni
/// moderniho `0x1040 + typ` nebo prechodneho rozhrani).
pub fn find(functions: &[pci::Function], modern_id: u16, transitional_id: u16) -> Vec<Address> {
    functions
        .iter()
        .filter(|f| f.vendor == VENDOR && (f.device == modern_id || f.device == transitional_id))
        .map(|f| f.address)
        .collect()
}
