//! IOMMU ARM SMMUv3: preklad DMA zarizeni (obdoba VT-d a AMD-Vi na x86).
//!
//! Pravidla jsou stejna jako na x86 (viz `arch/x86_64/iommu`): po zapnuti
//! nema zadne zarizeni domenu a veskere DMA je zakazane; domenu dostane az
//! omezeny Resource s identitou zarizeni a mapuje se do ni jen DMA pamet
//! z nej, identicky (adresa pro zarizeni = fyzicka adresa).
//!
//! Struktury v pameti:
//! - tabulka streamu (dvouurovnova): StreamID (u PCI = RID podle IORT) ->
//!   STE. Neplatna polozka prvni urovne i STE s Config = abort DMA zablokuji,
//! - STE domeny ukazuje na kontextovy deskriptor (CD) s tabulkami stranek
//!   ve formatu stage 1 (stejny format jako tabulky CPU, `mmu::PageTable`)
//!   a vlastnim ASID,
//! - fronta prikazu (CFGI_STE, TLBI_NH_ASID, SYNC) a fronta udalosti
//!   (chyby DMA; cte se pri operacich s domenami a loguje).
//!
//! MSI: zprava zarizeni (zapis do GITS_TRANSLATER) prochazi take SMMU, proto
//! se stranka ITS mapuje do domeny zarizeni (`map_msi_doorbell`).

use alloc::sync::Arc;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicU16, Ordering};

use super::mmio::{read32, write32, write64};
use super::mmu::PageTable;
use super::regs::{dcache_clean_invalidate, dsb_sy};
use crate::arch::api::IommuDomain;
use crate::arch::api::paging::{MapFlags, PageSize, PageTableOps};
use crate::error::Error;
use crate::log;
use crate::mm::{self, PhysAddr, VirtAddr};
use crate::sync::SpinLock;

const IDR0: u64 = 0x00;
const IDR1: u64 = 0x04;
const IDR5: u64 = 0x14;
const CR0: u64 = 0x20;
const CR0ACK: u64 = 0x24;
const CR1: u64 = 0x28;
const CR2: u64 = 0x2C;
const GERROR: u64 = 0x60;
const GERRORN: u64 = 0x64;
/// GERROR/GERRORN: chyba fronty prikazu.
const GERROR_CMDQ_ERR: u32 = 1 << 0;
const STRTAB_BASE: u64 = 0x80;
const STRTAB_BASE_CFG: u64 = 0x88;
const CMDQ_BASE: u64 = 0x90;
const CMDQ_PROD: u64 = 0x98;
const CMDQ_CONS: u64 = 0x9C;
const EVENTQ_BASE: u64 = 0xA0;
const EVENTQ_PROD: u64 = 0x1_00A8;
const EVENTQ_CONS: u64 = 0x1_00AC;

const CR0_SMMUEN: u32 = 1 << 0;
const CR0_EVENTQEN: u32 = 1 << 2;
const CR0_CMDQEN: u32 = 1 << 3;

/// Velikosti front (log2 poctu polozek).
const CMDQ_LOG2: u32 = 8;
const EVENTQ_LOG2: u32 = 7;
/// Dvouurovnova tabulka streamu: druha uroven pokryva 2^8 StreamID.
const STRTAB_SPLIT: u32 = 8;
const STE_SIZE: u64 = 64;

/// Atributy cache front a tabulek (Write-Back, inner shareable), CR1.
/// Pro SMMU bez koherence s cache (IDR0.COHACC = 0) se pouzije 0: vse bez
/// cache a jadro po kazdem zapisu struktur zapise cache do pameti.
const CR1_WB_ISH: u32 = 1 | (1 << 2) | (3 << 4) | (1 << 6) | (1 << 8) | (3 << 10);

