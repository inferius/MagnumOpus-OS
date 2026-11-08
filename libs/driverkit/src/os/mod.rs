//! Backend nad jadrem (feature `os`): [`OsPlatform`].
//!
//! Startovni zprava ovladace od spravce zarizeni (kind USER, v poradi):
//! 1. omezeny Resource (jen BARy z manifestu, konfigurace funkce, kvoty
//!    preruseni a DMA; s IOMMU vlastni domena zarizeni),
//! 2. stranka konfiguracniho prostoru PCI teto funkce,
//! 3. (volitelne) kanal ke klientovi: init nebo sluzba (vstup, sit, ...).
//!
//! Prvni zprava na bootstrap kanalu: [`crate::config`].
//!
//! Program ovladace je pak jen:
//!
//! ```ignore
//! libos::entry!(main);
//! fn main(startup: libos::Startup) -> i64 {
//!     driverkit::os::main::<MujOvladac<_>>(startup, "muj")
//! }
//! ```

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use core::cell::{Cell, RefCell};
use core::fmt;

use libos::abi::vm::{MAP_READ, MAP_WRITE};
use libos::hw::{Interrupt, Resource};
use libos::{Channel, Error, Mapping, Signals, Startup, Vmo, println};
use pci::msix::MsixTable;
use pci::{Address, Ecam};

use crate::config::{self, DriverConfig};
use crate::dma::{DmaBuffer, DmaMemory};
use crate::io::{Io, fits};
use crate::platform::{DeviceInfo, Event, IrqId, Platform, Sent};
use crate::{Driver, DriverError, Result};

/// Jak dlouho cekat na konfiguraci od spravce zarizeni.
const CONFIG_TIMEOUT: core::time::Duration = core::time::Duration::from_secs(5);

/// Rozhrani ovladace k jadru.
pub struct OsPlatform {
    /// Handle vlastniho procesu: libos ho pouziva pro kazde mapovani
    /// pameti (zavrit ho = dalsi `Mapping::new` selze s BAD_HANDLE).
    _process: libos::Process,
    /// Spojeni s rodicem (spravce zarizeni).
    _bootstrap: Channel,
    name: &'static str,
    info: DeviceInfo,
    address: Address,
    resource: Resource,
    ecam: Ecam,
    _config_vmo: Vmo,
    client: Option<Channel>,
    config: DriverConfig,
    irqs: RefCell<Vec<Interrupt>>,
    msix: RefCell<Option<MsixTable>>,
    msi: Cell<bool>,
}

/// Namapovany BAR.
pub struct OsMmio {
    map: Mapping,
    /// Offset BARu v mapovani (BAR nemusi zacinat na strance).
    base: u64,
    len: u64,
}

impl OsMmio {
    /// Okno `len` bajtu od fyzicke adresy `phys` (pro knihovny nad
    /// `OsPlatform`, ktere znaji sva okna, napr. capability virtio).
    pub fn map(resource: &Resource, phys: u64, len: u64) -> libos::Result<Self> {
        let page_size = libos::vm::page_size();
        let page = phys & !(page_size - 1);
        let size = (phys - page + len).next_multiple_of(page_size);
        let vmo = resource.device_memory(page, size)?;
        let map = Mapping::new(&vmo, 0, size, MAP_READ | MAP_WRITE)?;
        Ok(Self {
            map,
            base: phys - page,
            len,
        })
    }
}

macro_rules! os_io {
    ($($r:ident $w:ident $t:ty),*) => {
        impl Io for OsMmio {
            $(
                fn $r(&self, off: u64) -> $t {
                    if fits(off, core::mem::size_of::<$t>() as u64, self.len) {
                        self.map.read(self.base + off)
                    } else {
                        <$t>::MAX
                    }
                }
                fn $w(&self, off: u64, v: $t) {
                    if fits(off, core::mem::size_of::<$t>() as u64, self.len) {
                        self.map.write(self.base + off, v)
                    }
                }
            )*
            fn len(&self) -> u64 {
                self.len
            }
        }
    };
}

