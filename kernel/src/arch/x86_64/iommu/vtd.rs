//! IOMMU: backend Intel VT-d (spolecna pravidla viz `iommu/mod.rs`).
//!
//! Struktury (specifikace VT-d): korenova tabulka (256 sbernic x 16 B) ->
//! tabulka kontextu sbernice (256 funkci x 16 B) -> strankove tabulky druhe
//! urovne (3 nebo 4 urovne, zaznamy jako u x86: bit 0 cteni, bit 1 zapis).
//! Po kazde zmene se zneplatni cache kontextu a IOTLB (globalne, pres
//! registry). TODO: fronta zneplatneni (QI), preklad preruseni, RMRR.

use alloc::collections::BTreeMap;
use alloc::sync::{Arc, Weak};
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicU16, Ordering};

use super::super::acpi::IommuUnit;
use crate::arch::api::DmaAccess;
use crate::error::Error;
use crate::log;
use crate::mm::{self, PhysAddr};
use crate::sync::SpinLock;

// Registry (offsety).
const CAP: u64 = 0x08;
const ECAP: u64 = 0x10;
const GCMD: u64 = 0x18;
const GSTS: u64 = 0x1C;
const RTADDR: u64 = 0x20;
const CCMD: u64 = 0x28;
const FSTS: u64 = 0x34;
const FECTL: u64 = 0x38;
const FEDATA: u64 = 0x3C;
const FEADDR: u64 = 0x40;
const FEUADDR: u64 = 0x44;

const GCMD_TE: u32 = 1 << 31;
const GCMD_SRTP: u32 = 1 << 30;
const GCMD_WBF: u32 = 1 << 27;
const STS_TES: u32 = 1 << 31;
const STS_RTPS: u32 = 1 << 30;
const STS_WBFS: u32 = 1 << 27;

const PAGE: u64 = 4096;
/// Zaznam strankove tabulky druhe urovne: cteni | zapis.
const SL_RW: u64 = 0b11;
const SL_R: u64 = 0b01;
const SL_W: u64 = 0b10;

/// Vektor preruseni pro hlaseni chyb DMA (viz `apic::VECTOR_IOMMU_FAULT`).
use super::VECTOR_IOMMU_FAULT;

/// Jedna jednotka VT-d.
struct Unit {
    regs: u64,
    segment: u16,
    include_all: bool,
    devices: Vec<(u8, u8)>,
    cap: u64,
    /// Strankove tabulky nejsou koherentni s cache CPU (ECAP.C = 0): po
    /// zapisu je treba radky vyprazdnit.
    flush_cache: bool,
    /// Pocet urovni strankovych tabulek (3 = 39 bitu, 4 = 48 bitu).
    levels: u8,
    /// Hodnota AW v zaznamu kontextu.
    aw: u64,
    iotlb: u64,
    root: PhysAddr,
    /// Softwarova kopie trvalych bitu GCMD (spec: GCMD se nesmi cist).
    gcmd: SpinLock<u32>,
}

static UNITS: SpinLock<Vec<&'static Unit>> = SpinLock::new(Vec::new());
static ACTIVE: AtomicBool = AtomicBool::new(false);
static NEXT_DOMAIN: AtomicU16 = AtomicU16::new(1);
/// Domeny podle zarizeni (segment, sbernice, zarizeni << 3 | funkce).
static DOMAINS: SpinLock<BTreeMap<(u16, u8, u8), Weak<Domain>>> = SpinLock::new(BTreeMap::new());

fn r32(u: &Unit, off: u64) -> u32 {
    // SAFETY: registry jednotky jsou namapovane (map_mmio) po celou dobu behu.
    unsafe { ((u.regs + off) as *const u32).read_volatile() }
}

fn w32(u: &Unit, off: u64, v: u32) {
    // SAFETY: viz r32.
    unsafe { ((u.regs + off) as *mut u32).write_volatile(v) }
}

fn r64(u: &Unit, off: u64) -> u64 {
    // SAFETY: viz r32.
    unsafe { ((u.regs + off) as *const u64).read_volatile() }
}

fn w64(u: &Unit, off: u64, v: u64) {
    // SAFETY: viz r32.
    unsafe { ((u.regs + off) as *mut u64).write_volatile(v) }
}

/// Ceka, az `cond` plati (nejvys ~1 s pres TSC); jinak chyba.
fn wait(what: &str, mut cond: impl FnMut() -> bool) -> Result<(), Error> {
    let deadline = super::super::time::now_ns() + 1_000_000_000;
    while !cond() {
        if super::super::time::now_ns() > deadline {
            log::error!("IOMMU: {what} nedobehlo");
            return Err(Error::TimedOut);
        }
        core::hint::spin_loop();
    }
    Ok(())
}

