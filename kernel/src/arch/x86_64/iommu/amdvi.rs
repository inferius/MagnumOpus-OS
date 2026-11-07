//! IOMMU: backend AMD-Vi (spolecna pravidla viz `iommu/mod.rs`).
//!
//! Struktury (specifikace AMD I/O Virtualization Technology):
//! - **tabulka zarizeni:** 65 536 zaznamu po 32 B (2 MiB), index = adresa
//!   zarizeni na sbernici (sbernice << 8 | zarizeni << 3 | funkce). Vychozi
//!   zaznam "platny, preklad zapnuty, rezim 0, bez prav cteni a zapisu"
//!   kazdy pristup zarizeni zablokuje. Zarizeni s domenou ma rezim 4
//!   (4 urovne tabulek, 48 bitu) a ukazatel na koren tabulek domeny.
//! - **strankove tabulky:** zaznam = pritomno (bit 0), uroven dalsi tabulky
//!   (bity 11:9, 0 = stranka), adresa, cteni (bit 61), zapis (bit 62).
//! - **fronta prikazu:** zneplatneni zaznamu zarizeni a stranek domeny,
//!   cekani na dokonceni (zapis hodnoty do pameti, ktery jadro sleduje).
//! - **zaznam udalosti:** zablokovane pristupy; IOMMU hlasi preruseni MSI
//!   (nastavuje se v jejim konfiguracnim prostoru PCI).
//!
//! TODO: preklad preruseni, IVMD (oblasti pameti pro firmware), vyber
//! zarizeni podle bloku IVHD (zatim jednotka pokryva cely svuj segment).

use alloc::collections::BTreeMap;
use alloc::sync::{Arc, Weak};
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicU16, Ordering};

use super::super::acpi::AmdIommuUnit;
use super::VECTOR_IOMMU_FAULT;
use crate::acpi::PciEcam;
use crate::error::Error;
use crate::log;
use crate::mm::{self, PhysAddr};
use crate::sync::SpinLock;

// Registry (offsety).
const DEV_TABLE: u64 = 0x0000;
const CMD_BASE: u64 = 0x0008;
const EVT_BASE: u64 = 0x0010;
const CONTROL: u64 = 0x0018;
const CMD_HEAD: u64 = 0x2000;
const CMD_TAIL: u64 = 0x2008;
const EVT_HEAD: u64 = 0x2010;
const EVT_TAIL: u64 = 0x2018;
const STATUS: u64 = 0x2020;

const CTL_IOMMU_EN: u64 = 1 << 0;
const CTL_EVENT_LOG_EN: u64 = 1 << 2;
const CTL_EVENT_INT_EN: u64 = 1 << 3;
const CTL_CMD_BUF_EN: u64 = 1 << 12;

/// Velikost fronty prikazu a zaznamu udalosti: 256 zaznamu po 16 B.
const RING: u64 = 4096;
const RING_LEN_LOG2: u64 = 8;

/// Zaznam tabulky zarizeni: platny | preklad platny.
const DTE_V_TV: u64 = 0b11;
const DTE_IR: u64 = 1 << 61;
const DTE_IW: u64 = 1 << 62;
/// Pocet urovni tabulek domeny (rezim 4 = 48 bitu).
const LEVELS: u64 = 4;
const PTE_PR: u64 = 1;
const PTE_RW: u64 = 1 << 61 | 1 << 62;
const ADDR_MASK: u64 = 0x000F_FFFF_FFFF_F000;

struct Unit {
    regs: u64,
    segment: u16,
    dev_table: PhysAddr,
    cmd: PhysAddr,
    evt: PhysAddr,
    /// Pamet, do ktere IOMMU zapise hodnotu po dokonceni prikazu.
    sem: PhysAddr,
    /// Zamek fronty prikazu a poradove cislo posledniho cekani.
    cmd_lock: SpinLock<u64>,
}

