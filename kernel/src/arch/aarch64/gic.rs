//! Radic preruseni GICv3: distributor (spolecny), redistributor (jeden na
//! CPU, SGI/PPI a LPI) a rozhrani CPU pres systemove registry ICC_*.
//!
//! Pouzivana preruseni:
//! - SGI 1-3: IPI jadra (preplanovani, zastaveni, vyprazdneni TLB),
//! - PPI 27: virtualni casovac (`timer`),
//! - LPI od 8192: preruseni zarizeni (MSI pres ITS, `its`).
//!
//! SPI (drata od zarizeni do distributoru) jadro zatim nepouziva - zarizeni
//! PCIe maji MSI. Vsechna preruseni jsou ve skupine 1 (IRQ) se stejnou
//! prioritou (bez vnoreni preruseni, stejne jako na x86).

use alloc::vec::Vec;
use core::sync::atomic::{AtomicU64, Ordering};

use super::mmio::{alloc_zeroed_boot, read32, read64, write32, write64};
use super::regs::{dsb_sy, isb, mpidr_to_hw_id, mrs, msr};
use crate::log;
use crate::mm::{self, PhysAddr};

pub const SGI_RESCHEDULE: u32 = 1;
pub const SGI_HALT: u32 = 2;
pub const SGI_TLB_FLUSH: u32 = 3;
/// Prvni LPI (zbytek rozsahu je v `DEVICE_IRQ_LAST`).
pub const LPI_FIRST: u32 = 8192;
/// Pocet bitu cisla preruseni pro LPI (cisla < 2^14: 8192 LPI).
pub const LPI_ID_BITS: u32 = 14;

/// Vychozi priorita (stredni; nizsi cislo = vyssi priorita).
const PRIORITY: u8 = 0xA0;

// Distributor (GICD).
const GICD_CTLR: u64 = 0x0000;
const GICD_CTLR_RWP: u32 = 1 << 31;
const GICD_CTLR_ARE: u32 = 1 << 4;
const GICD_CTLR_GRP1: u32 = 1 << 1;
const GICD_CTLR_GRP0: u32 = 1 << 0;

// Redistributor (GICR): ramec RD_base a za nim SGI_base (+64 KiB).
const GICR_CTLR: u64 = 0x0000;
const GICR_CTLR_ENABLE_LPIS: u32 = 1 << 0;
const GICR_TYPER: u64 = 0x0008;
const GICR_TYPER_LAST: u64 = 1 << 4;
const GICR_TYPER_VLPIS: u64 = 1 << 1;
const GICR_WAKER: u64 = 0x0014;
const GICR_WAKER_SLEEP: u32 = 1 << 1;
const GICR_WAKER_CHILDREN_ASLEEP: u32 = 1 << 2;
const GICR_PROPBASER: u64 = 0x0070;
const GICR_PENDBASER: u64 = 0x0078;
const SGI_BASE: u64 = 0x1_0000;
const GICR_IGROUPR0: u64 = SGI_BASE + 0x0080;
const GICR_ISENABLER0: u64 = SGI_BASE + 0x0100;
const GICR_IPRIORITYR: u64 = SGI_BASE + 0x0400;

/// Atributy cache pro tabulky LPI: Write-Back (7) a sdileni inner (1).
const TABLE_ATTRS: u64 = (7 << 7) | (1 << 10);

static GICD: AtomicU64 = AtomicU64::new(0);
/// Oblasti redistributoru (virtualni adresa, delka) z MADT.
static GICR_REGIONS: spin::Once<Vec<(u64, u64)>> = spin::Once::new();
/// Tabulka konfigurace LPI (jedna pro vsechna CPU), fyzicka adresa.
static PROP_TABLE: AtomicU64 = AtomicU64::new(0);

/// Velikost tabulky konfigurace LPI v bajtech (bajt na LPI).
fn prop_table_len() -> u64 {
    (1u64 << LPI_ID_BITS) - LPI_FIRST as u64
}

