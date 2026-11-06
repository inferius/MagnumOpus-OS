//! Ovladace: spravce zarizeni a virtio-blk.

use alloc::vec;

use libos::{Error, Startup};

use super::{Test, TestResult, devices, fail};
use crate::{check, check_eq};

pub const TESTS: &[Test] = &[
    ("spravce zarizeni nasel disky", disks_found),
    ("systemovy disk: sektor 0 je ochranny MBR", boot_disk_mbr),
    ("testovaci disk: zapis a cteni", scratch_roundtrip),
    ("cteni za koncem disku -> OUT_OF_RANGE", read_past_end),
    ("omezeny Resource: rozsahy a kvoty", restricted_resource),
];

/// Testovaci disk ma presne 1 MiB (viz xtask).
const SCRATCH_SECTORS: u64 = 2048;

fn disks_found(_: &Startup) -> TestResult {
    let d = devices().ok_or("spravce zarizeni nebezi")?;
    check!(
        d.blocks.len() >= 2,
        "ocekavany alespon 2 disky (system, testovaci)"
    );
    Ok(())
}

fn boot_disk_mbr(_: &Startup) -> TestResult {
    let d = devices().ok_or("spravce zarizeni nebezi")?;
    let mut found = false;
    for b in d.blocks.iter().filter(|b| b.sectors() > SCRATCH_SECTORS) {
        let mut sector = vec![0u8; 512];
        b.read(0, &mut sector)
            .map_err(|e| fail("cteni sektoru 0", e))?;
        // Ochranny MBR disku GPT: podpis 0x55AA a oddil typu 0xEE.
        if sector[510] == 0x55 && sector[511] == 0xAA && sector[446 + 4] == 0xEE {
            found = true;
        }
    }
    check!(found, "zadny disk nema ochranny MBR (systemovy disk)");
    Ok(())
}

fn scratch() -> Result<&'static blockproto::BlockClient, alloc::string::String> {
    let d = devices().ok_or("spravce zarizeni nebezi")?;
    d.blocks
        .iter()
        .find(|b| b.sectors() == SCRATCH_SECTORS)
        .ok_or_else(|| "testovaci disk (1 MiB) nenalezen".into())
}

fn scratch_roundtrip(_: &Startup) -> TestResult {
    let b = scratch()?;
    // Vic nez jeden pozadavek (MAX_SECTORS), s prechodem pres hranici.
    let data: alloc::vec::Vec<u8> = (0..100 * 512).map(|i| (i * 7 + i / 512) as u8).collect();
    b.write(10, &data).map_err(|e| fail("zapis", e))?;
    b.flush().map_err(|e| fail("flush", e))?;
    let mut back = vec![0u8; data.len()];
    b.read(10, &mut back).map_err(|e| fail("cteni", e))?;
    check!(back == data, "prectena data se lisi od zapsanych");
    Ok(())
}

fn read_past_end(_: &Startup) -> TestResult {
    let b = scratch()?;
    let mut buf = vec![0u8; 1024];
    check_eq!(
        b.read(SCRATCH_SECTORS - 1, &mut buf),
        Err(Error::OutOfRange),
        "cteni pres konec"
    );
    Ok(())
}

fn restricted_resource(s: &Startup) -> TestResult {
    const PAGE: u64 = 4096;
    let root = s.root_resource.as_ref().ok_or("chybi korenovy Resource")?;
    let (ecam, _) = s.pci_ecam.first().ok_or("chybi okno ECAM")?;
    // Dve stranky konfiguracniho prostoru PCI (registry zarizeni, ne RAM).
    let base = root
        .phys_addr(ecam, 0)
        .map_err(|e| fail("fyzicka adresa ECAM", e))?;
    let child = root
        .restrict(&[(base, PAGE)], 1, PAGE)
        .map_err(|e| fail("vytvoreni omezeneho", e))?;
    child
        .device_memory(base, PAGE)
        .map_err(|e| fail("povoleny rozsah", e))?;
    check_eq!(
        child.device_memory(base + PAGE, PAGE).err(),
        Some(Error::AccessDenied),
        "rozsah mimo povolene"
    );
    check_eq!(
        child.restrict(&[(base, 2 * PAGE)], 1, PAGE).err(),
        Some(Error::AccessDenied),
        "rozsireni rozsahu potomkem"
    );
    check_eq!(
        child.restrict(&[(base, PAGE)], 2, PAGE).err(),
        Some(Error::AccessDenied),
        "zvyseni limitu preruseni potomkem"
    );
    let _irq = child.interrupt().map_err(|e| fail("prvni preruseni", e))?;
    check_eq!(
        child.interrupt().err(),
        Some(Error::NoResources),
        "druhe preruseni nad limit"
    );
    child
        .dma_memory(PAGE)
        .map_err(|e| fail("DMA v limitu", e))?;
    check_eq!(
        child.dma_memory(PAGE).err(),
        Some(Error::NoResources),
        "DMA nad limit"
    );
    // Vnoreny prostredek nesmi obejit kvoty rodice (pocitaji se v retezci).
    let grandchild = child
        .restrict(&[], 1, PAGE)
        .map_err(|e| fail("vnoreny prostredek", e))?;
    check_eq!(
        grandchild.interrupt().err(),
        Some(Error::NoResources),
        "preruseni pres vnoreny prostredek"
    );
    check_eq!(
        grandchild.dma_memory(PAGE).err(),
        Some(Error::NoResources),
        "DMA pres vnoreny prostredek"
    );
    check_eq!(
        root.restrict(&[(base + 16, PAGE)], 1, PAGE).err(),
        Some(Error::InvalidArgs),
        "nezarovnany rozsah"
    );
    check_eq!(
        libos::sys::vmo_create_contiguous(root.handle().raw(), u64::MAX).err(),
        Some(Error::InvalidArgs),
        "obri velikost DMA (nesmi shodit jadro)"
    );
    Ok(())
}