struct Smmu {
    regs: u64,
    /// Fyzicke adresy: prvni uroven tabulky streamu, fronty.
    l1: u64,
    l1_entries: u64,
    cmdq: u64,
    cmdq_prod: u32,
    eventq: u64,
    /// Druhe urovne tabulky streamu podle indexu prvni urovne.
    l2: Vec<Option<u64>>,
    /// Domeny podle StreamID.
    domains: Vec<(u32, Arc<SmmuDomain>)>,
}

static SMMU: SpinLock<Option<Smmu>> = SpinLock::new(None);
static ACTIVE: AtomicBool = AtomicBool::new(false);
/// Pristupy SMMU k tabulkam jsou koherentni s cache CPU (IDR0.COHACC).
static COHERENT: AtomicBool = AtomicBool::new(true);
/// Nejvetsi ASID (IDR0.ASID16: 16, jinak 8 bitu).
static MAX_ASID: AtomicU16 = AtomicU16::new(255);
static NEXT_ASID: AtomicU16 = AtomicU16::new(1);
/// Posun StreamID proti RID (z IORT: vystupni baze - vstupni baze).
static SID_BASE: SpinLock<(u32, u32)> = SpinLock::new((0, 0));

/// Vynulovana pamet pro struktury SMMU (fyzicka adresa).
fn alloc(bytes: u64) -> u64 {
    super::mmio::alloc_zeroed_boot(bytes, "tabulky SMMU").as_u64()
}

/// Vynulovana pamet pro struktury SMMU za behu (nova domena).
fn try_alloc(bytes: u64) -> Result<u64, Error> {
    super::mmio::alloc_zeroed(bytes)
        .map(|p| p.as_u64())
        .ok_or(Error::NoMemory)
}

/// Zapise slova do pameti struktury SMMU (pres primou mapu) a zapise cache.
fn write_words(phys: u64, words: &[u64]) {
    let virt = PhysAddr::new(phys).to_virt().as_u64();
    for (i, w) in words.iter().enumerate() {
        // SAFETY: struktura SMMU (STE, CD, prikaz) v prime mape.
        unsafe { ((virt + i as u64 * 8) as *mut u64).write_volatile(*w) };
    }
    dcache_clean_invalidate(virt, words.len() as u64 * 8);
}

fn wait_ack(regs: u64, value: u32) -> bool {
    for _ in 0..10_000_000 {
        if read32(regs + CR0ACK) == value {
            return true;
        }
        core::hint::spin_loop();
    }
    false
}

/// Je SMMU zapnuta?
pub fn active() -> bool {
    ACTIVE.load(Ordering::Acquire)
}