/// Vyprazdni radek cache s 16B zaznamem (jednotka bez koherence).
fn flush_line(u: &Unit, phys: PhysAddr) {
    if u.flush_cache {
        let p = phys.to_virt().as_u64();
        // SAFETY: clflush jen zapise radek cache do pameti.
        unsafe { core::arch::asm!("clflush [{}]", in(reg) p, options(nostack, preserves_flags)) };
    }
}

impl Unit {
    /// Prikaz pres GCMD: trvale bity z kopie + jednorazovy `bit`.
    fn command(&self, bit: u32, persistent: bool, done: impl Fn(u32) -> bool) -> Result<(), Error> {
        let mut g = self.gcmd.lock();
        let value = *g | bit;
        if persistent {
            *g = value;
        }
        w32(self, GCMD, value);
        wait("prikaz GCMD", || done(r32(self, GSTS)))
    }

    /// Zapise obsah zapisove fronty jednotky (kdyz ji ma) a zneplatni cache
    /// kontextu a IOTLB.
    fn invalidate(&self) {
        // SAFETY bariery: zaznamy tabulek musi byt v pameti pred zneplatnenim.
        core::sync::atomic::fence(Ordering::SeqCst);
        if self.cap & (1 << 4) != 0 {
            let _ = self.command(GCMD_WBF, false, |s| s & STS_WBFS == 0);
        }
        w64(self, CCMD, 1 << 63 | 1 << 61);
        let _ = wait("zneplatneni kontextu", || r64(self, CCMD) & (1 << 63) == 0);
        w64(self, self.iotlb + 8, 1 << 63 | 1 << 60);
        let _ = wait("zneplatneni IOTLB", || {
            r64(self, self.iotlb + 8) & (1 << 63) == 0
        });
    }

    fn covers(&self, segment: u16, bus: u8, devfn: u8) -> bool {
        self.segment == segment && (self.include_all || self.devices.contains(&(bus, devfn)))
    }
}

/// Zapne jednotky VT-d (vola `iommu::init`).
///
/// # Safety
///
/// Volat jednou pri startu, pred spustenim ovladacu.
pub unsafe fn init(units: &[IommuUnit]) {
    for info in units {
        match init_unit(info) {
            Ok(u) => {
                log::info!(
                    "IOMMU VT-d na {:#x}: segment {}, {} urovne tabulek, {}",
                    info.registers,
                    info.segment,
                    u.levels,
                    if info.include_all {
                        alloc::string::String::from("vsechna zarizeni")
                    } else {
                        alloc::format!("{} vyjmenovanych zarizeni", info.devices.len())
                    }
                );
                UNITS.lock().push(u);
            }
            Err(e) => log::error!("IOMMU na {:#x} nejde zapnout: {e}", info.registers),
        }
    }
    ACTIVE.store(!UNITS.lock().is_empty(), Ordering::Release);
}

fn init_unit(info: &IommuUnit) -> Result<&'static Unit, Error> {
    let regs = mm::map_mmio(PhysAddr::new(info.registers), PAGE).as_u64();
    let probe = Unit {
        regs,
        segment: info.segment,
        include_all: info.include_all,
        devices: info.devices.clone(),
        cap: 0,
        flush_cache: true,
        levels: 0,
        aw: 0,
        iotlb: 0,
        root: PhysAddr::new(0),
        gcmd: SpinLock::new(0),
    };
    let cap = r64(&probe, CAP);
    let ecap = r64(&probe, ECAP);
    let sagaw = (cap >> 8) & 0x1F;
    let (levels, aw) = if sagaw & (1 << 2) != 0 {
        (4, 2)
    } else if sagaw & (1 << 1) != 0 {
        (3, 1)
    } else {
        return Err(Error::NotSupported);
    };
    let root = mm::phys::alloc_frame_zeroed().ok_or(Error::NoMemory)?;
    let unit: &'static Unit = alloc::boxed::Box::leak(alloc::boxed::Box::new(Unit {
        cap,
        flush_cache: ecap & 1 == 0,
        levels,
        aw,
        iotlb: ((ecap >> 8) & 0x3FF) * 16,
        root,
        ..probe
    }));
    // Firmware mohl preklad zapnout s vlastnimi tabulkami: vypnout.
    if r32(unit, GSTS) & STS_TES != 0 {
        w32(unit, GCMD, 0);
        wait("vypnuti prekladu", || r32(unit, GSTS) & STS_TES == 0)?;
    }
    w64(unit, RTADDR, root.as_u64());
    unit.command(GCMD_SRTP, false, |s| s & STS_RTPS != 0)?;
    unit.invalidate();
    // Hlaseni chyb preruseni na prvni CPU.
    let (addr, data) = super::super::msi_message(VECTOR_IOMMU_FAULT);
    w32(unit, FEDATA, data);
    w32(unit, FEADDR, addr as u32);
    w32(unit, FEUADDR, (addr >> 32) as u32);
    w32(unit, FECTL, 0);
    unit.command(GCMD_TE, true, |s| s & STS_TES != 0)?;
    Ok(unit)
}

