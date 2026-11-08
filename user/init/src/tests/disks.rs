//! Ovladace disku NVMe a AHCI a ochrana cizich disku.
//!
//! Testovaci disky pripravi xtask (`xtask/src/testdisks.rs`): NVMe a AHCI
//! s nasim oddilem GPT (klient vidi jen oddil, prvni sektor oddilu nese
//! znacku) a cizi disk GPT na AHCI (jen pro cteni). Disky se poznaji podle
//! velikosti, kterou klient vidi.

use alloc::vec;
use alloc::vec::Vec;

use blockproto::{BlockClient, feature};
use libos::{Error, Startup};

use super::{Test, TestResult, devices, fail};
use crate::{check, check_eq};

pub const TESTS: &[Test] = &[
    ("NVMe: disk nalezen, vidi jen nas oddil", nvme_partition),
    ("NVMe: zapis a cteni", nvme_roundtrip),
    ("NVMe: flush", nvme_flush),
    ("NVMe: TRIM", nvme_trim),
    ("NVMe: cteni a zapis za koncem oddilu", nvme_bounds),
    ("NVMe: IOMMU chrani pamet mimo DMA", nvme_dma),
    ("AHCI: disk nalezen, vidi jen nas oddil", ahci_partition),
    ("AHCI: zapis a cteni", ahci_roundtrip),
    ("AHCI: flush", ahci_flush),
    ("AHCI: TRIM", ahci_trim),
    ("AHCI: cteni a zapis za koncem oddilu", ahci_bounds),
    ("AHCI: IOMMU chrani pamet mimo DMA", ahci_dma),
    ("cizi disk: jen pro cteni", foreign_read_only),
];

/// Velikosti, ktere klient vidi (musi sedet s xtask/src/testdisks.rs).
const NVME_SECTORS: u64 = 4096;
const AHCI_SECTORS: u64 = 6144;
const FOREIGN_SECTORS: u64 = 16384;
/// Znacky v prvnim sektoru nasich oddilu a v cizim oddilu (sektor 2048).
const NVME_MARK: &[u8] = b"MagnumOpus test NVMe";
const AHCI_MARK: &[u8] = b"MagnumOpus test AHCI";
const FOREIGN_MARK: &[u8] = b"cizi data - nezapisovat";
const FOREIGN_PART_START: u64 = 2048;

fn disk(sectors: u64, what: &str) -> Result<&'static BlockClient, alloc::string::String> {
    let d = devices().ok_or("spravce zarizeni nebezi")?;
    d.blocks
        .iter()
        .find(|b| b.sectors() == sectors)
        .ok_or_else(|| alloc::format!("{what} ({sectors} sektoru) nenalezen"))
}

fn nvme() -> Result<&'static BlockClient, alloc::string::String> {
    disk(NVME_SECTORS, "testovaci disk NVMe")
}

fn ahci() -> Result<&'static BlockClient, alloc::string::String> {
    disk(AHCI_SECTORS, "testovaci disk AHCI")
}

/// Sektor 0 klienta je prvni sektor naseho oddilu (znacka), ne MBR disku.
fn partition(b: &BlockClient, mark: &[u8]) -> TestResult {
    let mut s = vec![0u8; 512];
    b.read(0, &mut s).map_err(|e| fail("cteni sektoru 0", e))?;
    check!(&s[..mark.len()] == mark, "sektor 0 neni zacatek naseho oddilu");
    let f = b.features().map_err(|e| fail("vlastnosti", e))?;
    check!(f & feature::READ_ONLY == 0, "nas oddil je jen pro cteni");
    Ok(())
}

fn roundtrip(b: &BlockClient) -> TestResult {
    // Vic nez jeden pozadavek protokolu (MAX_SECTORS) a nezarovnany zacatek.
    let data: Vec<u8> = (0..100 * 512).map(|i| (i * 13 + i / 512) as u8).collect();
    b.write(10, &data).map_err(|e| fail("zapis", e))?;
    let mut back = vec![0u8; data.len()];
    b.read(10, &mut back).map_err(|e| fail("cteni", e))?;
    check!(back == data, "prectena data se lisi od zapsanych");
    // Posledni sektor oddilu.
    let last = b.sectors() - 1;
    let one = vec![0x5Au8; 512];
    b.write(last, &one).map_err(|e| fail("zapis posledniho sektoru", e))?;
    let mut back = vec![0u8; 512];
    b.read(last, &mut back).map_err(|e| fail("cteni posledniho sektoru", e))?;
    check!(back == one, "posledni sektor se lisi");
    Ok(())
}

fn flush(b: &BlockClient) -> TestResult {
    let f = b.features().map_err(|e| fail("vlastnosti", e))?;
    check!(
        f & (feature::FLUSH | feature::WRITE_THROUGH) != 0,
        "disk nehlasi zadnou zaruku trvanlivosti"
    );
    b.write(500, &[0x11u8; 4096]).map_err(|e| fail("zapis", e))?;
    b.flush().map_err(|e| fail("flush", e))?;
    Ok(())
}