/// Zapne SMMUv3 na `phys` (z IORT). `rid_to_sid` = (vstupni baze RID,
/// vystupni baze StreamID) mapovani korenoveho komplexu PCIe.
pub fn init(phys: u64, rid_to_sid: (u32, u32)) {
    let regs = mm::map_mmio(PhysAddr::new(phys), 0x2_0000).as_u64();
    let idr0 = read32(regs + IDR0);
    let idr1 = read32(regs + IDR1);
    let idr5 = read32(regs + IDR5);
    let s1p = idr0 & (1 << 1) != 0;
    let ttf_aarch64 = (idr0 >> 2) & 3 >= 2;
    let two_level = (idr0 >> 27) & 3 >= 1;
    let gran4k = idr5 & (1 << 4) != 0;
    if !(s1p && ttf_aarch64 && two_level && gran4k) {
        log::warn!(
            "SMMUv3 na {phys:#x} nema potrebne funkce (stage 1, AArch64 tabulky 4 KiB, dvouurovnova tabulka streamu), vypnuta"
        );
        return;
    }
    let sid_bits = (idr1 & 0x3F).min(16);
    *SID_BASE.lock() = rid_to_sid;
    let coherent = idr0 & (1 << 4) != 0;
    COHERENT.store(coherent, Ordering::Relaxed);
    MAX_ASID.store(
        if idr0 & (1 << 12) != 0 { u16::MAX } else { 255 },
        Ordering::Relaxed,
    );

    // Vypnout a pockat (firmware mohl nechat zapnute).
    write32(regs + CR0, 0);
    if !wait_ack(regs, 0) {
        log::error!("SMMUv3 se nevypnula, preklad DMA nezapnut");
        return;
    }
    write32(regs + CR1, if coherent { CR1_WB_ISH } else { 0 });
    // RECINVSID: nezname StreamID hlasit; PTM: TLB jen prikazy fronty.
    write32(regs + CR2, (1 << 1) | (1 << 2));

    // Tabulka streamu: prvni uroven (neplatne polozky = DMA zakazane).
    let l1_entries = 1u64 << sid_bits.saturating_sub(STRTAB_SPLIT);
    let l1 = alloc(l1_entries * 8);
    write64(regs + STRTAB_BASE, (1 << 62) | l1);
    // FMT = 2 urovne, SPLIT, LOG2SIZE = pocet bitu StreamID.
    write32(
        regs + STRTAB_BASE_CFG,
        (1 << 16) | (STRTAB_SPLIT << 6) | sid_bits,
    );

    let cmdq = alloc(16 << CMDQ_LOG2);
    write64(regs + CMDQ_BASE, (1 << 62) | cmdq | CMDQ_LOG2 as u64);
    write32(regs + CMDQ_PROD, 0);
    write32(regs + CMDQ_CONS, 0);
    let eventq = alloc(32 << EVENTQ_LOG2);
    write64(regs + EVENTQ_BASE, (1 << 62) | eventq | EVENTQ_LOG2 as u64);
    write32(regs + EVENTQ_PROD, 0);
    write32(regs + EVENTQ_CONS, 0);
    dsb_sy();

    write32(regs + CR0, CR0_CMDQEN | CR0_EVENTQEN);
    if !wait_ack(regs, CR0_CMDQEN | CR0_EVENTQEN) {
        log::error!("SMMUv3: fronty se nezapnuly, preklad DMA nezapnut");
        return;
    }
    let mut smmu = Smmu {
        regs,
        l1,
        l1_entries,
        cmdq,
        cmdq_prod: 0,
        eventq,
        l2: alloc::vec![None; l1_entries as usize],
        domains: Vec::new(),
    };
    smmu.command([0x04, 31]); // CFGI_ALL
    smmu.command([0x30, 0]); // TLBI_NSNH_ALL
    smmu.sync();
    write32(regs + CR0, CR0_CMDQEN | CR0_EVENTQEN | CR0_SMMUEN);
    if !wait_ack(regs, CR0_CMDQEN | CR0_EVENTQEN | CR0_SMMUEN) {
        log::error!("SMMUv3 se nezapnula");
        return;
    }
    *SMMU.lock() = Some(smmu);
    ACTIVE.store(true, Ordering::Release);
    log::info!("SMMUv3 na {phys:#x}: StreamID {sid_bits} bitu, preklad DMA zapnut");
}

impl Smmu {
    /// Zapise prikaz (2 slova) do fronty.
    fn command(&mut self, cmd: [u64; 2]) {
        let size = 1u32 << CMDQ_LOG2;
        let index_mask = 2 * size - 1;
        // Pockat na misto ve fronte (index s bitem obehu; CONS nese v bitech
        // 30:24 i kod posledni chyby, proto maskovat).
        let mut spins = 0u32;
        loop {
            self.recover_cmdq_error();
            let cons = read32(self.regs + CMDQ_CONS) & index_mask;
            let used = self.cmdq_prod.wrapping_sub(cons) & index_mask;
            if used < size {
                break;
            }
            spins += 1;
            if spins > 10_000_000 {
                // Fronta stoji: prikaz radeji zahodit nez prepsat
                // nezpracovane prikazy.
                log::error!(
                    "SMMUv3: fronta prikazu je plna, prikaz {:#x} zahozen",
                    cmd[0] & 0xFF
                );
                return;
            }
            core::hint::spin_loop();
        }
        let index = (self.cmdq_prod & (size - 1)) as u64;
        write_words(self.cmdq + index * 16, &cmd);
        self.cmdq_prod = (self.cmdq_prod + 1) & (2 * size - 1);
        dsb_sy();
        write32(self.regs + CMDQ_PROD, self.cmdq_prod);
    }

