//! Prevod casu FS <-> Windows FILETIME (100 ns od 1601).

use fs::Timestamp;
use osfs::image::{filetime, from_filetime};

#[test]
fn filetime_roundtrip_and_limits() {
    // 1. 1. 1970 = 116444736000000000 v FILETIME.
    assert_eq!(filetime(Timestamp::UNIX_EPOCH), 116_444_736_000_000_000);
    // Pocatek FILETIME (1601) a cas pred 1970 projdou tam i zpet.
    for t in [
        Timestamp { secs: -11_644_473_600, nanos: 0 },
        Timestamp { secs: -1, nanos: 999_999_900 },
        Timestamp { secs: 1_794_054_615, nanos: 123_456_700 },
        Timestamp { secs: 32_503_680_000, nanos: 100 },
    ] {
        assert_eq!(from_filetime(filetime(t)), t, "{t:?}");
    }
    // Rozliseni FILETIME je 100 ns: zbytek se zahodi.
    let t = Timestamp { secs: 0, nanos: 199 };
    assert_eq!(from_filetime(filetime(t)).nanos, 100);
    // Pred rokem 1601 -> 0, daleko v budoucnosti -> nejvetsi hodnota.
    assert_eq!(filetime(Timestamp { secs: -20_000_000_000, nanos: 0 }), 0);
    assert_eq!(filetime(Timestamp { secs: i64::MAX, nanos: 0 }), i64::MAX as u64);
}
