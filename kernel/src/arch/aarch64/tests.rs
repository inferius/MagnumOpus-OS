//! Self-testy casti jadra specificke pro aarch64 (spousti je `crate::tests`).

use super::acpi::parse_madt_entry;
use super::regs::{hw_id_to_mpidr, mpidr_to_hw_id};
use crate::acpi::Platform;

pub const TESTS: &[(&str, fn())] = &[
    ("acpi: polozky MADT pro GICv3", madt_entries),
    ("MPIDR <-> hardwarove ID", mpidr_roundtrip),
];

fn madt_entries() {
    let mut p = Platform::default();
    // GICC ma mit 80 B; kratsi nesmi shodit jadro.
    assert!(parse_madt_entry(0x0B, &[0x0B, 8, 0, 0, 0, 0, 0, 0], &mut p).is_none());
    assert!(p.cpus.is_empty());
    // Povolene GICC s MPIDR 0x0000_0001 (Aff0 = 1) a redistributorem.
    let mut gicc = [0u8; 80];
    gicc[0] = 0x0B;
    gicc[1] = 80;
    gicc[12] = 1;
    gicc[60..68].copy_from_slice(&0x080A_0000u64.to_le_bytes());
    gicc[68..76].copy_from_slice(&1u64.to_le_bytes());
    assert!(parse_madt_entry(0x0B, &gicc, &mut p).is_some());
    assert_eq!(p.cpus.len(), 1);
    assert_eq!(p.cpus[0].hw_id, 1);
    assert_eq!(p.arch.gicc_gicr, [0x080A_0000]);
    // GICD a ITS.
    let mut gicd = [0u8; 24];
    gicd[8..16].copy_from_slice(&0x0800_0000u64.to_le_bytes());
    gicd[20] = 3;
    assert!(parse_madt_entry(0x0C, &gicd, &mut p).is_some());
    assert_eq!(p.arch.gicd, 0x0800_0000);
    let mut its = [0u8; 20];
    its[8..16].copy_from_slice(&0x0808_0000u64.to_le_bytes());
    assert!(parse_madt_entry(0x0F, &its, &mut p).is_some());
    assert_eq!(p.arch.its, [0x0808_0000]);
}

fn mpidr_roundtrip() {
    // Aff3 je v MPIDR na bitech 39:32, v hardwarovem ID na 31:24.
    let mpidr = 0x0000_0002_0003_0405u64;
    let id = mpidr_to_hw_id(mpidr);
    assert_eq!(id, 0x0203_0405);
    assert_eq!(hw_id_to_mpidr(id), mpidr);
}
