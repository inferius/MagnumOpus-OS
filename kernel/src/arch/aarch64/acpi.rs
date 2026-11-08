//! ACPI tabulky specificke pro aarch64: MADT (rozhrani CPU a casti GICv3),
//! FADT (zpusob volani PSCI), IORT (SMMUv3 a mapovani RID na StreamID).
//! Obecnou cast (RSDP, XSDT, MCFG) cte `crate::acpi`. Ladici linku (SPCR)
//! najde uz loader.

use alloc::vec::Vec;
use core::fmt;

use super::regs::mpidr_to_hw_id;
use crate::acpi::read::{u8_at, u16_at, u32_at, u64_at};
use crate::acpi::{Cpu, Platform};
use crate::error::{Error, Result};
use crate::log;

/// Udaje o platforme aarch64 z ACPI.
#[derive(Debug, Default)]
pub struct Aarch64Platform {
    /// Distributor GICv3 (MADT GICD), fyzicka adresa.
    pub gicd: u64,
    /// Oblasti redistributoru (MADT GICR): (fyzicka adresa, delka).
    pub gicr: Vec<(u64, u64)>,
    /// Redistributory jednotlivych CPU (MADT GICC), kdyz chybi oblasti GICR.
    pub gicc_gicr: Vec<u64>,
    /// ITS (MADT GIC ITS), fyzicke adresy.
    pub its: Vec<u64>,
    /// PSCI se vola instrukci HVC (jinak SMC), z FADT.
    pub psci_hvc: bool,
    /// FADT hlasi PSCI (bez nej dalsi CPU nenastartujeme).
    pub psci: bool,
    /// SMMUv3 pred korenovym komplexem PCIe (IORT): fyzicka adresa
    /// a mapovani (vstupni baze RID, vystupni baze StreamID).
    pub smmu: Option<(u64, u32, u32)>,
}

impl fmt::Display for Aarch64Platform {
    /// Souhrn do logu: "GICv3 0x8000000, 1 ITS, PSCI hvc".
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "GICv3 {:#x}, {} ITS, PSCI {}, SMMUv3 {}",
            self.gicd,
            self.its.len(),
            match (self.psci, self.psci_hvc) {
                (false, _) => "ne",
                (true, true) => "hvc",
                (true, false) => "smc",
            },
            if self.smmu.is_some() { "ano" } else { "ne" }
        )
    }
}

/// Zpracuje jednu tabulku (volano z `crate::acpi` pres smlouvu).
pub fn table(signature: &[u8; 4], t: &[u8], p: &mut Platform) {
    match signature {
        b"APIC" => parse_madt(t, p),
        b"IORT" => parse_iort(t, p),
        b"FACP" => {
            // ARM_BOOT_ARCH (offset 129): bit 0 PSCI, bit 1 PSCI pres HVC.
            if let Some(flags) = u16_at(t, 129) {
                p.arch.psci = flags & 1 != 0;
                p.arch.psci_hvc = flags & 2 != 0;
            }
        }
        _ => {}
    }
}

/// Kontrola po precteni vsech tabulek.
pub fn check(p: &Platform) -> Result<()> {
    if p.arch.gicd == 0 || p.cpus.is_empty() {
        log::error!("MADT nepopisuje GICv3 (distributor a rozhrani CPU)");
        return Err(Error::NotFound);
    }
    if p.arch.gicr.is_empty() && p.arch.gicc_gicr.is_empty() {
        log::error!("MADT nepopisuje redistributory GICv3");
        return Err(Error::NotFound);
    }
    Ok(())
}

fn parse_madt(t: &[u8], p: &mut Platform) {
    let mut off = 44;
    while off + 2 <= t.len() {
        let (kind, len) = (t[off], t[off + 1] as usize);
        if len < 2 || off + len > t.len() {
            log::warn!("MADT: poskozena polozka na offsetu {off}, zbytek vynechan");
            break;
        }
        if parse_madt_entry(kind, &t[off..off + len], p).is_none() {
            log::warn!("MADT: prilis kratka polozka typu {kind} ({len} B), vynechana");
        }
        off += len;
    }
}

/// IORT: uzly (typ u8, delka u16, pocet mapovani ID u32 na offsetu 8,
/// offset pole mapovani u32 na 12). SMMUv3 (typ 4) ma adresu registru na
/// offsetu 16. Korenovy komplex PCIe (typ 2) mapuje RID pres polozky
/// (vstupni baze, pocet, vystupni baze, odkaz na cilovy uzel, priznaky; po
/// 20 B); kdyz odkaz vede na SMMUv3, zarizeni jsou za ni.
pub(super) fn parse_iort(t: &[u8], p: &mut Platform) {
    let (Some(count), Some(first)) = (u32_at(t, 36), u32_at(t, 40)) else {
        log::warn!("IORT je prilis kratka, vynechana");
        return;
    };
    let nodes = || {
        let mut off = first as usize;
        (0..count).map_while(move |_| {
            let kind = u8_at(t, off)?;
            let len = u16_at(t, off + 1)? as usize;
            let node = off;
            off += len.max(1);
            (len >= 16 && node + len <= t.len()).then_some((node, kind))
        })
    };
    for (rc, _) in nodes().filter(|&(_, k)| k == 2) {
        let (Some(maps), Some(array)) = (u32_at(t, rc + 8), u32_at(t, rc + 12)) else {
            continue;
        };
        for i in 0..maps as usize {
            let m = rc + array as usize + i * 20;
            let (Some(input), Some(output), Some(target)) =
                (u32_at(t, m), u32_at(t, m + 8), u32_at(t, m + 12))
            else {
                break;
            };
            let target = target as usize;
            if nodes().any(|(n, k)| n == target && k == 4)
                && let Some(base) = u64_at(t, target + 16)
            {
                p.arch.smmu = Some((base, input, output));
                return;
            }
        }
    }
}

/// Zpracuje jednu polozku MADT; `None` = polozka je kratsi nez format.
pub(super) fn parse_madt_entry(kind: u8, e: &[u8], p: &mut Platform) -> Option<()> {
    match kind {
        // GICC: rozhrani CPU. Priznak bit 0 = povolene.
        0x0B => {
            let flags = u32_at(e, 12)?;
            let gicr = u64_at(e, 60)?;
            let mpidr = u64_at(e, 68)?;
            if flags & 1 != 0 {
                p.cpus.push(Cpu {
                    hw_id: mpidr_to_hw_id(mpidr),
                    uid: u32_at(e, 8)?,
                });
                if gicr != 0 {
                    p.arch.gicc_gicr.push(gicr);
                }
            }
        }
        // GICD: distributor (verze GIC na offsetu 20: 3 nebo 4).
        0x0C => {
            p.arch.gicd = u64_at(e, 8)?;
            let version = u8_at(e, 20)?;
            if version != 0 && version < 3 {
                log::warn!("MADT: GIC verze {version}, jadro umi jen GICv3/v4");
            }
        }
        // GICR: oblast redistributoru.
        0x0E => {
            let base = u64_at(e, 4)?;
            let len = u32_at(e, 12)? as u64;
            p.arch.gicr.push((base, len));
        }
        // GIC ITS.
        0x0F => p.arch.its.push(u64_at(e, 8)?),
        _ => {}
    }
    Some(())
}
