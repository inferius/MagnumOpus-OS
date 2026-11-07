//! Testy pasem: posuny, prechody letniho casu, dvojznacne a chybejici
//! hodiny, nastaveni a oprava RTC v mistnim case.

use abi::time::{Civil, Reading, WallTime, flags, source};

use crate::{FormatOffset, LocalResult, Settings, ZONES, Zone, rtc_local_correction, weekday};

fn utc(year: i64, month: u8, day: u8, hour: u8, minute: u8) -> i64 {
    let c = Civil { year, month, day, hour, minute, second: 0, nanos: 0 };
    WallTime::from_civil(&c).unwrap().secs
}

fn civil(year: i64, month: u8, day: u8, hour: u8, minute: u8) -> Civil {
    Civil { year, month, day, hour, minute, second: 0, nanos: 0 }
}

fn prague() -> Zone {
    Zone::by_name("Europe/Prague").unwrap()
}

#[test]
fn weekdays() {
    assert_eq!(weekday(0), 4); // 1970-01-01 ctvrtek
    assert_eq!(weekday(utc(2026, 11, 7, 0, 0) / 86_400), 6); // sobota
    assert_eq!(weekday(-1), 3);
}

#[test]
fn prague_transitions_known_years() {
    let z = prague();
    // (rok, zacatek, konec) - posledni nedele brezna / rijna, 01:00 UTC.
    for (y, start_day, end_day) in [(2024, 31, 27), (2025, 30, 26), (2026, 29, 25), (2027, 28, 31), (2030, 31, 27)] {
        let (s, e) = z.transitions(y).unwrap();
        assert_eq!(s, utc(y, 3, start_day, 1, 0), "zacatek {y}");
        assert_eq!(e, utc(y, 10, end_day, 1, 0), "konec {y}");
    }
}

#[test]
fn prague_offsets_around_transitions() {
    let z = prague();
    let start = utc(2026, 3, 29, 1, 0);
    let end = utc(2026, 10, 25, 1, 0);
    assert_eq!(z.offset_at(start - 1).secs, 3600);
    assert_eq!(z.offset_at(start - 1).abbrev, "CET");
    assert_eq!(z.offset_at(start).secs, 7200);
    assert!(z.offset_at(start).dst);
    assert_eq!(z.offset_at(start).abbrev, "CEST");
    assert_eq!(z.offset_at(end - 1).secs, 7200);
    assert_eq!(z.offset_at(end).secs, 3600);
    // Leden a cervenec.
    assert_eq!(z.offset_at(utc(2026, 1, 15, 12, 0)).secs, 3600);
    assert_eq!(z.offset_at(utc(2026, 7, 15, 12, 0)).secs, 7200);
}

#[test]
fn prague_to_local() {
    let z = prague();
    let l = z.to_local(WallTime { secs: utc(2026, 7, 1, 22, 30), nanos: 5 });
    assert_eq!((l.civil.year, l.civil.month, l.civil.day, l.civil.hour, l.civil.minute), (2026, 7, 2, 0, 30));
    assert_eq!(l.civil.nanos, 5);
    assert!(l.offset.dst);
    // Silvestr: v UTC jeste stary rok, mistne uz novy.
    let l = z.to_local(WallTime { secs: utc(2026, 12, 31, 23, 30), nanos: 0 });
    assert_eq!((l.civil.year, l.civil.month, l.civil.day, l.civil.hour), (2027, 1, 1, 0));
}

#[test]
fn prague_gap_and_ambiguous_hour() {
    let z = prague();
    // 29. 3. 2026: 02:00 -> 03:00, 02:30 neexistuje.
    let gap = z.local_to_utc(&civil(2026, 3, 29, 2, 30)).unwrap();
    assert_eq!(gap, LocalResult::Gap(utc(2026, 3, 29, 1, 30)));
    assert_eq!(z.local_to_utc(&civil(2026, 3, 29, 1, 59)).unwrap(), LocalResult::Single(utc(2026, 3, 29, 0, 59)));
    assert_eq!(z.local_to_utc(&civil(2026, 3, 29, 3, 0)).unwrap(), LocalResult::Single(utc(2026, 3, 29, 1, 0)));
    // 25. 10. 2026: 03:00 -> 02:00, 02:30 nastane dvakrat.
    let amb = z.local_to_utc(&civil(2026, 10, 25, 2, 30)).unwrap();
    assert_eq!(
        amb,
        LocalResult::Ambiguous { earlier: utc(2026, 10, 25, 0, 30), later: utc(2026, 10, 25, 1, 30) }
    );
    assert_eq!(amb.resolve(), utc(2026, 10, 25, 0, 30));
    assert_eq!(z.local_to_utc(&civil(2026, 10, 25, 3, 0)).unwrap(), LocalResult::Single(utc(2026, 10, 25, 2, 0)));
    assert_eq!(z.local_to_utc(&civil(2026, 2, 30, 0, 0)), None);
}

