//! DMA po kusech ze strany procesu (`VMO_PINNED`, `DMA_MAP`, `DMA_SEGMENTS`,
//! `DMA_SYNC`): rozpocet pripnute pameti, prava handlu (`DMA`, smer),
//! meze a mapovani, ktere nejde predat. Prostredek je pro neexistujici
//! zarizeni 00:ff:1f.0: s IOMMU dostane domenu (jeden usek IOVA, napoveda),
//! bez ni (`--iommu none`) fyzicke stranky.
//! Skutecny prenos zarizeni a zablokovany pristup po odmapovani: sada
//! `ovladace z manifestu` (edu `sg`, `stale`).

use alloc::format;
use alloc::string::String;

use libos::dma::{DmaMap, map, sync};
use libos::hw::Resource;
use libos::{Channel, Error, Rights, Startup, Vmo};

use super::{Test, TestResult, fail};
use crate::{check, check_eq};

pub const TESTS: &[Test] = &[
    ("pripnuta pamet: rozpocet DMA, bez pecete", pinned_budget),
    ("DMA_MAP: prava DMA a smeru, pripnuti, meze", map_rights),
    ("DMA_MAP: useky, DMA_SYNC, REQUIRE_IOMMU, kvota a predani", map_segments),
];

const PAGE: u64 = 4096;

/// Prostredek pro neexistujici zarizeni (jako ovladac od devmgr).
fn device_resource(s: &Startup, dma_max: u64) -> Result<Resource, String> {
    let root = s.root_resource.as_ref().ok_or("chybi korenovy Resource")?;
    root.restrict_device(&[], 0, dma_max, (0, 0xFF, 0x1F, 0))
        .map_err(|e| fail("RESOURCE_CREATE pro zarizeni", e))
}

fn pinned_budget(_s: &Startup) -> TestResult {
    let before = libos::job::current_info().map_err(|e| fail("JOB_INFO", e))?;
    check!(before.dma.limit != abi_unlimited(), "pripnuta pamet nema strop");
    let vmo = Vmo::create_pinned(3 * PAGE).map_err(|e| fail("VMO_CREATE PINNED", e))?;
    let after = libos::job::current_info().map_err(|e| fail("JOB_INFO", e))?;
    check_eq!(after.dma.used - before.dma.used, 3 * PAGE, "strzeno z rozpoctu DMA");
    check!(after.user.used - before.user.used >= 3 * PAGE, "a z uzivatelske pameti");
    check_eq!(vmo.seal().err(), Some(Error::NotSupported), "pripnutou pamet nejde zapecetit");
    drop(vmo);
    let end = libos::job::current_info().map_err(|e| fail("JOB_INFO", e))?;
    check_eq!(end.dma.used, before.dma.used, "zanik vratil rozpocet");
    // Neznama volba VMO_CREATE.
    check_eq!(
        libos::sys::vmo_create_with(PAGE, 0, 1 << 5).err(),
        Some(Error::InvalidArgs),
        "neznama volba"
    );
    Ok(())
}

fn abi_unlimited() -> u64 {
    libos::abi::job::UNLIMITED
}

fn map_rights(s: &Startup) -> TestResult {
    let res = device_resource(s, 1 << 20)?;
    let vmo = Vmo::create_pinned(2 * PAGE).map_err(|e| fail("VMO_CREATE PINNED", e))?;
    let rw = map::DEVICE_READ | map::DEVICE_WRITE;
    // Handle bez DMA (odesilatel bufferu zarizeni nedovolil).
    let no_dma = Vmo::from_handle(
        vmo.handle()
            .duplicate(Some(Rights::BASIC | Rights::READ | Rights::WRITE | Rights::MAP))
            .map_err(|e| fail("duplikat", e))?,
    );
    check_eq!(DmaMap::new(&res, &no_dma, 0, PAGE, rw).err(), Some(Error::AccessDenied), "bez prava DMA");
    // Jen READ + DMA: zarizeni smi cist, zapisovat ne.
    let read_only = Vmo::from_handle(
        vmo.handle()
            .duplicate(Some(Rights::BASIC | Rights::READ | Rights::DMA))
            .map_err(|e| fail("duplikat", e))?,
    );
    check!(DmaMap::new(&res, &read_only, 0, PAGE, map::DEVICE_READ).is_ok(), "cteni zarizenim s READ");
    check_eq!(
        DmaMap::new(&res, &read_only, 0, PAGE, map::DEVICE_WRITE).err(),
        Some(Error::AccessDenied),
        "zapis zarizenim bez WRITE"
    );
    // Prostredek bez identity zarizeni (korenovy) nemapuje.
    let root = s.root_resource.as_ref().ok_or("chybi korenovy Resource")?;
    check_eq!(DmaMap::new(root, &vmo, 0, PAGE, rw).err(), Some(Error::AccessDenied), "bez zarizeni");
    // Nepripnuta pamet.
    let plain = Vmo::create(PAGE).map_err(|e| fail("VMO_CREATE", e))?;
    check_eq!(DmaMap::new(&res, &plain, 0, PAGE, rw).err(), Some(Error::BadState), "nepripnuta pamet");
    // Meze a priznaky.
    check_eq!(DmaMap::new(&res, &vmo, 1, PAGE, rw).err(), Some(Error::InvalidArgs), "nezarovnany offset");
    check_eq!(DmaMap::new(&res, &vmo, PAGE, 2 * PAGE, rw).err(), Some(Error::InvalidArgs), "za koncem");
    check_eq!(DmaMap::new(&res, &vmo, 0, PAGE, 0).err(), Some(Error::InvalidArgs), "bez smeru");
    check_eq!(DmaMap::new(&res, &vmo, 0, PAGE, 1 << 7).err(), Some(Error::InvalidArgs), "neznamy priznak");
    Ok(())
}

