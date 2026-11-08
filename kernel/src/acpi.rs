//! Cteni statickych ACPI tabulek.
//!
//! Jadro cte jen to, co nutne potrebuje samo. Obecna cast (tento modul):
//! RSDP, XSDT/RSDT a MCFG (okna konfigurace PCI Express). Ostatni tabulky
//! (seznam CPU a radicu preruseni v MADT, casovace, IOMMU) jsou specificke
//! pro architekturu a zpracuje je `arch` (`api::PlatformInfo`). Interpretaci
//! AML (napajeni, tlacitka, ...) bude delat sluzba v userspace.

use alloc::vec::Vec;

use crate::arch;
use crate::error::{Error, Result, ResultExt};
use crate::log;
use crate::mm::PhysAddr;

use self::read::{checksum_ok, u8_at, u16_at, u32_at, u64_at};

#[derive(Debug, Clone, Copy)]
pub struct Cpu {
    /// Hardwarove ID CPU (x86: x2APIC ID, aarch64: afinita z MPIDR).
    pub hw_id: u32,
}

/// Informace o platforme z ACPI.
#[derive(Debug)]
pub struct Platform {
    /// Povolena CPU (plni architektura z MADT).
    pub cpus: Vec<Cpu>,
    /// Okna konfiguracniho prostoru PCI Express (ECAM) z tabulky MCFG.
    pub pci_ecam: Vec<PciEcam>,
    /// Udaje specificke pro architekturu (radice preruseni, casovace, IOMMU).
    pub arch: arch::PlatformArchInfo,
    /// Fyzicka adresa RSDP a jeho kopie (36 B; ACPI 1.0 jen 20 B) pro
    /// sluzbu ACPI: RSDP muze lezet v pameti, kterou jadro po startu
    /// uvolni.
    pub rsdp_phys: u64,
    pub rsdp: [u8; 36],
}

impl Default for Platform {
    fn default() -> Self {
        Self {
            cpus: Vec::new(),
            pci_ecam: Vec::new(),
            arch: Default::default(),
            rsdp_phys: 0,
            rsdp: [0; 36],
        }
    }
}

/// Jedno okno ECAM: konfigurace sbernic `bus_start..=bus_end` segmentu.
#[derive(Debug, Clone, Copy)]
pub struct PciEcam {
    pub phys: u64,
    pub segment: u16,
    pub bus_start: u8,
    pub bus_end: u8,
}

/// Cteni z fyzicke pameti pres primou mapu. ACPI tabulky lezi v oblastech
/// z mapy pameti, ktere jsou v prime mape vzdy namapovane.
fn bytes(phys: u64, len: usize) -> &'static [u8] {
    // SAFETY: viz komentar funkce; tabulky se za behu nemeni a jadro je
    // nikdy neuvolni (AcpiReclaimable se do alokatoru nepridava).
    unsafe { core::slice::from_raw_parts(PhysAddr::new(phys).to_virt().as_mut_ptr(), len) }
}

/// Cteni poli tabulek s kontrolou mezi: poskozena tabulka firmwaru nesmi
/// shodit jadro. Pouziva i cast ACPI v `arch`.
pub mod read {
    pub fn u8_at(b: &[u8], off: usize) -> Option<u8> {
        b.get(off).copied()
    }

    pub fn u16_at(b: &[u8], off: usize) -> Option<u16> {
        Some(u16::from_le_bytes(b.get(off..off + 2)?.try_into().ok()?))
    }

    pub fn u32_at(b: &[u8], off: usize) -> Option<u32> {
        Some(u32::from_le_bytes(b.get(off..off + 4)?.try_into().ok()?))
    }

    pub fn u64_at(b: &[u8], off: usize) -> Option<u64> {
        Some(u64::from_le_bytes(b.get(off..off + 8)?.try_into().ok()?))
    }

    pub fn checksum_ok(b: &[u8]) -> bool {
        b.iter().fold(0u8, |acc, &x| acc.wrapping_add(x)) == 0
    }
}

