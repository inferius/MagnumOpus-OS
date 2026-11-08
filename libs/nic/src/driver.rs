//! Beh ovladace v systemu: startovni zprava, registry a DMA pres libos,
//! preruseni a smycka mezi kartou a sluzbou `netd` (`netproto::link`).
//!
//! Startovni zprava od spravce zarizeni (kind USER, v tomto poradi):
//! 1. omezeny Resource (BARy, konfigurace, kvoty preruseni a DMA; s IOMMU
//!    vlastni domena DMA),
//! 2. stranka konfiguracniho prostoru PCI teto funkce,
//! 3. kanal ke sluzbe site.
//!
//! Zpravy na bootstrap kanalu: `pci <sbernice> <zarizeni> <funkce>`
//! a `regs <fyzicka adresa> <velikost>` (okno registru = prvni pametovy
//! BAR, velikost zjisti spravce, ovladac BARy neprepisuje).
//!
//! Smycka: preruseni karty, zpravy od netd a kazdou sekundu kontrola
//! linky (kdyby se preruseni o zmene ztratilo). Zmena linky jde do netd
//! zpravou `LINK`.

use alloc::boxed::Box;
use alloc::vec::Vec;
use core::time::Duration;

use libos::abi::vm::{MAP_READ, MAP_WRITE};
use libos::hw::Resource;
use libos::{Channel, Error, Mapping, Signals, Startup, Vmo, println, wait_many};
use netproto::link;
use pci::irq::{DeviceIrq, Kind, Prefer};
use pci::{Address, Ecam};

use crate::{HwError, Layout, LinkState, MAX_FRAME, Nic};

/// Registry a DMA pamet jedne karty.
pub struct Mmio {
    regs: Mapping,
    /// Offset registru v mapovani (BAR mensi nez stranka nemusi zacinat
    /// na jejim zacatku).
    regs_off: u64,
    dma: Mapping,
    dma_phys: u64,
    _dma_vmo: Vmo,
}

