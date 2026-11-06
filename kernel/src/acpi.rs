//! Cteni statickych ACPI tabulek.
//!
//! Jadro cte jen to, co nutne potrebuje samo: seznam CPU a radicu preruseni
//! (MADT) a casovac HPET. Interpretaci AML (napajeni, tlacitka, ...) bude
//! delat sluzba v userspace.

use alloc::vec::Vec;

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

#[allow(dead_code, reason = "polozky se ctou pres Debug ve zprave paniky")]
#[derive(Debug)]
pub enum AcpiError {
    NoRsdp,
    BadChecksum(&'static str),
    BadSignature,
    MissingMadt,
}

/// Cteni z fyzicke pameti pres primou mapu. ACPI tabulky lezi v oblastech
/// z mapy pameti, ktere jsou v prime mape vzdy namapovane.
fn bytes(phys: u64, len: usize) -> &'static [u8] {
    // SAFETY: viz komentar funkce; tabulky se za behu nemeni a jadro je
    // nikdy neuvolni (AcpiReclaimable se do alokatoru nepridava).
    unsafe { core::slice::from_raw_parts(PhysAddr::new(phys).to_virt().as_mut_ptr(), len) }
}

fn u8_at(b: &[u8], off: usize) -> u8 {
    b[off]
}

fn u16_at(b: &[u8], off: usize) -> u16 {
    u16::from_le_bytes(b[off..off + 2].try_into().unwrap())
}

fn u32_at(b: &[u8], off: usize) -> u32 {
    u32::from_le_bytes(b[off..off + 4].try_into().unwrap())
}

fn u64_at(b: &[u8], off: usize) -> u64 {
    u64::from_le_bytes(b[off..off + 8].try_into().unwrap())
}

fn checksum_ok(b: &[u8]) -> bool {
    b.iter().fold(0u8, |acc, &x| acc.wrapping_add(x)) == 0
}

/// Vrati celou tabulku (vcetne 36bajtove hlavicky) a overi jeji checksum.
fn table(phys: u64) -> Result<&'static [u8], AcpiError> {
    let len = u32_at(bytes(phys, 36), 4) as usize;
    let t = bytes(phys, len);
    if !checksum_ok(t) {
        return Err(AcpiError::BadChecksum("tabulka"));
    }
    Ok(t)
}

pub fn parse(rsdp_phys: u64) -> Result<Platform, AcpiError> {
    if rsdp_phys == 0 {
        return Err(AcpiError::NoRsdp);
    }
    let rsdp = bytes(rsdp_phys, 36);
    if &rsdp[0..8] != b"RSD PTR " {
        return Err(AcpiError::BadSignature);
    }
    if !checksum_ok(&rsdp[0..20]) {
        return Err(AcpiError::BadChecksum("RSDP"));
    }
    // ACPI 2.0+: XSDT s 64bitovymi ukazateli. Starsi RSDT jen jako zaloha.
    let (root, entry_size) = if u8_at(rsdp, 15) >= 2 && checksum_ok(rsdp) {
        (u64_at(rsdp, 24), 8)
    } else {
        (u32_at(rsdp, 16) as u64, 4)
    };
    let root = table(root)?;

    let mut platform = Platform::default();
    let mut found_madt = false;
    for off in (36..root.len()).step_by(entry_size) {
        let phys = if entry_size == 8 {
            u64_at(root, off)
        } else {
            u32_at(root, off) as u64
        };
        let t = table(phys)?;
        match &t[0..4] {
            b"APIC" => {
                parse_madt(t, &mut platform);
                found_madt = true;
            }
            b"HPET" => platform.hpet_phys = Some(u64_at(t, 44)),
            _ => {}
        }
    }
    if !found_madt {
        return Err(AcpiError::MissingMadt);
    }
    Ok(platform)
}

fn parse_madt(t: &[u8], p: &mut Platform) {
    p.lapic_phys = u32_at(t, 36) as u64;
    let mut off = 44;
    while off + 2 <= t.len() {
        let (kind, len) = (t[off], t[off + 1] as usize);
        if len < 2 || off + len > t.len() {
            break;
        }
        let e = &t[off..off + len];
        match kind {
            // Lokalni APIC: bit 0 = povolen. CPU s jen "online capable"
            // (hotplug) zatim nestartujeme.
            0 if u32_at(e, 4) & 1 != 0 => p.cpus.push(Cpu {
                apic_id: e[3] as u32,
            }),
            1 => p.ioapics.push(IoApic {
                id: e[2],
                phys: u32_at(e, 4) as u64,
                gsi_base: u32_at(e, 8),
            }),
            2 => p.overrides.push(InterruptOverride {
                source_irq: e[3],
                gsi: u32_at(e, 4),
                flags: u16_at(e, 8),
            }),
            5 => p.lapic_phys = u64_at(e, 4),
            9 if u32_at(e, 8) & 1 != 0 => p.cpus.push(Cpu {
                apic_id: u32_at(e, 4),
            }),
            _ => {}
        }
        off += len;
    }
}