/// Je IOMMU aktivni?
pub fn active() -> bool {
    ACTIVE.load(Ordering::Acquire)
}

/// Domena pro preklad DMA jednoho zarizeni.
pub struct Domain {
    unit: &'static Unit,
    id: u16,
    segment: u16,
    bus: u8,
    devfn: u8,
    /// Nejvyssi strankova tabulka.
    top: PhysAddr,
    tables: SpinLock<Vec<PhysAddr>>,
}

/// Domena zarizeni (segment, sbernice, zarizeni << 3 | funkce): existujici,
/// nebo nova. `Ok(None)` = zadna IOMMU zarizeni nepokryva (DMA neomezene).
pub fn domain(segment: u16, bus: u8, devfn: u8) -> Result<Option<Arc<Domain>>, Error> {
    if !active() {
        return Ok(None);
    }
    let mut domains = DOMAINS.lock();
    if let Some(d) = domains.get(&(segment, bus, devfn)).and_then(Weak::upgrade) {
        return Ok(Some(d));
    }
    // Jednotka s vyjmenovanym zarizenim ma prednost pred "vsechna zarizeni".
    let unit = {
        let units = UNITS.lock();
        let exact = units
            .iter()
            .find(|u| u.segment == segment && u.devices.contains(&(bus, devfn)));
        let Some(u) = exact.or_else(|| units.iter().find(|u| u.covers(segment, bus, devfn))) else {
            return Ok(None);
        };
        *u
    };
    let top = mm::phys::alloc_frame_zeroed().ok_or(Error::NoMemory)?;
    let id = NEXT_DOMAIN.fetch_add(1, Ordering::Relaxed);
    let d = Arc::new(Domain {
        unit,
        id,
        segment,
        bus,
        devfn,
        top,
        tables: SpinLock::new(alloc::vec![top]),
    });
    d.attach()?;
    domains.insert((segment, bus, devfn), Arc::downgrade(&d));
    Ok(Some(d))
}

impl Domain {
    /// Zapise zaznam kontextu zarizeni (od ted preklad pres tuto domenu).
    fn attach(&self) -> Result<(), Error> {
        let u = self.unit;
        let root_entry = PhysAddr::new(u.root.as_u64() + self.bus as u64 * 16);
        let re = root_entry.to_virt().as_mut_ptr::<u64>();
        // SAFETY: korenova tabulka patri jednotce, zapis pod zamkem domen.
        let mut ctx = unsafe { re.read_volatile() };
        if ctx & 1 == 0 {
            let table = mm::phys::alloc_frame_zeroed().ok_or(Error::NoMemory)?;
            ctx = table.as_u64() | 1;
            // SAFETY: viz vyse.
            unsafe { re.write_volatile(ctx) };
            flush_line(u, root_entry);
        }
        let entry = PhysAddr::new((ctx & !0xFFF) + self.devfn as u64 * 16);
        let ce = entry.to_virt().as_mut_ptr::<u64>();
        // SAFETY: tabulka kontextu sbernice patri jednotce; nejdriv horni
        // polovina (sirka, domena), pak dolni s bitem "pritomno".
        unsafe {
            ce.add(1).write_volatile(u.aw | (self.id as u64) << 8);
            ce.write_volatile(self.top.as_u64() | 1);
        }
        flush_line(u, entry);
        u.invalidate();
        Ok(())
    }

    /// Zaznam posledni urovne pro adresu `iova` (vytvori chybejici tabulky).
    fn leaf(&self, tables: &mut Vec<PhysAddr>, iova: u64) -> Result<PhysAddr, Error> {
        let mut table = self.top;
        for level in (1..self.unit.levels).rev() {
            let index = (iova >> (12 + 9 * level as u64)) & 511;
            let e = PhysAddr::new(table.as_u64() + index * 8);
            let p = e.to_virt().as_mut_ptr::<u64>();
            // SAFETY: tabulky domeny, pod jejim zamkem.
            let mut v = unsafe { p.read_volatile() };
            if v & SL_RW == 0 {
                let new = mm::phys::alloc_frame_zeroed().ok_or(Error::NoMemory)?;
                tables.push(new);
                v = new.as_u64() | SL_RW;
                // SAFETY: viz vyse.
                unsafe { p.write_volatile(v) };
                flush_line(self.unit, e);
            }
            table = PhysAddr::new(v & 0x000F_FFFF_FFFF_F000);
        }
        Ok(PhysAddr::new(table.as_u64() + ((iova >> 12) & 511) * 8))
    }

