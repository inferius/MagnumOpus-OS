//! Simulace hardwaru pro testy ovladacu na hostiteli (feature `sim`).
//!
//! Ovladac napsany proti [`Platform`] se v testu spusti nad
//! [`SimPlatform`] misto jadra. Zarizeni predstavuje model
//! ([`DeviceModel`]) s registry, prerusenimi a pristupem do DMA pameti.
//! Simulace navic hlida, co by na skutecnem stroji hlidala IOMMU a jadro:
//!
//! - DMA zarizeni jen do pameti, kterou ovladac alokoval (jinak
//!   [`DriverError::DmaFault`] v [`Sim::faults`]),
//! - jen BARy z manifestu ([`Sim::bars`]), kvoty preruseni a DMA,
//! - cas je virtualni: test je deterministicky a rychly; cekani bez
//!   udalosti skonci po [`Sim::time_limit`] chybou misto zaseknuti.
//!
//! ```ignore
//! let sim = Sim::new("edu", info, EduModel::new()).bars(&[0]).irq_max(1);
//! sim.send(b"fact 5");
//! sim.close();
//! let exit = driverkit::run::<_, Edu<_>>(&sim.platform());
//! assert_eq!(sim.sent_text(), ["120"]);
//! assert!(sim.faults().is_empty());
//! ```
//!
//! Model pro hardware, ktery QEMU neemuluje (Intel grafika, Realtek, HDA
//! kodeky), staci napsat podle datasheetu jen v rozsahu, ktery ovladac
//! pouziva; [`RegFile`] resi beznou mechaniku registru (RO, W1C, reset).

mod model;
mod regfile;

pub use model::{Bus, DeviceModel};
pub use regfile::RegFile;

use alloc::boxed::Box;
use alloc::collections::VecDeque;
use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;
use core::cell::RefCell;
use core::fmt;

use crate::dma::{DmaBuffer, DmaMemory};
use crate::io::Io;
use crate::platform::{DeviceInfo, Event, IrqId, Platform, Sent};
use crate::{DriverError, Result};

use model::{DmaSpace, Region};

/// Jeden pristup ovladace do registru (zaznam pro testy a ladeni).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Access {
    pub at: u64,
    pub bar: u8,
    pub off: u64,
    pub size: u8,
    pub value: u64,
    pub write: bool,
}

/// Prvni IOVA, kterou simulace pridava (nenulova, at chyba "adresa 0" je
/// videt).
const IOVA_BASE: u64 = 0x1_0000_0000;
const PAGE: u64 = 4096;

pub(crate) struct State {
    pub now: u64,
    pub dma: DmaSpace,
    pub irqs_enabled: u16,
    pub pending: VecDeque<u16>,
    inbox: VecDeque<Vec<u8>>,
    client_closed: bool,
    client_full: bool,
    outbox: Vec<Vec<u8>>,
    pub logs: Vec<String>,
    faults: Vec<DriverError>,
    trace: Vec<Access>,
    irq_max: u16,
    dma_max: u64,
    bars: Option<Vec<u8>>,
    limit_ns: u64,
    step_ns: u64,
}

impl State {
    pub(crate) fn fault(&mut self, iova: u64, len: u64, write: bool) -> DriverError {
        let e = DriverError::DmaFault { iova, len, write };
        self.logs.push(alloc::format!("sim: {e}"));
        self.faults.push(e.clone());
        e
    }
}

struct Inner {
    name: String,
    info: DeviceInfo,
    model: RefCell<Box<dyn DeviceModel>>,
    state: RefCell<State>,
}

impl Inner {
    fn advance(&self, ns: u64) {
        let mut m = self.model.borrow_mut();
        let mut s = self.state.borrow_mut();
        s.now = s.now.saturating_add(ns);
        m.tick(&mut Bus { state: &mut s });
    }
}

/// Simulovane zarizeni a jeho okoli (klient, logy). Test drzi `Sim`
/// a ovladac dostane [`Sim::platform`].
#[derive(Clone)]
pub struct Sim {
    inner: Rc<Inner>,
}

