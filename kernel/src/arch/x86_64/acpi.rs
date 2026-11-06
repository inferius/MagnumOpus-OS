//! ACPI tabulky specificke pro x86: MADT (lokalni APIC, IOAPIC), HPET,
//! DMAR (Intel VT-d) a IVRS (AMD-Vi). Obecnou cast (RSDP, XSDT, MCFG) cte
//! `crate::acpi`, ktery sem preda ostatni tabulky.

use alloc::vec::Vec;
use core::fmt;

use crate::acpi::read::{u8_at, u16_at, u32_at, u64_at};
use crate::acpi::{Cpu, Platform};
use crate::error::{Error, Result};
use crate::log;

#[allow(
    dead_code,
    reason = "IOAPIC pouzije sprava preruseni zarizeni (milnik 6)"
)]
#[derive(Debug, Clone, Copy)]
pub struct IoApic {
    pub id: u8,
    pub phys: u64,
    pub gsi_base: u32,
}

#[allow(
    dead_code,
    reason = "IOAPIC pouzije sprava preruseni zarizeni (milnik 6)"
)]
#[derive(Debug, Clone, Copy)]
pub struct InterruptOverride {
    pub source_irq: u8,
    pub gsi: u32,
    pub flags: u16,
}

/// Jednotka AMD-Vi (IVHD z tabulky IVRS). Pokryva zarizeni sveho segmentu.
#[derive(Debug, Clone)]
pub struct AmdIommuUnit {
    pub segment: u16,
    /// Fyzicka adresa registru.
    pub registers: u64,
    /// Adresa samotne IOMMU na sbernici PCI (sbernice << 8 | zarizeni << 3
    /// | funkce) - pres jeji konfiguraci se nastavuje preruseni MSI.
    pub device_id: u16,
}

/// Jednotka preklada DMA (DRHD z tabulky DMAR).
#[derive(Debug, Clone)]
pub struct IommuUnit {
    pub segment: u16,
    /// Fyzicka adresa registru.
    pub registers: u64,
    /// Pokryva vsechna zarizeni segmentu, ktera nepokryva jina jednotka.
    pub include_all: bool,
    /// Zarizeni vyjmenovana v tabulce: (sbernice, zarizeni << 3 | funkce).
    pub devices: Vec<(u8, u8)>,
}

/// Udaje o platforme x86 z ACPI.
#[derive(Debug, Default)]
pub struct X86Platform {
    /// Byla nalezena platna MADT (bez ni nevime, jaka CPU existuji).
    madt_found: bool,
    pub lapic_phys: u64,
    pub ioapics: Vec<IoApic>,
    pub overrides: Vec<InterruptOverride>,
    pub hpet_phys: Option<u64>,
    /// Jednotky IOMMU (Intel VT-d) z tabulky DMAR.
    pub iommus: Vec<IommuUnit>,
    /// Jednotky IOMMU (AMD-Vi) z tabulky IVRS.
    pub amd_iommus: Vec<AmdIommuUnit>,
}

impl fmt::Display for X86Platform {
    /// Souhrn do logu: "1 IOAPIC, HPET ano".
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} IOAPIC, HPET {}",
            self.ioapics.len(),
            if self.hpet_phys.is_some() {
                "ano"
            } else {
                "ne"
            }
        )
    }
}

/// Zpracuje jednu tabulku (volano z `crate::acpi` pres smlouvu).
pub fn table(signature: &[u8; 4], t: &[u8], p: &mut Platform) {
    match signature {
        b"APIC" => {
            parse_madt(t, p);
            p.arch.madt_found = true;
        }
        b"DMAR" => parse_dmar(t, &mut p.arch),
        b"IVRS" => parse_ivrs(t, &mut p.arch),
        b"HPET" => match u64_at(t, 44) {
            Some(addr) => p.arch.hpet_phys = Some(addr),
            None => log::warn!("tabulka HPET je prilis kratka, vynechana"),
        },
        _ => {}
    }
}

/// Kontrola po precteni vsech tabulek.
pub fn check(p: &Platform) -> Result<()> {
    if !p.arch.madt_found {
        log::error!("chybi platna tabulka MADT (seznam CPU a radicu preruseni)");
        return Err(Error::NotFound);
    }
    Ok(())
}

fn parse_madt(t: &[u8], p: &mut Platform) {
    p.arch.lapic_phys = u32_at(t, 36).unwrap_or(0xFEE0_0000) as u64;
    let mut off = 44;
    while off + 2 <= t.len() {
        let (kind, len) = (t[off], t[off + 1] as usize);
        if len < 2 || off + len > t.len() {
            log::warn!("MADT: poskozena polozka na offsetu {off}, zbytek vynechan");
            break;
        }
        let e = &t[off..off + len];
        if parse_madt_entry(kind, e, p).is_none() {
            log::warn!("MADT: prilis kratka polozka typu {kind} ({len} B), vynechana");
        }
        off += len;
    }
}

