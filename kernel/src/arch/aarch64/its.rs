//! ITS (Interrupt Translation Service) GICv3: preklad MSI zprav zarizeni na
//! LPI.
//!
//! Zarizeni zapise do registru GITS_TRANSLATER cislo udalosti (EventID);
//! ITS podle identity zarizeni (DeviceID, u PCI = RID: sbernice << 8 |
//! zarizeni << 3 | funkce, na QEMU virt mapovane identicky - viz IORT)
//! a udalosti najde LPI a CPU (kolekci) a preruseni doruci. Jadro ITS ridi
//! frontou prikazu v pameti:
//!
//! - MAPD: zarizeni -> tabulka prekladu (ITT) zarizeni,
//! - MAPC: kolekce -> redistributor CPU,
//! - MAPTI: (zarizeni, udalost) -> LPI a kolekce,
//! - DISCARD: zrusi preklad udalosti, INV: nacti znovu konfiguraci LPI,
//! - SYNC: pockej na dokonceni predchozich prikazu.
//!
//! Udalost = cislo LPI - 8192, takze stejne cislo preruseni ma vzdy stejnou
//! udalost a zprava MSI je dana jen cislem preruseni. Vsechna preruseni
//! chodi na bootovaci CPU (kolekce 0), stejne jako na x86.

use alloc::collections::BTreeMap;
use core::sync::atomic::{AtomicU64, Ordering};

use super::gic::{self, LPI_FIRST};
use super::regs::{dcache_clean_invalidate, dsb_sy};
use crate::error::Error;
use crate::log;
use crate::mm::{self, PhysAddr};
use crate::sync::SpinLock;

const GITS_CTLR: u64 = 0x0000;
const GITS_CTLR_ENABLED: u32 = 1 << 0;
const GITS_CTLR_QUIESCENT: u32 = 1 << 31;
const GITS_TYPER: u64 = 0x0008;
const GITS_CBASER: u64 = 0x0080;
const GITS_CWRITER: u64 = 0x0088;
const GITS_CREADR: u64 = 0x0090;
const GITS_BASER: u64 = 0x0100;
const GITS_TRANSLATER: u64 = 0x1_0040;

const CMD_SIZE: u64 = 32;
/// Fronta prikazu: jedna stranka (128 prikazu).
const QUEUE_LEN: u64 = 4096;
/// Udalosti na zarizeni: cela rada LPI jadra (EventID = LPI - 8192).
const EVENT_BITS: u32 = 10;
const EVENTS: u32 = 1 << EVENT_BITS;

/// Atributy cache tabulek ITS: Write-Back inner (7 << 59) a sdileni
/// inner (1 << 10).
const BASER_ATTRS: u64 = (7 << 59) | (1 << 10);

struct Its {
    /// Virtualni adresa registru.
    regs: u64,
    /// Fronta prikazu (fyzicka adresa) a pozice zapisu.
    queue: u64,
    write: u64,
    /// Velikost polozky ITT (GITS_TYPER.ITT_entry_size).
    itt_entry: u64,
    /// Cilovy redistributor kolekce 0 (podle GITS_TYPER.PTA).
    target: u64,
    /// Zarizeni s ITT: DeviceID -> fyzicka adresa ITT.
    devices: BTreeMap<u32, u64>,
    /// Komu patri udalost (index = udalost): DeviceID.
    owners: [Option<u32>; EVENTS as usize],
}

static ITS: SpinLock<Option<Its>> = SpinLock::new(None);
/// Fyzicka adresa ITS (pro zpravy MSI) - jen cteni, bez zamku.
static ITS_PHYS: AtomicU64 = AtomicU64::new(0);

fn read32(addr: u64) -> u32 {
    // SAFETY: registry ITS namapovane jako zarizeni.
    unsafe { (addr as *const u32).read_volatile() }
}
fn write32(addr: u64, v: u32) {
    // SAFETY: viz `read32`.
    unsafe { (addr as *mut u32).write_volatile(v) }
}
fn read64(addr: u64) -> u64 {
    // SAFETY: viz `read32`.
    unsafe { (addr as *const u64).read_volatile() }
}
fn write64(addr: u64, v: u64) {
    // SAFETY: viz `read32`.
    unsafe { (addr as *mut u64).write_volatile(v) }
}