const HEADER_LEN: usize = 36;
/// Rozumny strop velikosti tabulky; vetsi delka znamena poskozenou hlavicku.
const MAX_TABLE_LEN: usize = 1 << 20;

/// Vrati celou tabulku (vcetne hlavicky) po overeni delky a checksumu.
fn table(phys: u64) -> Result<&'static [u8]> {
    if phys == 0 {
        return Err(Error::Corrupted);
    }
    let len = u32_at(bytes(phys, HEADER_LEN), 4).ok_or(Error::Corrupted)? as usize;
    if !(HEADER_LEN..=MAX_TABLE_LEN).contains(&len) {
        return Err(Error::Corrupted);
    }
    let t = bytes(phys, len);
    if !checksum_ok(t) {
        return Err(Error::Corrupted);
    }
    Ok(t)
}

fn signature(t: &[u8]) -> &str {
    core::str::from_utf8(&t[0..4]).unwrap_or("????")
}

pub fn parse(rsdp_phys: u64) -> Result<Platform> {
    if rsdp_phys == 0 {
        log::error!("loader nepredal ACPI RSDP");
        return Err(Error::NotFound);
    }
    let rsdp = bytes(rsdp_phys, 36);
    if &rsdp[0..8] != b"RSD PTR " || !checksum_ok(&rsdp[0..20]) {
        log::error!("neplatny ACPI RSDP na {rsdp_phys:#x}");
        return Err(Error::Corrupted);
    }
    // ACPI 2.0+: XSDT s 64bitovymi ukazateli. Starsi RSDT jen jako zaloha.
    let xsdt = u8_at(rsdp, 15).is_some_and(|rev| rev >= 2) && checksum_ok(rsdp);
    let (root_phys, entry_size) = if xsdt {
        (u64_at(rsdp, 24).unwrap_or(0), 8)
    } else {
        (u32_at(rsdp, 16).unwrap_or(0) as u64, 4)
    };
    let root = table(root_phys).warn_context("korenova ACPI tabulka (XSDT/RSDT)")?;

    let mut platform = Platform { rsdp_phys, ..Platform::default() };
    let rsdp_len = if xsdt { 36 } else { 20 };
    platform.rsdp[..rsdp_len].copy_from_slice(&rsdp[..rsdp_len]);
    let mut off = HEADER_LEN;
    while off + entry_size <= root.len() {
        let phys = if entry_size == 8 {
            u64_at(root, off)
        } else {
            u32_at(root, off).map(u64::from)
        };
        off += entry_size;
        // Vadna tabulka, kterou nepotrebujeme, nesmi zastavit start:
        // zalogovat a pokracovat.
        let t = match phys.ok_or(Error::Corrupted).and_then(table) {
            Ok(t) => t,
            Err(e) => {
                log::warn!("ACPI tabulka na {:#x} vynechana: {e}", phys.unwrap_or(0));
                continue;
            }
        };
        log::debug!(
            "ACPI tabulka {} na {:#x}, {} B",
            signature(t),
            phys.unwrap_or(0),
            t.len()
        );
        let signature: &[u8; 4] = t[0..4].try_into().unwrap();
        match signature {
            b"MCFG" => parse_mcfg(t, &mut platform),
            _ => arch::acpi_table(signature, t, &mut platform),
        }
    }
    arch::acpi_check(&platform)?;
    Ok(platform)
}

fn parse_mcfg(t: &[u8], p: &mut Platform) {
    // Hlavicka 36 B + 8 B rezerva, pak polozky po 16 B.
    let mut off = 44;
    while off + 16 <= t.len() {
        let entry = (|| {
            Some(PciEcam {
                phys: u64_at(t, off)?,
                segment: u16_at(t, off + 8)?,
                bus_start: u8_at(t, off + 10)?,
                bus_end: u8_at(t, off + 11)?,
            })
        })();
        if let Some(e) = entry {
            p.pci_ecam.push(e);
        }
        off += 16;
    }
}