    /// CMD_SYNC a cekani, az SMMU zpracuje vsechny prikazy.
    fn sync(&mut self) {
        self.command([0x46, 0]);
        for _ in 0..10_000_000 {
            self.recover_cmdq_error();
            let cons = read32(self.regs + CMDQ_CONS);
            if cons & ((2 << CMDQ_LOG2) - 1) == self.cmdq_prod {
                return;
            }
            core::hint::spin_loop();
        }
        log::error!("SMMUv3: CMD_SYNC nedobehl");
    }

    /// Kdyz SMMU hlasi chybu prikazu (GERROR.CMDQ_ERR != GERRORN), fronta
    /// stoji na chybnem prikazu: nahradit ho prikazem SYNC (nic nedela),
    /// chybu potvrdit (prepnout GERRORN.CMDQ_ERR) a SMMU pokracuje.
    fn recover_cmdq_error(&mut self) {
        let gerror = read32(self.regs + GERROR);
        let gerrorn = read32(self.regs + GERRORN);
        if (gerror ^ gerrorn) & GERROR_CMDQ_ERR == 0 {
            return;
        }
        let cons = read32(self.regs + CMDQ_CONS);
        let index = (cons & ((1 << CMDQ_LOG2) - 1)) as u64;
        log::error!(
            "SMMUv3: chyba prikazu na pozici {index} (kod {:#x}), prikaz preskocen",
            (cons >> 24) & 0x7F
        );
        write_words(self.cmdq + index * 16, &[0x46, 0]);
        dsb_sy();
        write32(self.regs + GERRORN, gerrorn ^ GERROR_CMDQ_ERR);
    }

    /// Fyzicka adresa STE pro `sid` (zalozi druhou uroven tabulky).
    fn ste(&mut self, sid: u32) -> Result<u64, Error> {
        let l1_index = (sid >> STRTAB_SPLIT) as usize;
        if l1_index as u64 >= self.l1_entries {
            return Err(Error::OutOfRange);
        }
        let l2 = match self.l2[l1_index] {
            Some(l2) => l2,
            None => {
                // Vsechny STE nove druhe urovne: platne, Config = abort.
                let l2 = try_alloc(STE_SIZE << STRTAB_SPLIT)?;
                for i in 0..(1u64 << STRTAB_SPLIT) {
                    write_words(l2 + i * STE_SIZE, &[1, 0]);
                }
                // L1: SPAN = SPLIT + 1, adresa druhe urovne.
                write_words(
                    self.l1 + l1_index as u64 * 8,
                    &[l2 | (STRTAB_SPLIT as u64 + 1)],
                );
                self.l2[l1_index] = Some(l2);
                l2
            }
        };
        Ok(l2 + (sid as u64 & ((1 << STRTAB_SPLIT) - 1)) * STE_SIZE)
    }

    /// Vypise udalosti (chyby DMA) z fronty udalosti do logu.
    fn drain_events(&mut self) {
        let size = 1u32 << EVENTQ_LOG2;
        loop {
            let prod = read32(self.regs + EVENTQ_PROD) & (2 * size - 1);
            let cons = read32(self.regs + EVENTQ_CONS) & (2 * size - 1);
            if prod == cons {
                return;
            }
            let rec = PhysAddr::new(self.eventq + (cons & (size - 1)) as u64 * 32)
                .to_virt()
                .as_u64();
            dcache_clean_invalidate(rec, 32);
            // SAFETY: zaznam fronty udalosti (32 B) v prime mape.
            let (w0, w2) = unsafe {
                (
                    (rec as *const u64).read_volatile(),
                    ((rec + 16) as *const u64).read_volatile(),
                )
            };
            log::warn!(
                "SMMU: zablokovany pristup DMA zarizeni (StreamID {:#x}) na adresu {w2:#x} (udalost {:#x})",
                w0 >> 32,
                w0 & 0xFF
            );
            write32(self.regs + EVENTQ_CONS, (cons + 1) & (2 * size - 1));
        }
    }
}

