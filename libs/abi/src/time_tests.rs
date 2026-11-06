//! Testy prevodu realneho casu na kalendar.

use crate::time::{Civil, Reading, WallTime, civil_from_days, days_from_civil, source};

fn civil(year: i64, month: u8, day: u8, hour: u8, minute: u8, second: u8) -> Civil {
    Civil { year, month, day, hour, minute, second, nanos: 0 }
}

#[test]
fn known_dates() {
    assert_eq!(days_from_civil(1970, 1, 1), Some(0));
    assert_eq!(days_from_civil(2000, 3, 1), Some(11_017));
    assert_eq!(days_from_civil(1969, 12, 31), Some(-1));
    // NTFS epocha (FILETIME 0).
    assert_eq!(days_from_civil(1601, 1, 1), Some(-134_774));
    let t = WallTime::from_civil(&civil(2026, 11, 7, 12, 30, 15)).unwrap();
    assert_eq!(t.secs, 1_794_054_615);
    assert_eq!(t.to_civil(), civil(2026, 11, 7, 12, 30, 15));
}

#[test]
fn before_1970_and_far_future() {
    let t = WallTime::from_civil(&civil(1900, 2, 28, 23, 59, 59)).unwrap();
    assert!(t.secs < 0);
    assert_eq!(t.to_civil(), civil(1900, 2, 28, 23, 59, 59));
    // 1900 neni prestupny, 2000 ano.
    assert!(WallTime::from_civil(&civil(1900, 2, 29, 0, 0, 0)).is_none());
    assert!(WallTime::from_civil(&civil(2000, 2, 29, 0, 0, 0)).is_some());
    // Daleko za rokem 2262 (hranice u64 ns) i pred naším letopočtem.
    for y in [-4713, 0, 1, 2262, 2263, 10_000, 1_000_000_000] {
        let c = civil(y, 6, 15, 1, 2, 3);
        assert_eq!(WallTime::from_civil(&c).unwrap().to_civil(), c, "rok {y}");
    }
}

#[test]
fn roundtrip_every_day_over_centuries() {
    for d in (-800_000..800_000).step_by(7) {
        let (y, m, day) = civil_from_days(d);
        assert_eq!(days_from_civil(y, m, day), Some(d));
    }
    // Krajni hodnoty se neprelijou.
    let _ = civil_from_days(i64::MAX / 86_400);
    let _ = civil_from_days(i64::MIN / 86_400);
}

#[test]
fn normalization_and_offsets() {
    assert_eq!(WallTime::new(5, -1), WallTime { secs: 4, nanos: 999_999_999 });
    assert_eq!(WallTime::new(-1, 2_500_000_000), WallTime { secs: 1, nanos: 500_000_000 });
    let t = WallTime { secs: -1, nanos: 900_000_000 }.add_ns(200_000_000);
    assert_eq!(t, WallTime { secs: 0, nanos: 100_000_000 });
    assert_eq!(WallTime { secs: i64::MAX, nanos: 0 }.add_ns(i128::MAX / 2).secs, i64::MAX);
    assert_eq!(WallTime { secs: -2, nanos: 5 }.as_nanos(), -1_999_999_995);
}

#[test]
fn reading_encoding() {
    let r = Reading {
        time: WallTime { secs: -12_345, nanos: 678 },
        source: source::UEFI,
        flags: 1,
    };
    assert_eq!(Reading::decode(&r.encode()), r);
}