#[test]
fn local_roundtrip_every_hour_of_year() {
    // Kazda hodina UTC -> mistni -> zpet musi dat puvodni okamzik (i v
    // opakovane hodine: jedna z moznosti je ten okamzik).
    for name in ["Europe/Prague", "America/New_York", "Australia/Sydney", "Pacific/Auckland", "Asia/Kolkata"] {
        let z = Zone::by_name(name).unwrap();
        let mut t = utc(2025, 12, 31, 0, 0);
        while t < utc(2027, 1, 2, 0, 0) {
            let local = z.to_local(WallTime { secs: t, nanos: 0 });
            match z.local_to_utc(&local.civil).unwrap() {
                LocalResult::Single(u) => assert_eq!(u, t, "{name} {t}"),
                LocalResult::Ambiguous { earlier, later } => assert!(earlier == t || later == t, "{name} {t}"),
                LocalResult::Gap(_) => panic!("{name}: existujici cas {t} hlasen jako mezera"),
            }
            t += 1800;
        }
    }
}

#[test]
fn us_rules() {
    let z = Zone::by_name("America/New_York").unwrap();
    // 2026: 8. 3. 02:00 EST = 07:00 UTC; 1. 11. 02:00 EDT = 06:00 UTC.
    assert_eq!(z.transitions(2026), Some((utc(2026, 3, 8, 7, 0), utc(2026, 11, 1, 6, 0))));
    assert_eq!(z.offset_at(utc(2026, 7, 4, 12, 0)).abbrev, "EDT");
    assert_eq!(z.offset_at(utc(2026, 7, 4, 12, 0)).secs, -4 * 3600);
    assert_eq!(z.offset_at(utc(2026, 12, 25, 12, 0)).secs, -5 * 3600);
    // Phoenix letni cas nema.
    let p = Zone::by_name("america/phoenix").unwrap();
    assert_eq!(p.offset_at(utc(2026, 7, 4, 12, 0)).secs, -7 * 3600);
}

#[test]
fn southern_hemisphere() {
    let z = Zone::by_name("Australia/Sydney").unwrap();
    // 2026: konec 5. 4. 03:00 AEDT = 4. 4. 16:00 UTC, zacatek 4. 10.
    // 02:00 AEST = 3. 10. 16:00 UTC.
    assert_eq!(z.transitions(2026), Some((utc(2026, 10, 3, 16, 0), utc(2026, 4, 4, 16, 0))));
    assert_eq!(z.offset_at(utc(2026, 1, 15, 0, 0)).secs, 11 * 3600);
    assert_eq!(z.offset_at(utc(2026, 4, 4, 15, 59)).secs, 11 * 3600);
    assert_eq!(z.offset_at(utc(2026, 4, 4, 16, 0)).secs, 10 * 3600);
    assert_eq!(z.offset_at(utc(2026, 7, 1, 0, 0)).secs, 10 * 3600);
    assert_eq!(z.offset_at(utc(2026, 10, 3, 16, 0)).secs, 11 * 3600);
    assert_eq!(z.offset_at(utc(2026, 12, 31, 23, 0)).secs, 11 * 3600);
}

#[test]
fn fixed_offsets() {
    let z = Zone::parse("UTC+2").unwrap();
    assert_eq!(z.std_offset, 7200);
    assert_eq!(z.offset_at(utc(2026, 7, 1, 0, 0)).secs, 7200);
    assert_eq!(Zone::parse("UTC-03:30").unwrap().std_offset, -(3 * 3600 + 1800));
    assert_eq!(Zone::parse("GMT+05:45").unwrap().std_offset, 5 * 3600 + 45 * 60);
    assert_eq!(Zone::parse("+01:00").unwrap().std_offset, 3600);
    assert_eq!(Zone::parse(" utc ").unwrap(), Zone::UTC);
    for bad in ["UTC+19", "UTC+2:60", "UTC+", "Europe/Atlantis", "UTC+1:2:3", "UTC++1", "2"] {
        assert_eq!(Zone::parse(bad), None, "{bad}");
    }
    assert_eq!(Zone::fixed(18 * 3600 + 1), None);
    assert_eq!(Zone::parse("UTC-3:30").unwrap().local_secs_to_utc(0), LocalResult::Single(3 * 3600 + 1800));
    assert_eq!(format!("{}", Zone::parse("UTC-3:30").unwrap().display()), "UTC-03:30");
    assert_eq!(format!("{}", prague().display()), "Europe/Prague");
    assert_eq!(format!("{}", FormatOffset(19800)), "+05:30");
}

