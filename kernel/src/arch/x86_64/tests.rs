//! Self-testy casti jadra specificke pro x86_64 (spousti je `crate::tests`).

use super::acpi::parse_madt_entry;
use crate::acpi::Platform;

pub const TESTS: &[(&str, fn())] = &[("acpi: kratka polozka MADT se preskoci", madt_short_entry)];

fn madt_short_entry() {
    let mut p = Platform::default();
    // Typ 0 (lokalni APIC) ma mit 8 B; 4 B nesmi shodit jadro.
    assert!(parse_madt_entry(0, &[0, 4, 1, 2], &mut p).is_none());
    assert!(p.cpus.is_empty());
    // Platna polozka: povoleny APIC s ID 7.
    assert!(parse_madt_entry(0, &[0, 8, 1, 7, 1, 0, 0, 0], &mut p).is_some());
    assert_eq!(p.cpus.len(), 1);
    assert_eq!(p.cpus[0].hw_id, 7);
}