os_io!(read8 write8 u8, read16 write16 u16, read32 write32 u32, read64 write64 u64);

/// DMA pamet z Resource ovladace.
pub struct OsDma {
    _vmo: Vmo,
    map: Mapping,
    iova: u64,
    len: u64,
}

impl DmaMemory for OsDma {
    fn iova(&self) -> u64 {
        self.iova
    }
    fn len(&self) -> u64 {
        self.len
    }
    fn read_bytes(&self, off: u64, buf: &mut [u8]) {
        self.map.read_bytes(off, buf)
    }
    fn write_bytes(&self, off: u64, data: &[u8]) {
        self.map.write_bytes(off, data)
    }
}

impl OsPlatform {
    /// Prevezme startovni zpravu a konfiguraci od spravce zarizeni.
    pub fn start(startup: Startup, name: &'static str) -> Result<Self> {
        let mut handles = startup.handles.into_iter();
        let (Some(res), Some(cfg)) = (handles.next(), handles.next()) else {
            return Err(DriverError::Config(String::from(
                "chybi handly ve startovni zprave (Resource, konfigurace PCI)",
            )));
        };
        let resource = Resource::from_handle(res);
        let config_vmo = Vmo::from_handle(cfg);
        let client = handles.next().map(Channel::from_handle);
        let msg = startup
            .bootstrap
            .read(Some(libos::time::deadline_after(CONFIG_TIMEOUT)))
            .map_err(DriverError::os("konfigurace od spravce zarizeni"))?;
        let text = core::str::from_utf8(&msg.data).unwrap_or("");
        let config = config::parse(text)
            .ok_or_else(|| DriverError::Config(alloc::format!("neplatna zprava '{text}'")))?;
        let address = Address {
            bus: config.bus,
            device: config.device,
            function: config.function,
        };
        let ecam = Ecam::single(&config_vmo, address)
            .map_err(DriverError::os("mapovani konfigurace PCI"))?;
        let info = DeviceInfo {
            bus: address.bus,
            device: address.device,
            function: address.function,
            vendor: ecam.read16(address, pci::VENDOR_ID),
            device_id: ecam.read16(address, pci::VENDOR_ID + 2),
        };
        Ok(Self {
            _process: startup.process,
            _bootstrap: startup.bootstrap,
            name,
            info,
            address,
            resource,
            ecam,
            _config_vmo: config_vmo,
            client,
            config,
            irqs: RefCell::new(Vec::new()),
            msix: RefCell::new(None),
            msi: Cell::new(false),
        })
    }

    /// Konfiguracni prostor funkce (pro knihovny jako `virtio`).
    pub fn ecam(&self) -> &Ecam {
        &self.ecam
    }

    pub fn address(&self) -> Address {
        self.address
    }

    /// Omezeny Resource ovladace.
    pub fn resource(&self) -> &Resource {
        &self.resource
    }

    /// Kanal ke klientovi (pro zpravy s handly).
    pub fn client(&self) -> Option<&Channel> {
        self.client.as_ref()
    }

    /// Prevezme preruseni, ktere ovladac zapnul sam (napr. `virtio::start`);
    /// smycka na nej pak ceka jako na ostatni.
    pub fn adopt_irq(&self, irq: Interrupt) -> IrqId {
        let mut irqs = self.irqs.borrow_mut();
        irqs.push(irq);
        IrqId(irqs.len() as u16 - 1)
    }

    /// Zapne pristup do pameti zarizeni a DMA (bus master), vypne INTx.
    pub fn enable_device(&self) {
        self.ecam.enable(self.address);
    }

