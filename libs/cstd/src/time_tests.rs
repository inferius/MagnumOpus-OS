//! Testy time.h na hostiteli: kalendar, strftime, pasma z `TZ`.

use crate::time::{civil_secs, format_tm, split, tm, zone_from_tz};

fn utc(secs: i64) -> tm {
    split(secs, 0, false, b"UTC\0".as_ptr()).unwrap()
}

#[test]
fn split_known_dates() {
    // 2026-11-14 12:34:56 UTC, sobota.
    let t = utc(1_794_659_696);
    assert_eq!((t.tm_year, t.tm_mon, t.tm_mday), (126, 10, 14));
    assert_eq!((t.tm_hour, t.tm_min, t.tm_sec), (12, 34, 56));
    assert_eq!(t.tm_wday, 6);
    assert_eq!(t.tm_yday, 317);
    // Pred rokem 1970.
    let t = utc(-1);
    assert_eq!((t.tm_year, t.tm_mon, t.tm_mday, t.tm_hour), (69, 11, 31, 23));
    assert_eq!(t.tm_wday, 3);
}

#[test]
fn civil_secs_normalizes() {
    let mut t = utc(0);
    assert_eq!(civil_secs(&t), Some(0));
    // 32. ledna = 1. unora; mesic 13 = leden pristiho roku.
    t.tm_mday = 32;
    assert_eq!(civil_secs(&t), Some(31 * 86_400));
    let mut t = utc(0);
    t.tm_mon = 12;
    assert_eq!(civil_secs(&t), Some(365 * 86_400));
    let mut t = utc(0);
    t.tm_sec = -1;
    assert_eq!(civil_secs(&t), Some(-1));
}

#[test]
fn strftime_formats() {
    let t = utc(1_794_659_696);
    let f = |s: &[u8]| String::from_utf8(format_tm(s, &t)).unwrap();
    assert_eq!(f(b"%Y-%m-%d %H:%M:%S"), "2026-11-14 12:34:56");
    assert_eq!(f(b"%F %T %z %Z"), "2026-11-14 12:34:56 +0000 UTC");
    assert_eq!(f(b"%a %A %b %B %j %u %w"), "Sat Saturday Nov November 318 6 6");
    assert_eq!(f(b"%I %p %e %y %C %%"), "12 PM 14 26 20 %");
    assert_eq!(f(b"%s"), "1794659696");
    assert_eq!(f(b"%c"), "Sat Nov 14 12:34:56 2026");
    // ISO tyden: 2026-11-14 je v tydnu 46.
    assert_eq!(f(b"%G-W%V"), "2026-W46");
    // 2021-01-01 (patek) patri do 53. tydne roku 2020.
    let n = utc(1_609_459_200);
    assert_eq!(String::from_utf8(format_tm(b"%G-W%V", &n)).unwrap(), "2020-W53");
}

#[test]
fn zones_from_tz() {
    let prague = zone_from_tz(Some("Europe/Prague"));
    // Leden: +1 h, cervenec: +2 h (letni cas).
    assert_eq!(prague.offset_at(1_768_000_000).secs, 3600);
    assert_eq!(prague.offset_at(1_783_000_000).secs, 7200);
    assert_eq!(zone_from_tz(Some(":UTC+5")).offset_at(0).secs, 5 * 3600);
    assert_eq!(zone_from_tz(None).offset_at(0).secs, 0);
    assert_eq!(zone_from_tz(Some("nesmysl")).offset_at(0).secs, 0, "neznamy TZ = UTC");
}