fn trim(b: &BlockClient) -> TestResult {
    let f = b.features().map_err(|e| fail("vlastnosti", e))?;
    check!(f & feature::DISCARD != 0, "disk nehlasi TRIM");
    b.write(1000, &[0x22u8; 64 * 512]).map_err(|e| fail("zapis", e))?;
    b.discard(1000, 64).map_err(|e| fail("TRIM", e))?;
    // Cely oddil najednou (vic polozek / vic useku v jednom prikazu).
    b.discard(2000, b.sectors() - 2000).map_err(|e| fail("TRIM do konce", e))?;
    check_eq!(
        b.discard(b.sectors() - 1, 2),
        Err(Error::OutOfRange),
        "TRIM za koncem oddilu"
    );
    // Po TRIM jde na stejne misto zase zapsat a precist.
    let data = [0x33u8; 512];
    b.write(1000, &data).map_err(|e| fail("zapis po TRIM", e))?;
    let mut back = [0u8; 512];
    b.read(1000, &mut back).map_err(|e| fail("cteni po TRIM", e))?;
    check!(back == data, "data po TRIM se lisi");
    Ok(())
}

fn bounds(b: &BlockClient) -> TestResult {
    let n = b.sectors();
    let mut buf = vec![0u8; 1024];
    check_eq!(b.read(n - 1, &mut buf), Err(Error::OutOfRange), "cteni pres konec");
    check_eq!(b.write(n, &buf[..512]), Err(Error::OutOfRange), "zapis za konec");
    check_eq!(
        b.write(u64::MAX - 1, &buf),
        Err(Error::OutOfRange),
        "preteceni LBA"
    );
    Ok(())
}

fn dma(b: &BlockClient) -> TestResult {
    let protected = b.test_dma_protection().map_err(|e| fail("test DMA", e))?;
    check!(protected, "radic zapsal mimo svou DMA pamet (IOMMU nechrani)");
    // Ovladac po testu dal funguje.
    let mut s = vec![0u8; 512];
    b.read(0, &mut s).map_err(|e| fail("cteni po testu DMA", e))?;
    Ok(())
}

fn nvme_partition(_: &Startup) -> TestResult {
    partition(nvme()?, NVME_MARK)
}
fn nvme_roundtrip(_: &Startup) -> TestResult {
    roundtrip(nvme()?)
}
fn nvme_flush(_: &Startup) -> TestResult {
    flush(nvme()?)
}
fn nvme_trim(_: &Startup) -> TestResult {
    trim(nvme()?)
}
fn nvme_bounds(_: &Startup) -> TestResult {
    bounds(nvme()?)
}
fn nvme_dma(_: &Startup) -> TestResult {
    dma(nvme()?)
}
fn ahci_partition(_: &Startup) -> TestResult {
    partition(ahci()?, AHCI_MARK)
}
fn ahci_roundtrip(_: &Startup) -> TestResult {
    roundtrip(ahci()?)
}
fn ahci_flush(_: &Startup) -> TestResult {
    flush(ahci()?)
}
fn ahci_trim(_: &Startup) -> TestResult {
    trim(ahci()?)
}
fn ahci_bounds(_: &Startup) -> TestResult {
    bounds(ahci()?)
}
fn ahci_dma(_: &Startup) -> TestResult {
    dma(ahci()?)
}

fn foreign_read_only(_: &Startup) -> TestResult {
    let b = disk(FOREIGN_SECTORS, "cizi disk")?;
    let f = b.features().map_err(|e| fail("vlastnosti", e))?;
    check!(f & feature::READ_ONLY != 0, "cizi disk neni hlasen jen pro cteni");
    check!(f & feature::DISCARD == 0, "cizi disk hlasi TRIM");
    // Klient vidi cely disk: sektor 0 je ochranny MBR.
    let mut s = vec![0u8; 512];
    b.read(0, &mut s).map_err(|e| fail("cteni MBR", e))?;
    check!(s[510] == 0x55 && s[511] == 0xAA, "sektor 0 neni MBR");
    let junk = vec![0xEEu8; 512];
    check_eq!(b.write(0, &junk), Err(Error::ReadOnly), "zapis do MBR");
    check_eq!(
        b.write(FOREIGN_PART_START, &junk),
        Err(Error::ReadOnly),
        "zapis do ciziho oddilu"
    );
    check_eq!(b.discard(FOREIGN_PART_START, 8), Err(Error::ReadOnly), "TRIM ciziho oddilu");
    b.read(FOREIGN_PART_START, &mut s)
        .map_err(|e| fail("cteni ciziho oddilu", e))?;
    check!(&s[..FOREIGN_MARK.len()] == FOREIGN_MARK, "cizi data se zmenila");
    // Flush na cizim disku nic nezapise; smi projit.
    b.flush().map_err(|e| fail("flush", e))?;
    Ok(())
}
