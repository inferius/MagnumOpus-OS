//! Cteni statickych ACPI tabulek.
//!
//! Jadro cte jen to, co nutne potrebuje samo: seznam CPU a radicu preruseni
//! (MADT) a casovac HPET. Interpretaci AML (napajeni, tlacitka, ...) bude
//! delat sluzba v userspace.

use alloc::vec::Vec;

use crate::error::{Error, Result, ResultExt};
use crate::log;
use crate::mm::PhysAddr;

#[derive(Debug, Clone, Copy)]
pub struct Cpu {
    /// ID lokalniho APIC (x2APIC ID).
    pub apic_id: u32,
}

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

/// Informace o platforme z ACPI.
#[derive(Debug, Default)]
pub struct Platform {
    pub cpus: Vec<Cpu>,
    pub lapic_phys: u64,
    pub ioapics: Vec<IoApic>,
    pub overrides: Vec<InterruptOverride>,
    pub hpet_phys: Option<u64>,
}

/// Cteni z fyzicke pameti pres primou mapu. ACPI tabulky lezi v oblastech
/// z mapy pameti, ktere jsou v prime mape vzdy namapovane.
fn bytes(phys: u64, len: usize) -> &'static [u8] {
    // SAFETY: viz komentar funkce; tabulky se za behu nemeni a jadro je
    // nikdy neuvolni (AcpiReclaimable se do alokatoru nepridava).
    unsafe { core::slice::from_raw_parts(PhysAddr::new(phys).to_virt().as_mut_ptr(), len) }
}

// Cteni poli s kontrolou mezi: poskozena tabulka firmwaru nesmi shodit jadro.
fn u8_at(b: &[u8], off: usize) -> Option<u8> {
    b.get(off).copied()
}

fn u16_at(b: &[u8], off: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(off..off + 2)?.try_into().ok()?))
}

fn u32_at(b: &[u8], off: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(off..off + 4)?.try_into().ok()?))
}

fn u64_at(b: &[u8], off: usize) -> Option<u64> {
    Some(u64::from_le_bytes(b.get(off..off + 8)?.try_into().ok()?))
}

fn checksum_ok(b: &[u8]) -> bool {
    b.iter().fold(0u8, |acc, &x| acc.wrapping_add(x)) == 0
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

    let mut platform = Platform::default();
    let mut found_madt = false;
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
        match &t[0..4] {
            b"APIC" => {
                parse_madt(t, &mut platform);
                found_madt = true;
            }
            b"HPET" => match u64_at(t, 44) {
                Some(addr) => platform.hpet_phys = Some(addr),
                None => log::warn!("tabulka HPET je prilis kratka, vynechana"),
            },
            _ => {}
        }
    }
    if !found_madt {
        log::error!("chybi platna tabulka MADT (seznam CPU a radicu preruseni)");
        return Err(Error::NotFound);
    }
    Ok(platform)
}

fn parse_madt(t: &[u8], p: &mut Platform) {
    p.lapic_phys = u32_at(t, 36).unwrap_or(0xFEE0_0000) as u64;
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
pub(crate) fn parse_madt_entry(kind: u8, e: &[u8], p: &mut Platform) -> Option<()> {
    match kind {
        // Lokalni APIC: bit 0 = povolen. CPU s jen "online capable"
        // (hotplug) zatim nestartujeme.
        0 if u32_at(e, 4)? & 1 != 0 => p.cpus.push(Cpu {
            apic_id: u8_at(e, 3)? as u32,
        }),
        1 => p.ioapics.push(IoApic {
            id: u8_at(e, 2)?,
            phys: u32_at(e, 4)? as u64,
            gsi_base: u32_at(e, 8)?,
        }),
        2 => p.overrides.push(InterruptOverride {
            source_irq: u8_at(e, 3)?,
            gsi: u32_at(e, 4)?,
            flags: u16_at(e, 8)?,
        }),
        5 => p.lapic_phys = u64_at(e, 4)?,
        9 if u32_at(e, 8)? & 1 != 0 => p.cpus.push(Cpu {
            apic_id: u32_at(e, 4)?,
        }),
        _ => {}
    }
    Some(())
}