/// Zpracuje jednu polozku MADT; `None` = polozka je kratsi nez ocekavany format.
pub(super) fn parse_madt_entry(kind: u8, e: &[u8], p: &mut Platform) -> Option<()> {
    match kind {
        // Lokalni APIC: bit 0 = povolen. CPU s jen "online capable"
        // (hotplug) zatim nestartujeme.
        0 if u32_at(e, 4)? & 1 != 0 => p.cpus.push(Cpu {
            hw_id: u8_at(e, 3)? as u32,
        }),
        1 => p.arch.ioapics.push(IoApic {
            id: u8_at(e, 2)?,
            phys: u32_at(e, 4)? as u64,
            gsi_base: u32_at(e, 8)?,
        }),
        2 => p.arch.overrides.push(InterruptOverride {
            source_irq: u8_at(e, 3)?,
            gsi: u32_at(e, 4)?,
            flags: u16_at(e, 8)?,
        }),
        5 => p.arch.lapic_phys = u64_at(e, 4)?,
        9 if u32_at(e, 8)? & 1 != 0 => p.cpus.push(Cpu {
            hw_id: u32_at(e, 4)?,
        }),
        _ => {}
    }
    Some(())
}

/// DMAR: popis jednotek IOMMU. Za hlavickou (36 B) sirka adresy (1 B),
/// priznaky (1 B), rezerva (10 B); od offsetu 48 struktury (typ u16, delka
/// u16). Typ 0 = DRHD: priznaky (bit 0 = vsechna zarizeni segmentu),
/// segment (offset 6), adresa registru (offset 8), od offsetu 16 rozsahy
/// zarizeni (typ u8, delka u8, ..., sbernice na offsetu 5, cesta
/// (zarizeni, funkce) od offsetu 6). Ostatni typy (RMRR, ATSR) se zatim
/// nepouzivaji.
fn parse_dmar(t: &[u8], p: &mut X86Platform) {
    let mut off = 48;
    while let (Some(kind), Some(len)) = (u16_at(t, off), u16_at(t, off + 2)) {
        let len = len as usize;
        if len < 4 || off + len > t.len() {
            log::warn!("DMAR: poskozena struktura na offsetu {off}, zbytek vynechan");
            return;
        }
        let s = &t[off..off + len];
        if kind == 0 {
            let (Some(flags), Some(segment), Some(registers)) =
                (u8_at(s, 4), u16_at(s, 6), u64_at(s, 8))
            else {
                log::warn!("DMAR: prilis kratka DRHD, vynechana");
                off += len;
                continue;
            };
            let mut devices = Vec::new();
            let mut so = 16;
            while let (Some(_stype), Some(slen)) = (u8_at(s, so), u8_at(s, so + 1)) {
                let slen = slen as usize;
                if slen < 6 || so + slen > s.len() {
                    break;
                }
                // Jen primo pripojena zarizeni (cesta delky 1); mosty
                // s podrizenymi sbernicemi zatim ne.
                if let (Some(bus), Some(dev), Some(func)) =
                    (u8_at(s, so + 5), u8_at(s, so + 6), u8_at(s, so + 7))
                {
                    devices.push((bus, dev << 3 | (func & 7)));
                }
                so += slen;
            }
            p.iommus.push(IommuUnit {
                segment,
                registers,
                include_all: flags & 1 != 0,
                devices,
            });
        }
        off += len;
    }
}

/// IVRS: popis jednotek AMD-Vi. Za hlavickou (36 B) informace (4 B)
/// a rezerva (8 B); od offsetu 48 bloky (typ u8, priznaky u8, delka u16).
/// IVHD (typ 0x10, 0x11, 0x40): ID zarizeni IOMMU (offset 4), adresa
/// registru (offset 8), segment (offset 16). Firmware muze tutez jednotku
/// popsat vic typy IVHD - pouzije se jen prvni podle adresy registru.
fn parse_ivrs(t: &[u8], p: &mut X86Platform) {
    let mut off = 48;
    while let (Some(kind), Some(len)) = (u8_at(t, off), u16_at(t, off + 2)) {
        let len = len as usize;
        if len < 4 || off + len > t.len() {
            log::warn!("IVRS: poskozeny blok na offsetu {off}, zbytek vynechan");
            return;
        }
        if matches!(kind, 0x10 | 0x11 | 0x40) {
            let s = &t[off..off + len];
            match (u16_at(s, 4), u64_at(s, 8), u16_at(s, 16)) {
                (Some(device_id), Some(registers), Some(segment)) => {
                    if !p.amd_iommus.iter().any(|u| u.registers == registers) {
                        p.amd_iommus.push(AmdIommuUnit {
                            segment,
                            registers,
                            device_id,
                        });
                    }
                }
                _ => log::warn!("IVRS: prilis kratky blok IVHD, vynechan"),
            }
        }
        off += len;
    }
}
