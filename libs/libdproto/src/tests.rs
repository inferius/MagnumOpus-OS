//! Testy protokolu libd na hostiteli: kontrola jmen a dekodovani
//! neduveryhodneho vstupu (vcetne fuzzu bez paniky).

extern crate std;

use alloc::string::String;
use alloc::vec;

use super::*;

#[test]
fn names() {
    for ok in ["libavcodec.so.61", "ld.so", "libc++.so.1", "lib_x-y.so"] {
        assert!(valid_name(ok), "{ok}");
    }
    let long = "a".repeat(MAX_NAME + 1);
    for bad in ["", ".hidden", "..", "a/b", "/system/lib/x", "lib\0x", "libx\n", "lib x", "libé.so", long.as_str()] {
        assert!(!valid_name(bad), "{bad:?}");
    }
    assert!(valid_name(&"a".repeat(MAX_NAME)));
}

#[test]
fn request_roundtrip_and_rejects() {
    for r in [Request::Open { name: String::from("libdltest.so.1") }, Request::Connect, Request::Stats] {
        assert_eq!(Request::decode(&r.encode()), Some(r));
    }
    // Jmeno s lomitkem se nedekoduje ani kdyz ho klient posle.
    let mut b = vec![1u8];
    b.extend_from_slice(&5u16.to_le_bytes());
    b.extend_from_slice(b"../xy");
    assert_eq!(Request::decode(&b), None);
    // Delka nesedi, prebytecne bajty, neznamy druh.
    assert_eq!(Request::decode(&[1, 10, 0, b'a']), None);
    assert_eq!(Request::decode(&[2, 0]), None);
    assert_eq!(Request::decode(&[9]), None);
    assert_eq!(Request::decode(&[]), None);
}

#[test]
fn reply_roundtrip_and_limits() {
    let lib = Library {
        span: 0x40000,
        segments: vec![
            Segment { start: 0, len: 0x10000, prot: 1, kind: kind::SHARED },
            Segment { start: 0x20000, len: 0x10000, prot: 3, kind: kind::DATA },
        ],
    };
    for r in [Reply::Library(lib.clone()), Reply::Ok, Reply::Stats(Stats { templates: 2, opens: 5, hits: 3, connections: 4 }), Reply::Error(-8)] {
        assert_eq!(Reply::decode(&r.encode()), Some(r));
    }
    // Segment za koncem rozsahu, neznamy druh, zadny segment.
    let mut bad = lib.clone();
    bad.segments[1].len = 0x30000;
    assert_eq!(Reply::decode(&Reply::Library(bad).encode()), None);
    let mut bad = lib.clone();
    bad.segments[0].kind = 7;
    assert_eq!(Reply::decode(&Reply::Library(bad).encode()), None);
    assert_eq!(Reply::decode(&Reply::Library(Library { span: 1, segments: vec![] }).encode()), None);
}

#[test]
fn fuzz_decode_never_panics() {
    let mut x = 0x9E37_79B9_7F4A_7C15u64;
    let mut next = || {
        x = x.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        (x >> 33) as u8
    };
    let seeds = [
        Request::Open { name: String::from("libdltest.so.1") }.encode(),
        Reply::Library(Library { span: 0x20000, segments: vec![Segment { start: 0, len: 0x10000, prot: 5, kind: 0 }] }).encode(),
        Reply::Stats(Stats::default()).encode(),
    ];
    for _ in 0..20_000 {
        let mut b = seeds[next() as usize % seeds.len()].clone();
        let n = 1 + next() as usize % 4;
        for _ in 0..n {
            let i = next() as usize % (b.len() + 1);
            match next() % 3 {
                0 if i < b.len() => b[i] = next(),
                1 => b.insert(i, next()),
                _ if i < b.len() => {
                    b.remove(i);
                }
                _ => {}
            }
        }
        let _ = Request::decode(&b);
        if let Some(Request::Open { name }) = Request::decode(&b) {
            assert!(valid_name(&name));
        }
        let _ = Reply::decode(&b);
    }
}