static UNITS: SpinLock<Vec<&'static Unit>> = SpinLock::new(Vec::new());
static ACTIVE: AtomicBool = AtomicBool::new(false);
static NEXT_DOMAIN: AtomicU16 = AtomicU16::new(1);
static DOMAINS: SpinLock<BTreeMap<(u16, u8, u8), Weak<Domain>>> = SpinLock::new(BTreeMap::new());

fn r64(u: &Unit, off: u64) -> u64 {
    // SAFETY: registry jednotky jsou namapovane (map_mmio) po celou dobu behu.
    unsafe { ((u.regs + off) as *const u64).read_volatile() }
}

fn w64(u: &Unit, off: u64, v: u64) {
    // SAFETY: viz r64.
    unsafe { ((u.regs + off) as *mut u64).write_volatile(v) }
}

fn entry(table: PhysAddr, index: u64) -> *mut u64 {
    PhysAddr::new(table.as_u64() + index * 8)
        .to_virt()
        .as_mut_ptr::<u64>()
}

impl Unit {
    /// Zaradi prikazy (po 16 B) a pocka na jejich dokonceni.
    fn run(&self, commands: &[[u32; 4]]) -> Result<(), Error> {
        let mut seq = self.cmd_lock.lock();
        *seq += 1;
        let wait = self.completion_wait(*seq);
        let mut tail = r64(self, CMD_TAIL) & (RING - 1);
        for c in commands.iter().chain(core::iter::once(&wait)) {
            let p = PhysAddr::new(self.cmd.as_u64() + tail)
                .to_virt()
                .as_mut_ptr::<u32>();
            for (i, &d) in c.iter().enumerate() {
                // SAFETY: fronta prikazu patri jednotce, zapis pod zamkem.
                unsafe { p.add(i).write_volatile(d) };
            }
            tail = (tail + 16) % RING;
        }
        core::sync::atomic::fence(Ordering::SeqCst);
        w64(self, CMD_TAIL, tail);
        let sem = self.sem.to_virt().as_mut_ptr::<u64>();
        let expected = *seq;
        let deadline = super::super::time::now_ns() + 1_000_000_000;
        // SAFETY: semafor je stranka jednotky; IOMMU do ni zapisuje.
        while unsafe { sem.read_volatile() } != expected {
            if super::super::time::now_ns() > deadline {
                log::error!("IOMMU AMD-Vi: prikazy nedobehly");
                return Err(Error::TimedOut);
            }
            core::hint::spin_loop();
        }
        Ok(())
    }

    /// COMPLETION_WAIT se zapisem `value` do semaforu.
    fn completion_wait(&self, value: u64) -> [u32; 4] {
        let a = self.sem.as_u64();
        [
            (a as u32 & !7) | 1,
            ((a >> 32) as u32 & 0xF_FFFF) | 0x1 << 28,
            value as u32,
            (value >> 32) as u32,
        ]
    }
}

fn invalidate_device(devid: u16) -> [u32; 4] {
    [devid as u32, 0x2 << 28, 0, 0]
}

fn invalidate_domain(domain: u16) -> [u32; 4] {
    // Vsechny stranky domeny: adresa 0x7FFF_FFFF_FFFF_F000, S = 1, PDE = 1.
    [
        0,
        domain as u32 | 0x3 << 28,
        0xFFFF_F000 | 0b11,
        0x7FFF_FFFF,
    ]
}

/// Zapne jednotky AMD-Vi (vola `iommu::init`).
///
/// # Safety
///
/// Volat jednou pri startu, pred spustenim ovladacu.
pub unsafe fn init(units: &[AmdIommuUnit], ecam: &[PciEcam]) {
    for info in units {
        match init_unit(info, ecam) {
            Ok(u) => {
                log::info!(
                    "IOMMU AMD-Vi na {:#x}: segment {}, {} urovne tabulek",
                    info.registers,
                    info.segment,
                    LEVELS
                );
                UNITS.lock().push(u);
            }
            Err(e) => log::error!("IOMMU na {:#x} nejde zapnout: {e}", info.registers),
        }
    }
    ACTIVE.store(!UNITS.lock().is_empty(), Ordering::Release);
}