fn alloc_zeroed(bytes: u64) -> PhysAddr {
    let order = bytes.div_ceil(4096).next_power_of_two().trailing_zeros() as usize;
    let p = mm::phys::alloc_pages(order).expect("nedostatek pameti pro tabulky ITS");
    // SAFETY: prave alokovane stranky v prime mape.
    unsafe { p.to_virt().as_mut_ptr::<u8>().write_bytes(0, 4096 << order) };
    dcache_clean_invalidate(p.to_virt().as_u64(), 4096 << order);
    p
}

/// Zapne ITS na `phys` (z MADT). Volat po `gic::init_distributor` a po
/// inicializaci GIC na bootovacim CPU. Bez ITS nefunguji MSI zarizeni.
pub fn init(phys: u64) {
    let regs = mm::map_mmio(PhysAddr::new(phys), 0x2_0000).as_u64();
    // ITS musi byt pri nastavovani vypnuty a v klidu.
    write32(regs + GITS_CTLR, 0);
    let mut spins = 0u32;
    while read32(regs + GITS_CTLR) & GITS_CTLR_QUIESCENT == 0 && spins < 10_000_000 {
        core::hint::spin_loop();
        spins += 1;
    }
    let typer = read64(regs + GITS_TYPER);
    let itt_entry = ((typer >> 4) & 0xF) + 1;
    let pta = typer & (1 << 19) != 0;
    let dev_bits = ((typer >> 13) & 0x1F) + 1;

    // Tabulky ITS (GITS_BASER0..7): zarizeni (typ 1) a kolekce (typ 4).
    for n in 0..8 {
        let reg = regs + GITS_BASER + n * 8;
        let baser = read64(reg);
        let kind = (baser >> 56) & 7;
        let entry = ((baser >> 48) & 0x1F) + 1;
        let entries = match kind {
            1 => 1u64 << dev_bits.min(16),
            4 => 64,
            _ => continue,
        };
        let pages = (entries * entry).div_ceil(4096).clamp(1, 256);
        let table = alloc_zeroed(pages * 4096);
        // Ploska tabulka, stranky 4 KiB, velikost v poctu stranek - 1.
        write64(
            reg,
            (1 << 63)
                | BASER_ATTRS
                | (kind << 56)
                | ((entry - 1) << 48)
                | table.as_u64()
                | (pages - 1),
        );
    }

    let queue = alloc_zeroed(QUEUE_LEN);
    write64(
        regs + GITS_CBASER,
        (1 << 63) | BASER_ATTRS | queue.as_u64() | (QUEUE_LEN / 4096 - 1),
    );
    write64(regs + GITS_CWRITER, 0);
    dsb_sy();
    write32(regs + GITS_CTLR, GITS_CTLR_ENABLED);

    // Kolekce 0 -> redistributor bootovaciho CPU.
    let rd = gic::redistributor();
    let target = if pta {
        // Fyzicka adresa redistributoru (bity 51:16).
        mm::translate(crate::mm::VirtAddr::new(rd)).map_or(0, |p| p.as_u64())
    } else {
        // Cislo procesoru (GICR_TYPER.Processor_Number) << 16.
        // SAFETY: registr TYPER redistributoru tohoto CPU.
        let typer = unsafe { ((rd + 8) as *const u64).read_volatile() };
        ((typer >> 8) & 0xFFFF) << 16
    };
    let mut its = Its {
        regs,
        queue: queue.as_u64(),
        write: 0,
        itt_entry,
        target,
        devices: BTreeMap::new(),
        owners: [None; EVENTS as usize],
    };
    its.command([0x09, 0, (1 << 63) | (target & 0x000F_FFFF_FFFF_0000), 0]); // MAPC ICID 0
    its.command([0x0D, 0, 0, 0]); // INVALL ICID 0
    its.sync();
    ITS_PHYS.store(phys, Ordering::Release);
    *ITS.lock() = Some(its);
    log::info!("ITS na {phys:#x}: polozka ITT {itt_entry} B, DeviceID {dev_bits} bitu");
}