impl Sim {
    /// Ovladac `name` pro zarizeni `info` s modelem `model`. Vychozi
    /// kvoty: 1 preruseni, 1 MiB DMA, vsechny BARy, limit 10 s virtualniho
    /// casu bez udalosti.
    pub fn new(name: &str, info: DeviceInfo, model: impl DeviceModel) -> Self {
        Self {
            inner: Rc::new(Inner {
                name: String::from(name),
                info,
                model: RefCell::new(Box::new(model)),
                state: RefCell::new(State {
                    now: 0,
                    dma: DmaSpace {
                        next_iova: IOVA_BASE,
                        ..Default::default()
                    },
                    irqs_enabled: 0,
                    pending: VecDeque::new(),
                    inbox: VecDeque::new(),
                    client_closed: false,
                    client_full: false,
                    outbox: Vec::new(),
                    logs: Vec::new(),
                    faults: Vec::new(),
                    trace: Vec::new(),
                    irq_max: 1,
                    dma_max: 1 << 20,
                    bars: None,
                    limit_ns: 10_000_000_000,
                    step_ns: 100_000,
                }),
            }),
        }
    }

    /// Kvoty jako v manifestu (`resources.*`).
    pub fn irq_max(self, n: u16) -> Self {
        self.inner.state.borrow_mut().irq_max = n;
        self
    }

    pub fn dma_max(self, bytes: u64) -> Self {
        self.inner.state.borrow_mut().dma_max = bytes;
        self
    }

    /// BARy, ktere ovladac smi mapovat (`resources.bars`).
    pub fn bars(self, bars: &[u8]) -> Self {
        self.inner.state.borrow_mut().bars = Some(bars.to_vec());
        self
    }

    /// Nejdelsi cekani bez udalosti (virtualni ns).
    pub fn time_limit(self, ns: u64) -> Self {
        self.inner.state.borrow_mut().limit_ns = ns;
        self
    }

    /// Kvoty podle manifestu ovladace.
    pub fn with_manifest(self, m: &crate::manifest::Manifest) -> Self {
        self.irq_max(m.irq_max as u16).dma_max(m.dma_max).bars(&m.bars)
    }

    /// Rozhrani pro ovladac.
    pub fn platform(&self) -> SimPlatform {
        SimPlatform {
            inner: self.inner.clone(),
        }
    }

    /// Klient posle ovladaci zpravu.
    pub fn send(&self, msg: &[u8]) {
        self.inner.state.borrow_mut().inbox.push_back(msg.to_vec());
    }

    /// Klient zavre kanal (po zpracovani zprav ve fronte ovladac skonci).
    pub fn close(&self) {
        self.inner.state.borrow_mut().client_closed = true;
    }

    /// Fronta klienta plna (`send` ovladace vrati [`Sent::Full`]).
    pub fn set_client_full(&self, full: bool) {
        self.inner.state.borrow_mut().client_full = full;
    }

    /// Zpravy od ovladace klientovi (a vyprazdni frontu).
    pub fn sent(&self) -> Vec<Vec<u8>> {
        core::mem::take(&mut self.inner.state.borrow_mut().outbox)
    }

    /// Zpravy od ovladace jako text.
    pub fn sent_text(&self) -> Vec<String> {
        self.sent()
            .into_iter()
            .map(|m| String::from_utf8_lossy(&m).into_owned())
            .collect()
    }

    /// Logy ovladace a simulace.
    pub fn logs(&self) -> Vec<String> {
        self.inner.state.borrow().logs.clone()
    }

    /// Pristupy DMA mimo pridelenou pamet (na stroji by je zablokovala
    /// IOMMU). Test ovladace by mel koncit s prazdnym seznamem.
    pub fn faults(&self) -> Vec<DriverError> {
        self.inner.state.borrow().faults.clone()
    }

    /// Pristupy ovladace do registru od zacatku (nebo od `clear_trace`).
    pub fn trace(&self) -> Vec<Access> {
        self.inner.state.borrow().trace.clone()
    }

    pub fn clear_trace(&self) {
        self.inner.state.borrow_mut().trace.clear();
    }

    /// Virtualni cas v ns.
    pub fn now(&self) -> u64 {
        self.inner.state.borrow().now
    }

    /// Posune cas (model dostane `tick`).
    pub fn advance(&self, ns: u64) {
        self.inner.advance(ns)
    }

    /// DMA alokace ovladace: (jmeno, IOVA, velikost).
    pub fn dma_regions(&self) -> Vec<(&'static str, u64, u64)> {
        let s = self.inner.state.borrow();
        s.dma
            .regions
            .iter()
            .map(|r| (r.name, r.iova, r.data.len() as u64))
            .collect()
    }