    fn missing_bar(&self, index: u8) -> DriverError {
        if self.ecam.memory_bar(self.address, index).is_some_and(|b| b.phys != 0) {
            DriverError::MissingRight {
                what: alloc::format!("mapovat BAR {index} (spravce zarizeni ho nepredal)"),
                hint: "pridejte BAR do resources.bars v manifestu ovladace",
            }
        } else {
            DriverError::Device(alloc::format!("zarizeni nema pametovy BAR {index}"))
        }
    }
}

impl Platform for OsPlatform {
    type Mmio = OsMmio;
    type Dma = OsDma;

    fn info(&self) -> &DeviceInfo {
        &self.info
    }

    fn name(&self) -> &str {
        self.name
    }

    fn map_bar(&self, index: u8) -> Result<OsMmio> {
        let bar = self
            .config
            .bars
            .iter()
            .find(|b| b.index == index)
            .copied()
            .ok_or_else(|| self.missing_bar(index))?;
        self.enable_device();
        OsMmio::map(&self.resource, bar.phys, bar.size).map_err(|e| match e {
            Error::AccessDenied | Error::OutOfRange => DriverError::MissingRight {
                what: alloc::format!("mapovat BAR {index} na {:#x}", bar.phys),
                hint: "Resource ovladace rozsah nepovoluje (manifest resources.bars, devmgr)",
            },
            err => DriverError::Os {
                op: "mapovani BARu",
                err,
            },
        })
    }

    fn alloc_dma(&self, name: &'static str, size: u64) -> Result<DmaBuffer<OsDma>> {
        let quota = |e| match e {
            Error::NoResources | Error::AccessDenied => DriverError::DmaQuota { buffer: name, size },
            err => DriverError::Os {
                op: "DMA pamet",
                err,
            },
        };
        let vmo = self.resource.dma_memory(size).map_err(quota)?;
        let iova = self
            .resource
            .phys_addr(&vmo, 0)
            .map_err(DriverError::os("adresa DMA pameti"))?;
        let map = Mapping::new(&vmo, 0, size, MAP_READ | MAP_WRITE)
            .map_err(DriverError::os("mapovani DMA pameti"))?;
        Ok(DmaBuffer::new(
            OsDma {
                _vmo: vmo,
                map,
                iova,
                len: size,
            },
            name,
        ))
    }

    fn enable_irq(&self) -> Result<IrqId> {
        let n = self.irqs.borrow().len() as u16;
        let irq = self.resource.interrupt().map_err(|e| match e {
            Error::NoResources | Error::AccessDenied => DriverError::IrqQuota {
                index: n,
                hint: "zvyste resources.irq_max v manifestu ovladace",
            },
            err => DriverError::Os {
                op: "preruseni",
                err,
            },
        })?;
        let mut msix = self.msix.borrow_mut();
        if msix.is_none() && !self.msi.get() {
            match MsixTable::open(&self.ecam, self.address, &self.resource) {
                Ok(t) => *msix = Some(t),
                Err(Error::NotSupported) => {
                    pci::msi::enable(&self.ecam, self.address, irq.msi()).map_err(|_| {
                        DriverError::IrqQuota {
                            index: n,
                            hint: "zarizeni nema MSI ani MSI-X (stara linka INTx neni podporovana)",
                        }
                    })?;
                    self.msi.set(true);
                }
                Err(err) => {
                    return Err(DriverError::Os {
                        op: "tabulka MSI-X",
                        err,
                    });
                }
            }
        } else if self.msi.get() {
            return Err(DriverError::IrqQuota {
                index: n,
                hint: "zarizeni ma jen MSI (bez X): jedno preruseni",
            });
        }
        if let Some(t) = msix.as_ref() {
            t.set(n, irq.msi()).map_err(|_| DriverError::IrqQuota {
                index: n,
                hint: "tabulka MSI-X zarizeni nema tolik polozek",
            })?;
            if n == 0 {
                t.enable(&self.ecam);
            }
        }
        drop(msix);
        Ok(self.adopt_irq(irq))
    }