/// Zapne distributor a pripravi spolecne casti (konfigurace LPI). Volat
/// jednou na bootovacim CPU.
///
/// `regions` = oblasti redistributoru (fyzicka adresa, delka).
pub fn init_distributor(gicd_phys: u64, regions: &[(u64, u64)]) {
    let gicd = mm::map_mmio(PhysAddr::new(gicd_phys), 0x1_0000).as_u64();
    GICD.store(gicd, Ordering::Release);
    let regions = regions
        .iter()
        .map(|&(phys, len)| (mm::map_mmio(PhysAddr::new(phys), len).as_u64(), len))
        .collect();
    GICR_REGIONS.call_once(|| regions);

    // Jedna bezpecnostni domena (QEMU secure=off): Grp0 i Grp1, smerovani
    // podle afinity (ARE).
    write32(gicd + GICD_CTLR, 0);
    wait_rwp(gicd);
    write32(
        gicd + GICD_CTLR,
        GICD_CTLR_ARE | GICD_CTLR_GRP1 | GICD_CTLR_GRP0,
    );
    wait_rwp(gicd);

    // Konfigurace LPI: priorita, zatim vse vypnute (zapina `its`).
    let len = prop_table_len();
    let table = alloc_zeroed_boot(len, "tabulku konfigurace LPI");
    // SAFETY: prave alokovana tabulka v prime mape.
    unsafe {
        table
            .to_virt()
            .as_mut_ptr::<u8>()
            .write_bytes(PRIORITY, len as usize)
    };
    super::regs::dcache_clean_invalidate(table.to_virt().as_u64(), len);
    PROP_TABLE.store(table.as_u64(), Ordering::Release);
}

fn wait_rwp(gicd: u64) {
    let mut spins = 0u32;
    while read32(gicd + GICD_CTLR) & GICD_CTLR_RWP != 0 && spins < 10_000_000 {
        core::hint::spin_loop();
        spins += 1;
    }
}

/// Najde redistributor aktualniho CPU (podle afinity v GICR_TYPER).
fn own_redistributor() -> Option<u64> {
    let affinity = mpidr_to_hw_id(mrs!("mpidr_el1")) as u64;
    for &(base, len) in GICR_REGIONS.get()? {
        let mut rd = base;
        while rd < base + len {
            let typer = read64(rd + GICR_TYPER);
            if typer >> 32 == affinity {
                return Some(rd);
            }
            if typer & GICR_TYPER_LAST != 0 {
                break;
            }
            // GICv4 (virtualni LPI) ma ramce navic.
            rd += if typer & GICR_TYPER_VLPIS != 0 {
                0x4_0000
            } else {
                0x2_0000
            };
        }
    }
    None
}

/// Redistributor aktualniho CPU (cache v per-CPU nemame: hleda se jen pri
/// startu CPU a pri zapnuti LPI).
pub fn redistributor() -> u64 {
    own_redistributor().expect("redistributor GICv3 pro toto CPU nenalezen")
}