/// Domena prekladu DMA jednoho zarizeni.
pub struct SmmuDomain {
    asid: u16,
    sid: u32,
    table: SpinLock<PageTable>,
}

/// Domena zarizeni (segment, sbernice, zarizeni << 3 | funkce): existujici,
/// nebo nova. `Ok(None)` = SMMU neni (DMA neomezene).
pub fn domain(segment: u16, bus: u8, devfn: u8) -> Result<Option<Arc<SmmuDomain>>, Error> {
    if !active() {
        return Ok(None);
    }
    if segment != 0 {
        return Err(Error::NotSupported);
    }
    let rid = (bus as u32) << 8 | devfn as u32;
    let (in_base, out_base) = *SID_BASE.lock();
    let sid = rid.wrapping_sub(in_base).wrapping_add(out_base);
    let mut guard = SMMU.lock();
    let smmu = guard.as_mut().ok_or(Error::NotSupported)?;
    smmu.drain_events();
    if let Some((_, d)) = smmu.domains.iter().find(|(s, _)| *s == sid) {
        return Ok(Some(d.clone()));
    }
    let ste = smmu.ste(sid)?;
    // ASID: kazda domena vlastni a domeny se neruseji (zarizeni drzi
    // domenu po celou dobu behu), takze staci citac do maxima SMMU.
    let asid = NEXT_ASID.load(Ordering::Relaxed);
    if asid == 0 || asid > MAX_ASID.load(Ordering::Relaxed) {
        log::error!(
            "SMMU: dosla cisla ASID, zarizeni {bus:02x}:{:02x}.{} nedostane domenu",
            devfn >> 3,
            devfn & 7
        );
        return Err(Error::NoResources);
    }
    // Pamet nejdriv, ASID az po uspechu: pri nedostatku pameti se nic
    // neztrati (ani cislo ASID, ani tabulky).
    let coherent = COHERENT.load(Ordering::Relaxed);
    let table = PageTable::new_for_device(!coherent).map_err(|_| Error::NoMemory)?;
    let cd = match try_alloc(64) {
        Ok(cd) => cd,
        Err(e) => {
            // SAFETY: tabulky jsou prave vytvorene a nikde aktivni.
            unsafe { table.destroy() };
            return Err(e);
        }
    };
    NEXT_ASID.store(asid.wrapping_add(1), Ordering::Relaxed);

    // Kontextovy deskriptor: stage 1, 48 bitu, granule 4 KiB, ASID domeny,
    // MAIR jako CPU. Pruchod tabulek s cache (WB, inner shareable), jen kdyz
    // je SMMU koherentni; jinak bez cache (jadro zapisuje tabulky do pameti).
    let walk_attrs = if coherent {
        (1 << 8) | (1 << 10) | (3 << 12) // IR0, OR0 WB, SH0 inner
    } else {
        0
    };
    let cd0 = 16 // T0SZ
        | walk_attrs
        | (16 << 16) | (2 << 22) // T1SZ, TG1 4 KiB (platne, i kdyz nepouzite)
        | (1 << 30) // EPD1: horni polovina nepouzita
        | (1 << 31) // V
        | (5 << 32) // IPS 48 bitu
        | (1 << 35) // AFFD: chybejici Access flag neni chyba
        | (1 << 41) // AA64
        // R (hlasit chyby), A (chybu ukoncit abortem), ASET; bez S (stall).
        | (1 << 45) | (1 << 46) | (1 << 47)
        | ((asid as u64) << 48);
    write_words(
        cd,
        &[
            cd0,
            table.root() & 0x000F_FFFF_FFFF_FFF0,
            0,
            boot_info::aarch64::MAIR,
            0,
            0,
            0,
            0,
        ],
    );
    // STE: platna, Config = stage 1, jeden CD; atributy pristupu k CD.
    let ste1 = if coherent {
        (1 << 2) | (1 << 4) | (3 << 6) // S1CIR, S1COR WB, S1CSH inner
    } else {
        0
    };
    write_words(ste + 8, &[ste1, 0, 0, 0, 0, 0, 0]);
    write_words(ste, &[1 | (0b101 << 1) | (cd & 0x000F_FFFF_FFFF_FFC0)]);
    // CFGI_STE bez Leaf: zneplatni i polozku prvni urovne (mohla vzniknout).
    smmu.command([0x03 | ((sid as u64) << 32), 0]);
    smmu.sync();

    let d = Arc::new(SmmuDomain {
        asid,
        sid,
        table: SpinLock::new(table),
    });
    smmu.domains.push((sid, d.clone()));
    log::debug!(
        "SMMU: domena zarizeni {bus:02x}:{:02x}.{} (StreamID {sid:#x}, ASID {asid})",
        devfn >> 3,
        devfn & 7
    );
    Ok(Some(d))
}