    /// Pristup k modelu (stav, vlozeni udalosti: prichozi paket, stisk
    /// klavesy, odpojeni kabelu). `M` musi byt typ modelu z [`Sim::new`].
    pub fn with_model<M: DeviceModel, R>(&self, f: impl FnOnce(&mut M, &mut Bus) -> R) -> R {
        let mut m = self.inner.model.borrow_mut();
        let mut s = self.inner.state.borrow_mut();
        let model = m
            .as_any()
            .downcast_mut::<M>()
            .expect("with_model: jiny typ modelu nez v Sim::new");
        f(model, &mut Bus { state: &mut s })
    }
}

/// [`Platform`] nad simulaci.
#[derive(Clone)]
pub struct SimPlatform {
    inner: Rc<Inner>,
}

/// Okno registru simulovaneho BARu.
pub struct SimMmio {
    inner: Rc<Inner>,
    bar: u8,
    len: u64,
}

impl SimMmio {
    fn read(&self, off: u64, size: u8) -> u64 {
        if !crate::io::fits(off, size as u64, self.len) {
            let mut s = self.inner.state.borrow_mut();
            s.logs.push(alloc::format!(
                "sim: cteni mimo BAR {} ({off:#x}, {size} B; BAR ma {:#x} B)",
                self.bar, self.len
            ));
            return u64::MAX >> (64 - 8 * size as u32);
        }
        let mut m = self.inner.model.borrow_mut();
        let mut s = self.inner.state.borrow_mut();
        let value = m.read(&mut Bus { state: &mut s }, self.bar, off, size);
        let at = s.now;
        s.trace.push(Access { at, bar: self.bar, off, size, value, write: false });
        value
    }

    fn write(&self, off: u64, size: u8, value: u64) {
        if !crate::io::fits(off, size as u64, self.len) {
            let mut s = self.inner.state.borrow_mut();
            s.logs.push(alloc::format!(
                "sim: zapis mimo BAR {} ({off:#x}, {size} B) se zahodil",
                self.bar
            ));
            return;
        }
        let mut m = self.inner.model.borrow_mut();
        let mut s = self.inner.state.borrow_mut();
        let at = s.now;
        s.trace.push(Access { at, bar: self.bar, off, size, value, write: true });
        m.write(&mut Bus { state: &mut s }, self.bar, off, size, value);
    }
}

impl Io for SimMmio {
    fn read8(&self, off: u64) -> u8 {
        self.read(off, 1) as u8
    }
    fn read16(&self, off: u64) -> u16 {
        self.read(off, 2) as u16
    }
    fn read32(&self, off: u64) -> u32 {
        self.read(off, 4) as u32
    }
    fn read64(&self, off: u64) -> u64 {
        self.read(off, 8)
    }
    fn write8(&self, off: u64, v: u8) {
        self.write(off, 1, v as u64)
    }
    fn write16(&self, off: u64, v: u16) {
        self.write(off, 2, v as u64)
    }
    fn write32(&self, off: u64, v: u32) {
        self.write(off, 4, v as u64)
    }
    fn write64(&self, off: u64, v: u64) {
        self.write(off, 8, v)
    }
    fn len(&self) -> u64 {
        self.len
    }
}

/// Simulovana DMA pamet (oblast v [`Sim`]).
pub struct SimDma {
    inner: Rc<Inner>,
    index: usize,
    iova: u64,
    len: u64,
}

impl DmaMemory for SimDma {
    fn iova(&self) -> u64 {
        self.iova
    }
    fn len(&self) -> u64 {
        self.len
    }
    fn read_bytes(&self, off: u64, buf: &mut [u8]) {
        let s = self.inner.state.borrow();
        let r = &s.dma.regions[self.index];
        buf.copy_from_slice(&r.data[off as usize..off as usize + buf.len()]);
    }
    fn write_bytes(&self, off: u64, data: &[u8]) {
        let mut s = self.inner.state.borrow_mut();
        let r = &mut s.dma.regions[self.index];
        r.data[off as usize..off as usize + data.len()].copy_from_slice(data);
    }
}

impl Platform for SimPlatform {
    type Mmio = SimMmio;
    type Dma = SimDma;

    fn info(&self) -> &DeviceInfo {
        &self.inner.info
    }

    fn name(&self) -> &str {
        &self.inner.name
    }