impl crate::Hw for Mmio {
    fn read8(&self, reg: u32) -> u8 {
        self.regs.read(self.regs_off + reg as u64)
    }
    fn read16(&self, reg: u32) -> u16 {
        self.regs.read(self.regs_off + reg as u64)
    }
    fn read32(&self, reg: u32) -> u32 {
        self.regs.read(self.regs_off + reg as u64)
    }
    fn write8(&self, reg: u32, v: u8) {
        self.regs.write(self.regs_off + reg as u64, v)
    }
    fn write16(&self, reg: u32, v: u16) {
        self.regs.write(self.regs_off + reg as u64, v)
    }
    fn write32(&self, reg: u32, v: u32) {
        self.regs.write(self.regs_off + reg as u64, v)
    }
    fn dma_addr(&self) -> u64 {
        self.dma_phys
    }
    fn dma_read(&self, off: usize, buf: &mut [u8]) {
        self.dma.read_bytes(off as u64, buf)
    }
    fn dma_write(&self, off: usize, data: &[u8]) {
        self.dma.write_bytes(off as u64, data)
    }
    fn dma_read32(&self, off: usize) -> u32 {
        self.dma.read(off as u64)
    }
    fn dma_write32(&self, off: usize, v: u32) {
        self.dma.write(off as u64, v)
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
    fn sleep_us(&self, us: u64) {
        libos::time::sleep(Duration::from_micros(us))
    }
}

/// Zarizeni predane ovladaci: ID PCI a adresa.
#[derive(Debug, Clone, Copy)]
pub struct Device {
    pub address: Address,
    pub vendor: u16,
    pub device: u16,
}

/// Otevre kartu nad pripravenymi registry a DMA.
pub type Open = fn(Mmio, &Device, u64) -> Result<Box<dyn Nic>, HwError>;

/// Jak dlouho cekat na zpravy od spravce zarizeni.
const CONFIG_TIMEOUT: Duration = Duration::from_secs(5);
/// Kontrola linky bez preruseni.
const LINK_POLL: Duration = Duration::from_secs(1);

/// Rozebere `pci b d f` a `regs phys size`.
pub fn parse_config(pci: &str, regs: &str) -> Option<(Address, u64, u64)> {
    let mut p = pci.split_whitespace();
    if p.next()? != "pci" {
        return None;
    }
    let mut n = || p.next()?.parse::<u8>().ok();
    let a = Address {
        bus: n()?,
        device: n()?,
        function: n()?,
    };
    if p.next().is_some() || a.device > 31 || a.function > 7 {
        return None;
    }
    let mut r = regs.split_whitespace();
    if r.next()? != "regs" {
        return None;
    }
    let phys = r.next()?.parse::<u64>().ok()?;
    let size = r.next()?.parse::<u64>().ok()?;
    if r.next().is_some() || size == 0 {
        return None;
    }
    Some((a, phys, size))
}

fn read_config(bootstrap: &Channel) -> Option<(Address, u64, u64)> {
    let deadline = || Some(libos::time::deadline_after(CONFIG_TIMEOUT));
    let pci = bootstrap.read(deadline()).ok()?;
    let regs = bootstrap.read(deadline()).ok()?;
    parse_config(core::str::from_utf8(&pci.data).ok()?, core::str::from_utf8(&regs.data).ok()?)
}

fn link_info(s: LinkState) -> (u8, u32) {
    let mut flags = 0;
    if s.up {
        flags |= link::FLAG_UP;
        if s.full_duplex {
            flags |= link::FLAG_FULL_DUPLEX;
        }
    }
    (flags, s.speed_mbps)
}

/// Cely beh ovladace `name`; vraci navratovy kod procesu.
pub fn run(name: &str, startup: Startup, open: Open) -> i64 {
    let mut handles = startup.handles.into_iter();
    let (Some(res), Some(ecam_vmo), Some(client)) = (handles.next(), handles.next(), handles.next()) else {
        println!("{name}: chybi handly ve startovni zprave");
        return 1;
    };
    let resource = Resource::from_handle(res);
    let ecam_vmo = Vmo::from_handle(ecam_vmo);
    let client = Channel::from_handle(client);
    let Some((address, regs_phys, regs_size)) = read_config(&startup.bootstrap) else {
        println!("{name}: neplatna konfigurace od spravce zarizeni");
        return 2;
    };
    let ecam = match Ecam::single(&ecam_vmo, address) {
        Ok(e) => e,
        Err(e) => {
            println!("{name} {address}: konfigurace PCI: {e}");
            return 2;
        }
    };
    let dev = Device {
        address,
        vendor: ecam.read16(address, pci::VENDOR_ID),
        device: ecam.read16(address, pci::VENDOR_ID + 2),
    };
    // Pamet a DMA zapnout, INTx vypnout (preruseni jdou pres MSI).
    ecam.enable(address);
    let mmio = match map(&resource, regs_phys, regs_size) {
        Ok(m) => m,
        Err(e) => {
            println!("{name} {address}: registry a DMA nelze namapovat: {e}");
            return 3;
        }
    };
    let seed = libos::time::now() ^ (u64::from(address.bus) << 16 | u64::from(address.device) << 8);
    let mut nic = match open(mmio, &dev, seed) {
        Ok(n) => n,
        Err(e) => {
            println!("{name} {address}: inicializace selhala: {e}");
            return 3;
        }
    };
    for n in nic.notes() {
        println!("{name} {address}: {n}");
    }
    // Preruseni az po resetu karty (reset by MSI-X tabulku nesmazal, ale
    // karta pred resetem muze posilat preruseni ze stareho nastaveni).
    let irq = match DeviceIrq::attach(&ecam, address, &resource, Prefer::Msi) {
        Ok(i) => i,
        Err(e) => {
            nic.stop();
            println!("{name} {address}: preruseni (MSI/MSI-X) nelze nastavit: {e}");
            return 3;
        }
    };
    let result = serve(name, &dev, &mut *nic, &irq, &client);
    // Kartu zastavit v kazdem pripade: pamet DMA se s procesem uvolni.
    nic.stop();
    if let Kind::Msi(m) = &irq.kind {
        m.disable(&ecam);
    }
    let s = nic.stats();
    if s.rx_errors > 0 {
        println!("{name} {address}: zahozeno {} vadnych prijatych ramcu", s.rx_errors);
    }
    match result {
        Ok(()) => 0,
        Err(e) => {
            println!("{name} {address}: {e}, koncim");
            4
        }
    }
}

fn map(resource: &Resource, regs_phys: u64, regs_size: u64) -> libos::Result<Mmio> {
    let page = libos::vm::page_size();
    let start = regs_phys & !(page - 1);
    let regs_off = regs_phys - start;
    let size = (regs_off + regs_size).next_multiple_of(page);
    let regs_vmo = resource.device_memory(start, size)?;
    let regs = Mapping::new(&regs_vmo, 0, size, MAP_READ | MAP_WRITE)?;
    let dma_size = (Layout::SIZE as u64).next_multiple_of(page);
    let dma_vmo = resource.dma_memory(dma_size)?;
    let dma_phys = resource.phys_addr(&dma_vmo, 0)?;
    let dma = Mapping::new(&dma_vmo, 0, dma_size, MAP_READ | MAP_WRITE)?;
    Ok(Mmio {
        regs,
        regs_off,
        dma,
        dma_phys,
        _dma_vmo: dma_vmo,
    })
}

/// Chyba smycky: karta, nebo jadro (preruseni, kanal).
enum Failure {
    Hw(HwError),
    Os(Error),
}

impl core::fmt::Display for Failure {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Failure::Hw(e) => write!(f, "karta: {e}"),
            Failure::Os(e) => write!(f, "system: {e}"),
        }
    }
}