fn init_unit(info: &AmdIommuUnit, ecam: &[PciEcam]) -> Result<&'static Unit, Error> {
    let regs = mm::map_mmio(PhysAddr::new(info.registers), 0x4000).as_u64();
    // Tabulka zarizeni: 2 MiB, vse zablokovane.
    let dev_table = mm::phys::alloc_pages(9).ok_or(Error::NoMemory)?;
    for i in 0..65_536u64 {
        let p = entry(dev_table, i * 4);
        // SAFETY: prave alokovana tabulka (512 stranek v prime mape).
        unsafe {
            p.write_volatile(DTE_V_TV);
            p.add(1).write_volatile(0);
            p.add(2).write_volatile(0);
            p.add(3).write_volatile(0);
        }
    }
    let cmd = mm::phys::alloc_frame_zeroed().ok_or(Error::NoMemory)?;
    let evt = mm::phys::alloc_frame_zeroed().ok_or(Error::NoMemory)?;
    let sem = mm::phys::alloc_frame_zeroed().ok_or(Error::NoMemory)?;
    let unit: &'static Unit = alloc::boxed::Box::leak(alloc::boxed::Box::new(Unit {
        regs,
        segment: info.segment,
        dev_table,
        cmd,
        evt,
        sem,
        cmd_lock: SpinLock::new(0),
    }));
    // Firmware mohl IOMMU zapnout: nejdriv vypnout, pak nastavit.
    w64(unit, CONTROL, 0);
    w64(unit, DEV_TABLE, dev_table.as_u64() | 511);
    w64(unit, CMD_BASE, cmd.as_u64() | RING_LEN_LOG2 << 56);
    w64(unit, EVT_BASE, evt.as_u64() | RING_LEN_LOG2 << 56);
    w64(unit, CMD_HEAD, 0);
    w64(unit, CMD_TAIL, 0);
    w64(unit, EVT_HEAD, 0);
    w64(unit, EVT_TAIL, 0);
    let msi = enable_msi(info, ecam);
    let mut ctl = CTL_IOMMU_EN | CTL_CMD_BUF_EN | CTL_EVENT_LOG_EN;
    if msi {
        ctl |= CTL_EVENT_INT_EN;
    } else {
        log::warn!(
            "IOMMU AMD-Vi: preruseni MSI nejde nastavit, zablokovane pristupy se nebudou hlasit"
        );
    }
    w64(unit, CONTROL, ctl);
    // Zaznamy tabulky zarizeni mohla mit jednotka v cache z doby firmwaru.
    let all: Vec<[u32; 4]> = (0..=u16::MAX).map(invalidate_device).collect();
    for chunk in all.chunks(200) {
        unit.run(chunk)?;
    }
    Ok(unit)
}

/// Nastavi preruseni MSI jednotky v jejim konfiguracnim prostoru PCI.
fn enable_msi(info: &AmdIommuUnit, ecam: &[PciEcam]) -> bool {
    let bus = (info.device_id >> 8) as u8;
    let devfn = info.device_id as u8;
    let Some(w) = ecam
        .iter()
        .find(|e| e.segment == info.segment && (e.bus_start..=e.bus_end).contains(&bus))
    else {
        return false;
    };
    let phys = w.phys + (((bus - w.bus_start) as u64) << 20 | (devfn as u64) << 12);
    let cfg = mm::map_mmio(PhysAddr::new(phys), 4096).as_u64();
    // SAFETY: stranka konfiguracniho prostoru IOMMU, namapovana bez cache.
    let r8 = |o: u64| unsafe { ((cfg + o) as *const u8).read_volatile() };
    let r16 = |o: u64| unsafe { ((cfg + o) as *const u16).read_volatile() };
    // SAFETY: viz vyse.
    let w16 = |o: u64, v: u16| unsafe { ((cfg + o) as *mut u16).write_volatile(v) };
    // SAFETY: viz vyse.
    let w32 = |o: u64, v: u32| unsafe { ((cfg + o) as *mut u32).write_volatile(v) };
    if r16(0x06) & (1 << 4) == 0 {
        return false;
    }
    let mut ptr = (r8(0x34) & !3) as u64;
    for _ in 0..48 {
        if ptr == 0 {
            return false;
        }
        if r8(ptr) == 0x05 {
            let ctrl = r16(ptr + 2);
            let (addr, data) = super::super::msi_message(VECTOR_IOMMU_FAULT);
            w32(ptr + 4, addr as u32);
            if ctrl & (1 << 7) != 0 {
                w32(ptr + 8, (addr >> 32) as u32);
                w16(ptr + 12, data as u16);
            } else {
                w16(ptr + 8, data as u16);
            }
            // Jedna zprava, zapnuto.
            w16(ptr + 2, (ctrl & !(0b111 << 4)) | 1);
            return true;
        }
        ptr = (r8(ptr + 1) & !3) as u64;
    }
    false
}