    fn map_bar(&self, index: u8) -> Result<SimMmio> {
        if let Some(bars) = &self.inner.state.borrow().bars
            && !bars.contains(&index)
        {
            return Err(DriverError::MissingRight {
                what: alloc::format!("mapovat BAR {index}"),
                hint: "pridejte BAR do resources.bars v manifestu ovladace",
            });
        }
        let len = self
            .inner
            .model
            .borrow()
            .bar_size(index)
            .ok_or_else(|| DriverError::Device(alloc::format!("zarizeni nema BAR {index}")))?;
        Ok(SimMmio {
            inner: self.inner.clone(),
            bar: index,
            len,
        })
    }

    fn alloc_dma(&self, name: &'static str, size: u64) -> Result<DmaBuffer<SimDma>> {
        let mut s = self.inner.state.borrow_mut();
        let size = size.max(1);
        if s.dma.used.saturating_add(size) > s.dma_max {
            return Err(DriverError::DmaQuota { buffer: name, size });
        }
        let iova = s.dma.next_iova;
        // Mezera jedne stranky za kazdou oblasti: preteceni za konec bufferu
        // nenarazi do dalsiho (jako ochranna stranka).
        s.dma.next_iova += size.next_multiple_of(PAGE) + PAGE;
        s.dma.used += size;
        let index = s.dma.regions.len();
        s.dma.regions.push(Region {
            iova,
            data: alloc::vec![0; size as usize],
            name,
        });
        Ok(DmaBuffer::new(
            SimDma {
                inner: self.inner.clone(),
                index,
                iova,
                len: size,
            },
            name,
        ))
    }

    fn enable_irq(&self) -> Result<IrqId> {
        let mut s = self.inner.state.borrow_mut();
        if s.irqs_enabled >= s.irq_max {
            return Err(DriverError::IrqQuota {
                index: s.irqs_enabled,
                hint: "zvyste resources.irq_max v manifestu ovladace",
            });
        }
        s.irqs_enabled += 1;
        Ok(IrqId(s.irqs_enabled - 1))
    }

    fn now(&self) -> u64 {
        self.inner.state.borrow().now
    }

    fn relax(&self) {
        self.inner.advance(1_000);
    }

    fn wait(&self, deadline: Option<u64>) -> Result<Event> {
        let start = self.now();
        loop {
            {
                let mut s = self.inner.state.borrow_mut();
                if let Some(v) = s.pending.pop_front() {
                    return Ok(Event::Interrupt(IrqId(v)));
                }
                if let Some(m) = s.inbox.pop_front() {
                    return Ok(Event::Message(m));
                }
                if s.client_closed {
                    return Ok(Event::ClientClosed);
                }
                if deadline.is_some_and(|d| s.now >= d) {
                    return Ok(Event::Timer);
                }
                if s.now - start >= s.limit_ns {
                    let ms = s.limit_ns / 1_000_000;
                    s.logs.push(alloc::format!(
                        "sim: ovladac ceka {ms} ms a nic se nedeje (zarizeni nevyvolalo preruseni, klient nic neposlal)"
                    ));
                    return Err(DriverError::Timeout {
                        what: "simulace: cekani na udalost",
                        ms,
                    });
                }
            }
            let step = {
                let s = self.inner.state.borrow();
                match deadline {
                    Some(d) => s.step_ns.min(d.saturating_sub(s.now)).max(1),
                    None => s.step_ns,
                }
            };
            self.inner.advance(step);
        }
    }

    fn send(&self, data: &[u8]) -> Result<Sent> {
        let mut s = self.inner.state.borrow_mut();
        if s.client_full {
            return Ok(Sent::Full);
        }
        s.outbox.push(data.to_vec());
        Ok(Sent::Sent)
    }

    fn log(&self, args: fmt::Arguments) {
        let line = alloc::format!("{} {}: {}", self.inner.name, self.inner.info, args);
        self.inner.state.borrow_mut().logs.push(line);
    }

    fn dma_wmb(&self) {
        core::sync::atomic::fence(core::sync::atomic::Ordering::SeqCst);
    }

    fn dma_rmb(&self) {
        core::sync::atomic::fence(core::sync::atomic::Ordering::SeqCst);
    }

    fn mmio_wmb(&self) {
        core::sync::atomic::fence(core::sync::atomic::Ordering::SeqCst);
    }
}