#[test]
fn table_is_consistent() {
    for (i, z) in ZONES.iter().enumerate() {
        assert!(z.std_offset.abs() <= crate::MAX_OFFSET, "{}", z.name);
        assert!(ZONES[..i].iter().all(|o| o.name != z.name), "duplicitni {}", z.name);
        if let Some(d) = z.dst {
            assert!(d.save > 0, "{}", z.name);
            // Prechody musi v kazdem roce existovat a byt ruzne.
            for y in [1970, 2000, 2026, 2100] {
                let (s, e) = z.transitions(y).unwrap();
                assert_ne!(s, e, "{} {y}", z.name);
            }
        }
    }
}

#[test]
fn settings_parse() {
    let s = Settings::parse("# cas\nzone = Europe/Prague\nrtc = local # Windows\n\n").unwrap();
    assert_eq!(s.zone, prague());
    assert!(s.rtc_local);
    assert_eq!(Settings::parse("").unwrap(), Settings::default());
    assert_eq!(Settings::parse("rtc = utc").unwrap().rtc_local, false);
    assert_eq!(Settings::parse("zone=Mars").unwrap_err().what, "nezname pasmo");
    assert_eq!(Settings::parse("\nfoo = 1").unwrap_err().line, 2);
    assert!(Settings::parse("zone").is_err());
    assert!(Settings::parse("rtc = windows").is_err());
}

#[test]
fn rtc_offset_follows_dst() {
    let s = Settings { zone: prague(), rtc_local: true };
    assert_eq!(s.rtc_offset(WallTime { secs: utc(2026, 7, 1, 0, 0), nanos: 0 }), 7200);
    assert_eq!(s.rtc_offset(WallTime { secs: utc(2026, 1, 1, 0, 0), nanos: 0 }), 3600);
    let s = Settings { rtc_local: false, ..s };
    assert_eq!(s.rtc_offset(WallTime { secs: utc(2026, 7, 1, 0, 0), nanos: 0 }), 0);
}

fn reading(secs: i64, source: u16, flags: u16) -> Reading {
    Reading { time: WallTime { secs, nanos: 7 }, source, flags }
}

#[test]
fn rtc_local_boot_correction() {
    let s = Settings { zone: prague(), rtc_local: true };
    // Windows nechal v RTC 14:00 letniho casu; jadro to vzalo jako 14:00 UTC.
    let r = reading(utc(2026, 7, 1, 14, 0), source::RTC, flags::ZONE_ASSUMED_UTC);
    assert_eq!(rtc_local_correction(&s, &r), Some(WallTime { secs: utc(2026, 7, 1, 12, 0), nanos: 7 }));
    // V zime o hodinu.
    let r = reading(utc(2026, 1, 1, 14, 0), source::UEFI, flags::ZONE_ASSUMED_UTC);
    assert_eq!(rtc_local_correction(&s, &r).unwrap().secs, utc(2026, 1, 1, 13, 0));
    // Opakovana hodina: drivejsi (letni cas).
    let r = reading(utc(2026, 10, 25, 2, 30), source::RTC, flags::ZONE_ASSUMED_UTC);
    assert_eq!(rtc_local_correction(&s, &r).unwrap().secs, utc(2026, 10, 25, 0, 30));
    // Bez opravy: RTC v UTC, uz nastaveno za behu, firmware pasmo uvedl.
    let r = reading(utc(2026, 7, 1, 14, 0), source::RTC, flags::ZONE_ASSUMED_UTC);
    assert_eq!(rtc_local_correction(&Settings { rtc_local: false, ..s }, &r), None);
    let r = reading(utc(2026, 7, 1, 14, 0), source::SET, 0);
    assert_eq!(rtc_local_correction(&s, &r), None);
    let r = reading(utc(2026, 7, 1, 14, 0), source::UEFI, 0);
    assert_eq!(rtc_local_correction(&s, &r), None);
}