/// Inicializace na kazdem CPU: probuzeni redistributoru, SGI a casovac,
/// LPI, rozhrani CPU. Volat se zakazanymi prerusenimi, po `init_distributor`.
pub fn init_cpu(timer_intid: u32) {
    let rd = redistributor();
    let waker = read32(rd + GICR_WAKER);
    write32(rd + GICR_WAKER, waker & !GICR_WAKER_SLEEP);
    let mut spins = 0u32;
    while read32(rd + GICR_WAKER) & GICR_WAKER_CHILDREN_ASLEEP != 0 && spins < 10_000_000 {
        core::hint::spin_loop();
        spins += 1;
    }

    // SGI a PPI: skupina 1, priorita, zapnout IPI jadra a casovac.
    write32(rd + GICR_IGROUPR0, u32::MAX);
    for i in 0..8 {
        write32(
            rd + GICR_IPRIORITYR + i * 4,
            u32::from_ne_bytes([PRIORITY; 4]),
        );
    }
    write32(
        rd + GICR_ISENABLER0,
        (1 << SGI_RESCHEDULE) | (1 << SGI_HALT) | (1 << SGI_TLB_FLUSH) | (1 << timer_intid),
    );

    enable_lpis(rd);

    // Rozhrani CPU: systemove registry, maska priorit, skupina 1.
    // SAFETY: ICC_* registry GICv3 tohoto CPU; preruseni jsou zakazana.
    unsafe {
        msr!("S3_0_C12_C12_5", mrs!("S3_0_C12_C12_5") | 1); // ICC_SRE_EL1.SRE
        isb();
        msr!("S3_0_C4_C6_0", 0xFF); // ICC_PMR_EL1: vse projde
        msr!("S3_0_C12_C12_3", 0); // ICC_BPR1_EL1
        msr!("S3_0_C12_C12_4", 0); // ICC_CTLR_EL1: EOImode 0
        msr!("S3_0_C12_C12_7", 1); // ICC_IGRPEN1_EL1
    }
    isb();
}

/// Tabulky LPI pro redistributor a zapnuti LPI (jednou na CPU).
fn enable_lpis(rd: u64) {
    let prop = PROP_TABLE.load(Ordering::Acquire);
    if read32(rd + GICR_CTLR) & GICR_CTLR_ENABLE_LPIS != 0 {
        log::warn!("LPI uz byla zapnuta firmwarem, tabulky nelze zmenit");
        return;
    }
    write64(
        rd + GICR_PROPBASER,
        prop | TABLE_ATTRS | (LPI_ID_BITS as u64 - 1),
    );
    // Tabulka cekajicich LPI: bit na preruseni (vcetne 0..8192), zarovnani
    // 64 KiB, vynulovana.
    let pend_len = (1u64 << LPI_ID_BITS) / 8;
    let pend = alloc_zeroed_boot(pend_len.max(0x1_0000), "tabulku cekajicich LPI");
    write64(rd + GICR_PENDBASER, pend.as_u64() | TABLE_ATTRS);
    dsb_sy();
    write32(
        rd + GICR_CTLR,
        read32(rd + GICR_CTLR) | GICR_CTLR_ENABLE_LPIS,
    );
    dsb_sy();
}

/// Zapne nebo vypne LPI `intid` v tabulce konfigurace (pak je potreba
/// prikaz INV pres ITS).
pub fn set_lpi_enabled(intid: u32, enabled: bool) {
    let prop = PhysAddr::new(PROP_TABLE.load(Ordering::Acquire));
    let byte = prop.to_virt().as_u64() + (intid - LPI_FIRST) as u64;
    let value = PRIORITY | enabled as u8;
    // SAFETY: bajt tabulky konfigurace LPI v prime mape (index < delka).
    unsafe { (byte as *mut u8).write_volatile(value) };
    super::regs::dcache_clean_invalidate(byte, 1);
}

/// Potvrdi preruseni (ICC_IAR1_EL1); `None` = spurious.
pub fn acknowledge() -> Option<u32> {
    let id = (mrs!("S3_0_C12_C12_0") & 0xFF_FFFF) as u32;
    (!(1020..=1023).contains(&id)).then_some(id)
}

/// Prvni a posledni SPI (pevne linky zarizeni na desce, napr. GED ACPI).
pub const SPI_FIRST: u32 = 32;
pub const SPI_LAST: u32 = 1019;

const GICD_TYPER: u64 = 0x0004;
const GICD_IGROUPR: u64 = 0x0080;
const GICD_ISENABLER: u64 = 0x0100;
const GICD_ICENABLER: u64 = 0x0180;
const GICD_ICPENDR: u64 = 0x0280;
const GICD_IPRIORITYR: u64 = 0x0400;
const GICD_ICFGR: u64 = 0x0C00;
const GICD_IROUTER: u64 = 0x6000;