/// Je AMD-Vi aktivni?
pub fn active() -> bool {
    ACTIVE.load(Ordering::Acquire)
}

/// Domena prekladu DMA jednoho zarizeni.
pub struct Domain {
    unit: &'static Unit,
    id: u16,
    segment: u16,
    devid: u16,
    root: PhysAddr,
    tables: SpinLock<Vec<PhysAddr>>,
}

/// Domena zarizeni; `Ok(None)` = zadna jednotka AMD-Vi segment nepokryva.
pub fn domain(segment: u16, bus: u8, devfn: u8) -> Result<Option<Arc<Domain>>, Error> {
    if !active() {
        return Ok(None);
    }
    let mut domains = DOMAINS.lock();
    if let Some(d) = domains.get(&(segment, bus, devfn)).and_then(Weak::upgrade) {
        return Ok(Some(d));
    }
    let Some(unit) = UNITS.lock().iter().find(|u| u.segment == segment).copied() else {
        return Ok(None);
    };
    let root = mm::phys::alloc_frame_zeroed().ok_or(Error::NoMemory)?;
    let d = Arc::new(Domain {
        unit,
        id: NEXT_DOMAIN.fetch_add(1, Ordering::Relaxed),
        segment,
        devid: (bus as u16) << 8 | devfn as u16,
        root,
        tables: SpinLock::new(alloc::vec![root]),
    });
    d.set_dte(
        d.root.as_u64() | LEVELS << 9 | DTE_V_TV | DTE_IR | DTE_IW,
        d.id as u64,
    )?;
    domains.insert((segment, bus, devfn), Arc::downgrade(&d));
    Ok(Some(d))
}

impl Domain {
    /// Zapise zaznam tabulky zarizeni (nejdriv domena, pak platnost).
    fn set_dte(&self, q0: u64, q1: u64) -> Result<(), Error> {
        let p = entry(self.unit.dev_table, self.devid as u64 * 4);
        // SAFETY: zaznam tabulky zarizeni patri jednotce, zapis pod zamkem domen.
        unsafe {
            p.add(1).write_volatile(q1);
            p.write_volatile(q0);
        }
        self.unit.run(&[invalidate_device(self.devid)])
    }

    /// Zaznam posledni urovne pro adresu `iova` (vytvori chybejici tabulky).
    fn leaf(&self, tables: &mut Vec<PhysAddr>, iova: u64) -> Result<*mut u64, Error> {
        let mut table = self.root;
        for level in (2..=LEVELS).rev() {
            let p = entry(table, (iova >> (12 + 9 * (level - 1))) & 511);
            // SAFETY: tabulky domeny, pod jejim zamkem.
            let mut v = unsafe { p.read_volatile() };
            if v & PTE_PR == 0 {
                let new = mm::phys::alloc_frame_zeroed().ok_or(Error::NoMemory)?;
                tables.push(new);
                v = new.as_u64() | (level - 1) << 9 | PTE_RW | PTE_PR;
                // SAFETY: viz vyse.
                unsafe { p.write_volatile(v) };
            }
            table = PhysAddr::new(v & ADDR_MASK);
        }
        Ok(entry(table, (iova >> 12) & 511))
    }