impl From<HwError> for Failure {
    fn from(e: HwError) -> Self {
        Failure::Hw(e)
    }
}

impl From<Error> for Failure {
    fn from(e: Error) -> Self {
        Failure::Os(e)
    }
}

fn serve(name: &str, dev: &Device, nic: &mut dyn Nic, irq: &DeviceIrq, client: &Channel) -> Result<(), Failure> {
    let m = nic.mac();
    let mut state = nic.link()?;
    println!(
        "{name} {}: {} {:04x}:{:04x}, MAC {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}, {}, linka {}",
        dev.address,
        nic.model(),
        dev.vendor,
        dev.device,
        m[0],
        m[1],
        m[2],
        m[3],
        m[4],
        m[5],
        irq.name(),
        describe(state),
    );
    let (flags, speed_mbps) = link_info(state);
    let info = link::Info {
        mac: m,
        mtu: 1500,
        flags,
        speed_mbps,
    };
    if client.write(&info.encode(), Vec::new()).is_err() {
        return Ok(());
    }
    let mut frame = [0u8; 1 + link::MAX_FRAME];
    frame[0] = link::FRAME;
    // Priciny z doby pred nastavenim MSI smazat: cekajici pricina by
    // u nekterych karet novou zpravu MSI neposlala.
    nic.interrupt()?;
    loop {
        let client_mask = if nic.tx_space() == 0 {
            Signals::PEER_CLOSED
        } else {
            Signals::READABLE | Signals::PEER_CLOSED
        };
        let deadline = libos::time::deadline_after(LINK_POLL);
        let observed = match wait_many(
            &[(irq.irq.handle(), Signals::SIGNALED), (client.handle(), client_mask)],
            Some(deadline),
        ) {
            Ok((_, o)) => Some(o),
            Err(Error::TimedOut) => None,
            Err(e) => return Err(e.into()),
        };
        if observed.as_ref().is_some_and(|o| o[1].contains(Signals::PEER_CLOSED)) {
            return Ok(());
        }
        let mut link_event = observed.is_none();
        if observed.as_ref().is_some_and(|o| o[0].contains(Signals::SIGNALED)) {
            // Potvrdit pred obsluhou: preruseni behem ni znovu probudi.
            let _ = irq.irq.ack();
            let ev = nic.interrupt()?;
            if ev.fatal {
                return Err(HwError::Corrupted("chyba sbernice (system error)").into());
            }
            link_event |= ev.link;
        }
        if link_event {
            let now = nic.link()?;
            if now != state {
                println!("{name} {}: linka {}", dev.address, describe(now));
                state = now;
                let (flags, speed_mbps) = link_info(now);
                let msg = link::Status { flags, speed_mbps }.encode();
                if let Err(Error::PeerClosed) = client.write(&msg, Vec::new()) {
                    return Ok(());
                }
            }
        }
        nic.reclaim()?;
        let mut closed = false;
        nic.receive(&mut |f| {
            let n = f.len();
            frame[1..1 + n].copy_from_slice(f);
            // Plna fronta netd: ramec zahodit (jako na siti).
            if let Err(Error::PeerClosed) = client.write(&frame[..1 + n], Vec::new()) {
                closed = true;
            }
        })?;
        if closed {
            return Ok(());
        }
        let mut sent = false;
        while nic.tx_space() > 0 {
            let msg = match client.try_read() {
                Ok(m) => m,
                Err(Error::ShouldWait) => break,
                Err(_) => return Ok(()),
            };
            let Some((&kind, f)) = msg.data.split_first() else {
                continue;
            };
            if kind != link::FRAME || f.len() < 14 || f.len() > MAX_FRAME {
                continue;
            }
            nic.send(f);
            sent = true;
        }
        if sent {
            nic.kick();
        }
    }
}

fn describe(s: LinkState) -> alloc::string::String {
    if !s.up {
        return "odpojena".into();
    }
    alloc::format!(
        "pripojena {} Mb/s {}",
        s.speed_mbps,
        if s.full_duplex { "plny duplex" } else { "polovicni duplex" }
    )
}
