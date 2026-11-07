//! Testy realneho casu a spolecneho typu chyb. Testy ACPI tabulek konkretni
//! architektury jsou v `arch/<cil>/tests.rs` (`arch::TESTS`).

use crate::error::Error;

pub const TESTS: &[(&str, fn())] = &[
    ("chyby: stabilni kody a nazvy", error_codes),
    ("cas: hardwarove hodiny odpovidaji realnemu casu", hardware_clock_matches_wall_clock),
];

fn hardware_clock_matches_wall_clock() {
    let p = crate::PLATFORM.get().unwrap();
    // QEMU ma RTC v UTC (vychozi `-rtc base=utc`) a UEFI cte stejne hodiny.
    // Architektura bez vlastnich hodin (zatim aarch64) test preskoci.
    let Some(hw) = crate::arch::hardware_clock(p) else { return };
    let wall = crate::clock::now().expect("realny cas neni znamy").time;
    let diff = (hw.secs - wall.secs).abs();
    assert!(diff <= 2, "hardwarove hodiny a realny cas se lisi o {diff} s");
}

fn error_codes() {
    // Kody jsou soucast ABI syscallu, nesmi se menit.
    assert_eq!(Error::BadHandle.code(), -1);
    assert_eq!(Error::TimedOut.code(), -4);
    assert_eq!(Error::Corrupted.code(), -10);
    assert_eq!(Error::AccessDenied.name(), "ACCESS_DENIED");
}
