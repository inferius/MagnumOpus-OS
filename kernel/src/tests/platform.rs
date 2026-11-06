//! Testy ACPI a spolecneho typu chyb.

use crate::acpi::{self, Platform};
use crate::error::Error;

pub const TESTS: &[(&str, fn())] = &[
    ("acpi: kratka polozka MADT se preskoci", madt_short_entry),
    ("chyby: stabilni kody a nazvy", error_codes),
];

fn madt_short_entry() {
    let mut p = Platform::default();
    // Typ 0 (lokalni APIC) ma mit 8 B; 4 B nesmi shodit jadro.
    assert!(acpi::parse_madt_entry(0, &[0, 4, 1, 2], &mut p).is_none());
    assert!(p.cpus.is_empty());
    // Platna polozka: povoleny APIC s ID 7.
    assert!(acpi::parse_madt_entry(0, &[0, 8, 1, 7, 1, 0, 0, 0], &mut p).is_some());
    assert_eq!(p.cpus.len(), 1);
    assert_eq!(p.cpus[0].apic_id, 7);
}

fn error_codes() {
    // Kody jsou soucast ABI syscallu, nesmi se menit.
    assert_eq!(Error::BadHandle.code(), -1);
    assert_eq!(Error::TimedOut.code(), -4);
    assert_eq!(Error::Corrupted.code(), -10);
    assert_eq!(Error::AccessDenied.name(), "ACCESS_DENIED");
}