impl Its {
    /// Zapise prikaz do fronty a pocka, az ho ITS zpracuje.
    fn command(&mut self, cmd: [u64; 4]) {
        let slot = PhysAddr::new(self.queue + self.write).to_virt().as_u64();
        // SAFETY: slot fronty prikazu (32 B) v prime mape.
        unsafe {
            let p = slot as *mut u64;
            for (i, w) in cmd.iter().enumerate() {
                p.add(i).write_volatile(*w);
            }
        }
        dcache_clean_invalidate(slot, CMD_SIZE);
        self.write = (self.write + CMD_SIZE) % QUEUE_LEN;
        write64(self.regs + GITS_CWRITER, self.write);
        let mut spins = 0u32;
        while read64(self.regs + GITS_CREADR) & 0xF_FFE0 != self.write && spins < 10_000_000 {
            core::hint::spin_loop();
            spins += 1;
        }
        if spins == 10_000_000 {
            log::error!(
                "ITS nezpracoval prikaz {:#x} (CREADR {:#x})",
                cmd[0] & 0xFF,
                read64(self.regs + GITS_CREADR)
            );
        }
    }

    fn sync(&mut self) {
        self.command([0x05, 0, self.target & 0x000F_FFFF_FFFF_0000, 0]);
    }

    /// Zalozi ITT zarizeni (MAPD), pokud jeste neni.
    fn ensure_device(&mut self, device: u32) {
        if self.devices.contains_key(&device) {
            return;
        }
        let itt = alloc_zeroed((EVENTS as u64 * self.itt_entry).max(256));
        self.command([
            0x08 | ((device as u64) << 32),
            (EVENT_BITS - 1) as u64,
            (1 << 63) | (itt.as_u64() & 0x000F_FFFF_FFFF_FF00),
            0,
        ]);
        self.devices.insert(device, itt.as_u64());
    }

    /// Prelozi udalost `event` zarizeni `device` na LPI `intid` (MAPTI).
    fn map(&mut self, device: u32, intid: u32) {
        let event = intid - LPI_FIRST;
        match self.owners[event as usize] {
            Some(owner) if owner == device => return,
            Some(owner) => {
                // Preruseni dostal jiny ovladac: stary preklad zrusit.
                self.command([0x0F | ((owner as u64) << 32), event as u64, 0, 0]);
            }
            None => {}
        }
        self.ensure_device(device);
        gic::set_lpi_enabled(intid, true);
        self.command([
            0x0A | ((device as u64) << 32),
            event as u64 | ((intid as u64) << 32),
            0, // kolekce 0
            0,
        ]);
        self.command([0x0C | ((device as u64) << 32), event as u64, 0, 0]); // INV
        self.sync();
        self.owners[event as usize] = Some(device);
    }
}

/// Zprava MSI pro preruseni `intid` zarizeni `device` (segment, sbernice,
/// zarizeni << 3 | funkce): adresa GITS_TRANSLATER, data = udalost.
pub fn msi_message(intid: u32, device: Option<(u16, u8, u8)>) -> Result<(u64, u32), Error> {
    let Some((segment, bus, devfn)) = device else {
        // Bez identity zarizeni ITS nevi, komu preruseni patri.
        return Err(Error::InvalidArgs);
    };
    if segment != 0 {
        // IORT s vice segmenty zatim necteme.
        return Err(Error::NotSupported);
    }
    let mut guard = ITS.lock();
    let its = guard.as_mut().ok_or(Error::NotSupported)?;
    let device_id = (bus as u32) << 8 | devfn as u32;
    its.map(device_id, intid);
    drop(guard);
    let doorbell = ITS_PHYS.load(Ordering::Acquire) + GITS_TRANSLATER;
    // Zprava MSI prochazi SMMU: zarizeni musi smet zapsat do ITS.
    super::smmu::map_msi_doorbell(segment, bus, devfn, doorbell);
    Ok((doorbell, intid - LPI_FIRST))
}