fn map_segments(s: &Startup) -> TestResult {
    let res = device_resource(s, 4 * PAGE)?;
    let vmo = Vmo::create_pinned(4 * PAGE).map_err(|e| fail("VMO_CREATE PINNED", e))?;
    let rw = map::DEVICE_READ | map::DEVICE_WRITE;
    // S IOMMU projde REQUIRE_IOMMU, bez ni NOT_SUPPORTED.
    let iommu = match DmaMap::new(&res, &vmo, 0, PAGE, rw | map::REQUIRE_IOMMU) {
        Ok(m) => {
            let iova = m.device_addr().map_err(|e| fail("DMA_SEGMENTS", e))?;
            check!(iova >= 1 << 32, format!("IOVA {iova:#x} v okne nad 4 GiB"));
            // Napoveda: prekryv odmitnut, volne misto prijato.
            check_eq!(DmaMap::at(&res, &vmo, 0, PAGE, rw, iova).err(), Some(Error::AlreadyExists), "prekryv napovedy");
            let hint = iova + (64 << 20);
            let h = DmaMap::at(&res, &vmo, 0, PAGE, rw, hint).map_err(|e| fail("DMA_MAP s napovedou", e))?;
            check_eq!(h.device_addr().map_err(|e| fail("DMA_SEGMENTS", e))?, hint, "adresa z napovedy");
            true
        }
        Err(Error::NotSupported) => {
            check_eq!(DmaMap::at(&res, &vmo, 0, PAGE, rw, 1 << 40).err(), Some(Error::NotSupported), "napoveda bez IOMMU");
            false
        }
        Err(e) => return Err(fail("DMA_MAP REQUIRE_IOMMU", e)),
    };
    let m = DmaMap::new(&res, &vmo, PAGE, 3 * PAGE, rw).map_err(|e| fail("DMA_MAP", e))?;
    let segs = m.segments().map_err(|e| fail("DMA_SEGMENTS", e))?;
    if iommu {
        check_eq!(segs.len(), 1, "s IOMMU jeden usek");
    }
    let total: u64 = segs.iter().map(|s| s.len).sum();
    check_eq!(total, 3 * PAGE, "useky pokryvaji mapovani");
    check!(segs.iter().all(|s| s.addr % PAGE == 0 && s.len % PAGE == 0), "useky po strankach");
    check_eq!(m.device_addr().map_err(|e| fail("DMA_SEGMENTS", e))?, segs[0].addr, "prvni usek");
    // Prazdny vystup vrati jen pocet.
    check_eq!(libos::sys::dma_segments(m.handle().raw(), &mut []), Ok(segs.len()), "pocet bez vystupu");
    m.sync(0, 3 * PAGE, sync::FOR_DEVICE).map_err(|e| fail("DMA_SYNC", e))?;
    m.sync(100, 5000, sync::FOR_CPU).map_err(|e| fail("DMA_SYNC", e))?;
    check_eq!(m.sync(0, 3 * PAGE + 1, sync::FOR_CPU).err(), Some(Error::InvalidArgs), "sync za koncem");
    check_eq!(m.sync(0, PAGE, 9).err(), Some(Error::InvalidArgs), "neznamy smer");
    // Kvota prostredku (4 stranky): dalsi 2 se nevejdou, po odmapovani ano.
    check_eq!(DmaMap::new(&res, &vmo, 0, 2 * PAGE, rw).err(), Some(Error::NoResources), "kvota");
    // Mapovani patri ovladaci: nejde predat jinemu procesu.
    let (a, _b) = Channel::create().map_err(|e| fail("kanal", e))?;
    // Pres sys: neuspesny zapis handle nespotrebuje, zavre se tady (obal
    // Channel::write by ho zapomnel a mapovani by zilo dal).
    let dup = m.handle().duplicate(None).map_err(|e| fail("duplikat mapovani", e))?;
    let r = libos::sys::channel_write(a.handle().raw(), b"map", &[dup.raw()]);
    check_eq!(r.err(), Some(Error::AccessDenied), "predani mapovani");
    drop(dup);
    drop(m);
    check!(DmaMap::new(&res, &vmo, 0, 2 * PAGE, rw).is_ok(), "kvota vracena odmapovanim");
    Ok(())
}