    /// Kolik bitu adresy domena preklada (3 urovne = 39, 4 = 48).
    pub fn iova_bits(&self) -> u32 {
        12 + 9 * self.unit.levels as u32
    }

    /// Povoli zarizeni pristup `access` na `iova..+pages` stranek
    /// k souvislym strankam od `phys`.
    pub fn map(&self, iova: u64, phys: u64, pages: u64, access: DmaAccess) -> Result<(), Error> {
        let bits = access.read as u64 * SL_R | access.write as u64 * SL_W;
        let mut tables = self.tables.lock();
        for i in 0..pages {
            let e = match self.leaf(&mut tables, iova + i * PAGE) {
                Ok(e) => e,
                Err(err) => {
                    drop(tables);
                    self.unmap(iova, i);
                    return Err(err);
                }
            };
            // SAFETY: zaznam v tabulce domeny, pod zamkem.
            unsafe { e.to_virt().as_mut_ptr::<u64>().write_volatile((phys + i * PAGE) | bits) };
            flush_line(self.unit, e);
        }
        self.unit.invalidate();
        Ok(())
    }

    /// Odebere zarizeni pristup ke strankam a vyprazdni IOTLB (synchronne,
    /// `invalidate` ceka na dokonceni).
    pub fn unmap(&self, iova: u64, pages: u64) {
        let mut tables = self.tables.lock();
        for i in 0..pages {
            if let Ok(e) = self.leaf(&mut tables, iova + i * PAGE) {
                // SAFETY: viz map.
                unsafe { e.to_virt().as_mut_ptr::<u64>().write_volatile(0) };
                flush_line(self.unit, e);
            }
        }
        self.unit.invalidate();
    }
}

impl Drop for Domain {
    fn drop(&mut self) {
        // Odpojit zarizeni (zaznam kontextu), pak uvolnit tabulky. Pod zamkem
        // domen: mezitim mohla pro stejne zarizeni vzniknout nova domena -
        // pak jeji zaznam kontextu nemazat.
        let mut domains = DOMAINS.lock();
        let key = (self.segment, self.bus, self.devfn);
        let ours = domains
            .get(&key)
            .is_some_and(|w| core::ptr::eq(w.as_ptr(), self));
        if ours {
            domains.remove(&key);
        }
        let u = self.unit;
        let root_entry = PhysAddr::new(u.root.as_u64() + self.bus as u64 * 16);
        // SAFETY: viz attach.
        let ctx = unsafe { root_entry.to_virt().as_mut_ptr::<u64>().read_volatile() };
        if ours && ctx & 1 != 0 {
            let entry = PhysAddr::new((ctx & !0xFFF) + self.devfn as u64 * 16);
            // SAFETY: viz attach.
            unsafe { entry.to_virt().as_mut_ptr::<u64>().write_volatile(0) };
            flush_line(u, entry);
            u.invalidate();
        }
        drop(domains);
        for t in self.tables.get_mut().drain(..) {
            mm::phys::free_pages(t, 0);
        }
    }
}

/// Obsluha preruseni chyb: zaloguje zablokovane pristupy a smaze zaznamy.
pub fn on_fault_interrupt() {
    for &u in UNITS.lock().iter() {
        let fro = ((u.cap >> 24) & 0x3FF) * 16;
        let records = ((u.cap >> 40) & 0xFF) + 1;
        for i in 0..records {
            let off = fro + i * 16;
            let high = r64(u, off + 8);
            if high & (1 << 63) == 0 {
                continue;
            }
            let low = r64(u, off);
            let source = high & 0xFFFF;
            let reason = (high >> 32) & 0xFF;
            let read = high & (1 << 62) != 0;
            log::warn!(
                "IOMMU: zablokovan {} zarizeni {:02x}:{:02x}.{} na adrese {:#x} (duvod {reason:#x})",
                if read { "cteni" } else { "zapis" },
                source >> 8,
                (source >> 3) & 0x1F,
                source & 7,
                low & !0xFFF
            );
            w64(u, off + 8, 1 << 63);
        }
        // Preteceni zaznamu (PFO) smazat.
        w32(u, FSTS, 1);
    }
}
