//! Vypnuti a restart, udaje o platforme, okna ECAM z MCFG.
//!
//! Vypnuti (S5): `AcpiEnterSleepStatePrep(5)` (metody `_PTS`, `_SST`),
//! pak zapis typu spanku do PM1 (x86) nebo registru Sleep Control
//! (HW-reduced s registry). Kdyz to firmware neumi (ARM: HW-reduced bez
//! registru spanku), vypne se pres PSCI (`SYSTEM_POWER` v jadre).
//!
//! Restart: registr reset z FADT (`AcpiReset`; q35: port 0xCF9 = 6), jinak
//! firmware pres jadro (PSCI SYSTEM_RESET, na x86 0xCF9 a radic klavesnice).

use alloc::string::String;
use alloc::vec::Vec;

use abi::syscall::power_op;
use acpica::ffi;
use libos::hw::Resource;
use libos::{Error, println};
use powerproto::{Status, feature};

use crate::osl::hw::{Ecam, set_ecam};

pub fn platform() -> ffi::Platform {
    let mut p = ffi::Platform::default();
    // SAFETY: tabulky jsou nactene; struktura ma rozlozeni MAGNUM_PLATFORM.
    let st = unsafe { ffi::AcpiMagnumPlatform(&mut p) };
    if st != ffi::AE_OK {
        println!("acpid: FADT nelze precist: {}", acpica::status_name(st));
    }
    p
}

/// Okna konfigurace PCI z tabulky MCFG (pro `AcpiOsReadPciConfiguration`).
pub fn read_mcfg() {
    let mut table: *mut u8 = core::ptr::null_mut();
    // SAFETY: AcpiGetTable vrati ukazatel na namapovanou tabulku.
    let st = unsafe { AcpiGetTable(c"MCFG".as_ptr(), 1, &mut table) };
    if st != ffi::AE_OK || table.is_null() {
        println!("acpid: tabulka MCFG neni (konfigurace PCI jen pres porty)");
        return;
    }
    // SAFETY: hlavicka ACPI (36 B) s delkou na offsetu 4.
    let len = unsafe { (table.add(4) as *const u32).read_unaligned() } as usize;
    let mut windows = Vec::new();
    let mut off = 44;
    while off + 16 <= len.min(4096) {
        // SAFETY: zaznam lezi uvnitr tabulky (overeno delkou).
        let (phys, segment, start, end) = unsafe {
            (
                (table.add(off) as *const u64).read_unaligned(),
                (table.add(off + 8) as *const u16).read_unaligned(),
                *table.add(off + 10),
                *table.add(off + 11),
            )
        };
        windows.push(Ecam { phys, segment, bus_start: start, bus_end: end });
        off += 16;
    }
    println!("acpid: MCFG: {} oken ECAM", windows.len());
    set_ecam(windows);
}

unsafe extern "C" {
    fn AcpiGetTable(sig: *const core::ffi::c_char, instance: u32, out: *mut *mut u8) -> ffi::Status;
}

pub fn status(p: &ffi::Platform) -> Status {
    let mut features = feature::REBOOT;
    // S5 jde vzdy: na HW-reduced bez registru spanku pres PSCI.
    if p.s5_supported != 0 || p.hw_reduced != 0 {
        features |= feature::SHUTDOWN;
    }
    if crate::events::HAS_POWER_BUTTON.load(core::sync::atomic::Ordering::Acquire) {
        features |= feature::POWER_BUTTON;
    }
    if p.hw_reduced != 0 {
        features |= feature::HW_REDUCED;
    }
    let oem_len = p.oem_id.iter().position(|&b| b == 0).unwrap_or(6);
    Status {
        features,
        fadt_revision: p.revision,
        oem: String::from_utf8_lossy(&p.oem_id[..oem_len]).trim_end().into(),
        thermal_zones: crate::sensors::thermal_count(),
        batteries: crate::sensors::battery_count(),
        cpus: crate::sensors::cpu_count(),
    }
}

/// Vypne pocitac. Vraci jen pri chybe.
pub fn shutdown(p: &ffi::Platform, power: &Resource) -> Error {
    println!("acpid: vstupuji do S5 (vypnuti)");
    // SAFETY: ACPICA je inicializovana; volani z hlavniho vlakna.
    let prep = unsafe { ffi::AcpiEnterSleepStatePrep(ffi::ACPI_STATE_S5) };
    if prep != ffi::AE_OK {
        println!("acpid: priprava S5 selhala: {}", acpica::status_name(prep));
    }
    if p.s5_supported != 0 && (p.hw_reduced == 0 || p.sleep_registers != 0) {
        // SAFETY: dtto. Pri uspechu se pocitac vypne behem zapisu.
        let st = unsafe { ffi::AcpiEnterSleepState(ffi::ACPI_STATE_S5) };
        println!("acpid: zapis S5 se vratil: {}", acpica::status_name(st));
    }
    println!("acpid: vypinam pres firmware (PSCI)");
    match power.system_power(power_op::OFF) {
        Ok(()) => Error::BadState,
        Err(e) => {
            println!("acpid: vypnuti selhalo: {e}");
            e
        }
    }
}

/// Restartuje pocitac. Vraci jen pri chybe.
pub fn reboot(p: &ffi::Platform, power: &Resource) -> Error {
    println!("acpid: restartuji");
    if p.reset_register != 0 {
        // SAFETY: ACPICA je inicializovana.
        let st = unsafe { ffi::AcpiReset() };
        if st == ffi::AE_OK {
            // Zapis do registru reset muze chvili trvat.
            libos::time::sleep(core::time::Duration::from_millis(500));
        }
        println!("acpid: registr reset z FADT nezabral ({})", acpica::status_name(st));
    }
    match power.system_power(power_op::RESET) {
        Ok(()) => Error::BadState,
        Err(e) => {
            println!("acpid: restart selhal: {e}");
            e
        }
    }
}
