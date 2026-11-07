//! Testy realneho casu a spolecneho typu chyb. Testy ACPI tabulek konkretni
//! architektury jsou v `arch/<cil>/tests.rs` (`arch::TESTS`).

use crate::error::Error;

pub const TESTS: &[(&str, fn())] = &[
    ("chyby: stabilni kody a nazvy", error_codes),
    ("cas: hardwarove hodiny odpovidaji realnemu casu", hardware_clock_matches_wall_clock),
    ("cas: plynula korekce (slew, frekvence) bez kroku zpet", timeline_slew),
];

fn timeline_slew() {
    use crate::clock::Timeline;
    use abi::time::{WallTime, adjust::MAX_SLEW_PPB};
    const S: i128 = 1_000_000_000;
    let t0 = WallTime::new(1_000_000, 0);
    let mut t = Timeline::new(0, t0);
    // Dorovnat -1 s (hodiny jdou napred): zpomalit, nikdy zpet.
    t.adjust(10 * S, Some(-1_000_000_000), None);
    let dur = 1_000_000_000i128 * 1_000_000_000 / MAX_SLEW_PPB as i128;
    let mut last = t.real_at(10 * S).as_nanos();
    for k in 1..=2000 {
        let m = 10 * S + dur * k / 1000;
        let r = t.real_at(m).as_nanos();
        assert!(r >= last, "realny cas sel zpet v kroku {k}");
        // Rychlost nejvys o 500 ppm mensi nez monotonni.
        let dm = dur / 1000;
        assert!(r - last >= dm - dm * MAX_SLEW_PPB as i128 / S - 1, "zpomaleni nad mez");
        last = r;
    }
    // Po dorovnani: presne o 1 s pozadu proti pruchodu bez korekce.
    let end = 10 * S + dur * 2;
    assert_eq!(t.real_at(end).as_nanos(), t0.add_ns(end).as_nanos() - S);
    assert_eq!(t.remaining(end), 0);
    // Sousedni nanosekundy v miste nejvetsiho zaokrouhleni neklesaji.
    for m in [10 * S + 1, 10 * S + 2, 10 * S + 999_999, 10 * S + 1_000_001] {
        assert!(t.real_at(m + 1).as_nanos() >= t.real_at(m).as_nanos());
    }
    // Frekvence: +100 ppm = o 100 us za sekundu vic.
    let mut f = Timeline::new(0, t0);
    f.adjust(0, None, Some(100_000));
    assert_eq!(f.real_at(S).as_nanos() - t0.as_nanos(), S + 100_000);
    // Novy posun nahradi zbytek starsiho, skok zrusi dorovnavani a necha frekvenci.
    f.adjust(S, Some(5_000_000), None);
    assert_eq!(f.status(S).remaining_ns, 5_000_000);
    assert_eq!(f.status(S).slew_ppb, MAX_SLEW_PPB);
    f.adjust(S + S / 100, Some(1_000), None);
    assert_eq!(f.status(S + S / 100).remaining_ns, 1_000);
    let real = WallTime::new(2_000_000, 0);
    f.step(3 * S, real);
    assert_eq!(f.real_at(3 * S), real);
    assert_eq!(f.status(3 * S).remaining_ns, 0);
    assert_eq!(f.status(3 * S).freq_ppb, 100_000);
}

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