    fn now(&self) -> u64 {
        libos::time::now()
    }

    fn relax(&self) {
        core::hint::spin_loop();
    }

    fn wait(&self, deadline: Option<u64>) -> Result<Event> {
        let irqs = self.irqs.borrow();
        let mut items: Vec<(&libos::Handle, Signals)> =
            irqs.iter().map(|i| (i.handle(), Signals::SIGNALED)).collect();
        if let Some(c) = &self.client {
            items.push((c.handle(), Signals::READABLE | Signals::PEER_CLOSED));
        }
        if items.is_empty() {
            let Some(d) = deadline else {
                return Err(DriverError::Config(String::from(
                    "ovladac nema preruseni, klienta ani casovac: nemel by na co cekat",
                )));
            };
            // Jen casovac: spat do terminu.
            libos::time::sleep(core::time::Duration::from_nanos(d.saturating_sub(libos::time::now())));
            return Ok(Event::Timer);
        }
        let observed = match libos::wait_many(&items, deadline) {
            Ok((_, o)) => o,
            Err(Error::TimedOut) => return Ok(Event::Timer),
            Err(err) => return Err(DriverError::Os { op: "cekani na udalost", err }),
        };
        drop(items);
        for (i, s) in observed.iter().take(irqs.len()).enumerate() {
            if s.intersects(Signals::SIGNALED) {
                // Potvrdit hned: preruseni behem obsluhy se neztrati.
                irqs[i].ack().map_err(DriverError::os("potvrzeni preruseni"))?;
                return Ok(Event::Interrupt(IrqId(i as u16)));
            }
        }
        if let (Some(c), Some(s)) = (&self.client, observed.get(irqs.len())) {
            if s.intersects(Signals::READABLE) {
                return match c.try_read() {
                    Ok(m) => Ok(Event::Message(m.data)),
                    Err(Error::ShouldWait) => self.wait_again(deadline, irqs),
                    Err(Error::PeerClosed) => Ok(Event::ClientClosed),
                    Err(err) => Err(DriverError::Os { op: "zprava od klienta", err }),
                };
            }
            if s.intersects(Signals::PEER_CLOSED) {
                return Ok(Event::ClientClosed);
            }
        }
        Ok(Event::Timer)
    }

    fn send(&self, data: &[u8]) -> Result<Sent> {
        let c = self
            .client
            .as_ref()
            .ok_or_else(|| DriverError::Config(String::from("ovladac nema kanal ke klientovi")))?;
        match c.write(data, vec![]) {
            Ok(()) => Ok(Sent::Sent),
            Err(Error::ShouldWait) => Ok(Sent::Full),
            Err(Error::PeerClosed) => Err(DriverError::ClientClosed),
            Err(err) => Err(DriverError::Os { op: "zprava klientovi", err }),
        }
    }

    fn log(&self, args: fmt::Arguments) {
        println!("{} {}: {}", self.name, self.info, args);
    }

    fn dma_wmb(&self) {
        libos::barrier::dma_wmb()
    }

    fn dma_rmb(&self) {
        libos::barrier::dma_rmb()
    }

    fn mmio_wmb(&self) {
        libos::barrier::mmio_wmb()
    }
}

impl OsPlatform {
    fn wait_again(
        &self,
        deadline: Option<u64>,
        irqs: core::cell::Ref<'_, Vec<Interrupt>>,
    ) -> Result<Event> {
        drop(irqs);
        self.wait(deadline)
    }
}

/// Cely program ovladace: start, smycka, navratovy kod.
pub fn main<D: Driver<OsPlatform>>(startup: Startup, name: &'static str) -> i64 {
    let p = match OsPlatform::start(startup, name) {
        Ok(p) => p,
        Err(e) => {
            println!("{name}: {e}");
            return e.exit_code().max(1);
        }
    };
    crate::run::<OsPlatform, D>(&p).code()
}