/// Nejvyssi SPI, ktere distributor ma (GICD_TYPER.ITLinesNumber).
pub fn spi_last_supported() -> u32 {
    let gicd = GICD.load(Ordering::Acquire);
    if gicd == 0 {
        return 0;
    }
    let lines = read32(gicd + GICD_TYPER) & 0x1F;
    (32 * (lines + 1) - 1).min(SPI_LAST)
}

/// Nastavi SPI `intid`: skupina 1, priorita, spousteni (hrana/uroven),
/// smerovani na CPU s afinitou `hw_id`. Zustane vypnute (`set_spi_enabled`).
pub fn configure_spi(intid: u32, edge: bool, hw_id: u32) {
    let gicd = GICD.load(Ordering::Acquire);
    let (word, bit) = ((intid / 32) as u64 * 4, 1u32 << (intid % 32));
    write32(gicd + GICD_ICENABLER + word, bit);
    wait_rwp(gicd);
    write32(gicd + GICD_IGROUPR + word, read32(gicd + GICD_IGROUPR + word) | bit);
    // Priorita: bajtovy registr (GICD_IPRIORITYR je bajtove adresovatelny).
    let prio = gicd + GICD_IPRIORITYR + intid as u64;
    // SAFETY: registr distributoru v namapovanem okne (intid <= 1019).
    unsafe { (prio as *mut u8).write_volatile(PRIORITY) };
    let cfg = gicd + GICD_ICFGR + (intid / 16) as u64 * 4;
    let shift = (intid % 16) * 2 + 1;
    let v = read32(cfg) & !(1 << shift);
    write32(cfg, v | ((edge as u32) << shift));
    let aff = hw_id as u64;
    let route = (aff >> 24 & 0xFF) << 32 | (aff >> 16 & 0xFF) << 16 | (aff >> 8 & 0xFF) << 8 | (aff & 0xFF);
    write64(gicd + GICD_IROUTER + intid as u64 * 8, route);
    write32(gicd + GICD_ICPENDR + word, bit);
    dsb_sy();
}

/// Zapne / vypne SPI v distributoru.
pub fn set_spi_enabled(intid: u32, enabled: bool) {
    let gicd = GICD.load(Ordering::Acquire);
    let (word, bit) = ((intid / 32) as u64 * 4, 1u32 << (intid % 32));
    write32(gicd + if enabled { GICD_ISENABLER } else { GICD_ICENABLER } + word, bit);
    if !enabled {
        wait_rwp(gicd);
    }
}

/// Konec obsluhy (ICC_EOIR1_EL1): snizeni priority i deaktivace.
pub fn end_of_interrupt(intid: u32) {
    // SAFETY: zapis EOIR pro prave potvrzene preruseni.
    unsafe { msr!("S3_0_C12_C12_1", intid as u64) };
    isb();
}

/// Posle SGI `sgi` na CPU s hardwarovym ID `hw_id` (ICC_SGI1R_EL1).
pub fn send_sgi(hw_id: u32, sgi: u32) {
    let aff0 = (hw_id & 0xFF) as u64;
    let aff1 = ((hw_id >> 8) & 0xFF) as u64;
    let aff2 = ((hw_id >> 16) & 0xFF) as u64;
    let aff3 = ((hw_id >> 24) & 0xFF) as u64;
    let value = (aff3 << 48)
        | (aff2 << 32)
        | ((aff0 / 16) << 44)
        | ((sgi as u64) << 24)
        | (aff1 << 16)
        | (1 << (aff0 % 16));
    // Zapisy do pameti (napr. pozadavek pro cil) musi byt videt drive.
    dsb_sy();
    // SAFETY: odeslani SGI nema jine ucinky nez preruseni cile.
    unsafe { msr!("S3_0_C12_C11_5", value) };
    isb();
}