    pub fn map(&self, phys: u64, pages: u64) -> Result<(), Error> {
        let mut tables = self.tables.lock();
        for i in 0..pages {
            let addr = phys + i * 4096;
            match self.leaf(&mut tables, addr) {
                // SAFETY: zaznam v tabulce domeny, pod zamkem.
                Ok(p) => unsafe { p.write_volatile(addr | PTE_RW | PTE_PR) },
                Err(e) => {
                    drop(tables);
                    self.unmap(phys, i);
                    return Err(e);
                }
            }
        }
        drop(tables);
        self.unit.run(&[invalidate_domain(self.id)])
    }

    pub fn unmap(&self, phys: u64, pages: u64) {
        let mut tables = self.tables.lock();
        for i in 0..pages {
            if let Ok(p) = self.leaf(&mut tables, phys + i * 4096) {
                // SAFETY: viz map.
                unsafe { p.write_volatile(0) };
            }
        }
        drop(tables);
        let _ = self.unit.run(&[invalidate_domain(self.id)]);
    }
}

impl Drop for Domain {
    fn drop(&mut self) {
        // Zarizeni zpet do "vse zablokovano", pak uvolnit tabulky. Mezitim
        // mohla pro stejne zarizeni vzniknout nova domena - pak nic nemenit.
        let mut domains = DOMAINS.lock();
        let key = (self.segment, (self.devid >> 8) as u8, self.devid as u8);
        let ours = domains
            .get(&key)
            .is_some_and(|w| core::ptr::eq(w.as_ptr(), self));
        if ours {
            domains.remove(&key);
            let _ = self.set_dte(DTE_V_TV, 0);
            let _ = self.unit.run(&[invalidate_domain(self.id)]);
        }
        drop(domains);
        for t in self.tables.get_mut().drain(..) {
            mm::phys::free_pages(t, 0);
        }
    }
}

/// Obsluha preruseni: zaloguje udalosti (zablokovane pristupy) a posune
/// cteci pozici zaznamu udalosti.
pub fn on_fault_interrupt() {
    for &u in UNITS.lock().iter() {
        let tail = r64(u, EVT_TAIL) & (RING - 1);
        let mut head = r64(u, EVT_HEAD) & (RING - 1);
        // Jeden zablokovany prenos muze vyvolat desitky udalosti: podrobne
        // jen prvni 4, zbytek souhrnem (log nesmi zahltit system).
        let mut shown = 0;
        let mut more = 0u32;
        while head != tail {
            let p = PhysAddr::new(u.evt.as_u64() + head)
                .to_virt()
                .as_mut_ptr::<u32>();
            // SAFETY: zaznam udalosti patri jednotce; IOMMU ho uz zapsala.
            let e: [u32; 4] = unsafe { [0, 1, 2, 3].map(|i| p.add(i).read_volatile()) };
            let code = e[1] >> 28;
            let devid = e[0] & 0xFFFF;
            let addr = (e[3] as u64) << 32 | e[2] as u64;
            head = (head + 16) % RING;
            if shown == 4 {
                more += 1;
                continue;
            }
            shown += 1;
            log::warn!(
                "IOMMU: zablokovan pristup zarizeni {:02x}:{:02x}.{} na adrese {:#x} (udalost AMD-Vi {code:#x})",
                devid >> 8,
                (devid >> 3) & 0x1F,
                devid & 7,
                addr & !0xFFF
            );
        }
        if more > 0 {
            log::warn!("IOMMU: dalsich {more} udalosti AMD-Vi vynechano");
        }
        w64(u, EVT_HEAD, head);
        // Priznaky preruseni a preteceni zaznamu smazat.
        w64(u, STATUS, 0b11);
    }
}