impl SmmuDomain {
    fn map_range(&self, phys: u64, pages: u64, flags: MapFlags) -> Result<(), Error> {
        let mut table = self.table.lock();
        for i in 0..pages {
            let a = phys + i * 4096;
            match table.map(VirtAddr::new(a), PhysAddr::new(a), PageSize::Size4K, flags) {
                Ok(()) => {}
                Err(crate::arch::api::paging::MapError::AlreadyMapped) => {}
                Err(_) => return Err(Error::NoMemory),
            }
        }
        Ok(())
    }

    fn invalidate(&self) {
        if let Some(smmu) = SMMU.lock().as_mut() {
            smmu.command([0x11 | ((self.asid as u64) << 48), 0]); // TLBI_NH_ASID
            smmu.sync();
            smmu.drain_events();
        }
    }
}

impl IommuDomain for Arc<SmmuDomain> {
    fn map(&self, phys: u64, pages: u64) -> Result<(), Error> {
        // USER: pristup povolen bez ohledu na privilegovanost transakce.
        self.map_range(phys, pages, MapFlags::WRITE | MapFlags::USER)
    }

    fn unmap(&self, phys: u64, pages: u64) {
        {
            let mut table = self.table.lock();
            for i in 0..pages {
                table.unmap_4k(VirtAddr::new(phys + i * 4096));
            }
        }
        self.invalidate();
    }
}

/// Namapuje stranku ITS (doorbell MSI) do domeny zarizeni, aby jeho
/// zpravy MSI prosly SMMU. Bez domeny (nebo bez SMMU) nic nedela.
pub fn map_msi_doorbell(segment: u16, bus: u8, devfn: u8, doorbell: u64) {
    if !active() {
        return;
    }
    let rid = (bus as u32) << 8 | devfn as u32;
    let (in_base, out_base) = *SID_BASE.lock();
    let sid = rid.wrapping_sub(in_base).wrapping_add(out_base);
    let domain = SMMU.lock().as_ref().and_then(|s| {
        s.domains
            .iter()
            .find(|(s, _)| *s == sid)
            .map(|(_, d)| d.clone())
    });
    if segment != 0 {
        return;
    }
    if let Some(d) = domain {
        // Registr zarizeni: bez cache (Device), jen zapis zarizenim.
        if d.map_range(
            doorbell & !0xFFF,
            1,
            MapFlags::WRITE | MapFlags::USER | MapFlags::UNCACHED,
        )
        .is_err()
        {
            log::error!(
                "SMMU: stranku MSI nejde namapovat do domeny StreamID {:#x}",
                d.sid
            );
        }
    }
}
