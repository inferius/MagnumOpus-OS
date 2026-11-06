//! Testy ACPI, realneho casu a spolecneho typu chyb.

use crate::acpi::{self, Platform};
use crate::error::Error;

pub const TESTS: &[(&str, fn())] = &[
    ("acpi: kratka polozka MADT se preskoci", madt_short_entry),
    ("chyby: stabilni kody a nazvy", error_codes),
    ("cas: CMOS RTC odpovida realnemu casu", rtc_matches_wall_clock),
];

fn rtc_matches_wall_clock() {
    let p = crate::PLATFORM.get().unwrap();
    // QEMU ma RTC v UTC (vychozi `-rtc base=utc`) a UEFI cte stejne hodiny.
    let rtc = crate::arch::rtc::read_time(p.rtc_century).expect("RTC vratil nesmysl");
    let wall = crate::clock::now().expect("realny cas neni znamy").time;
    let diff = (rtc.secs - wall.secs).abs();
    assert!(diff <= 2, "RTC a realny cas se lisi o {diff} s");
}

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
