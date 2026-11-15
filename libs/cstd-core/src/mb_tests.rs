//! Testy vicebajtoveho kodovani (UTF-8 a locale C).

use crate::mb::{Enc, MB_ERROR, MB_INCOMPLETE, MbState};

fn all(enc: Enc, s: &[u8]) -> Option<alloc::vec::Vec<u32>> {
    let mut st = MbState::INITIAL;
    let mut out = alloc::vec::Vec::new();
    let mut i = 0;
    while i < s.len() {
        let (c, n) = enc.mbrtowc(&s[i..], &mut st);
        if n == MB_ERROR || n == MB_INCOMPLETE {
            return None;
        }
        out.push(c.unwrap());
        i += n.max(1);
    }
    Some(out)
}

#[test]
fn utf8_valid_and_invalid() {
    assert_eq!(all(Enc::Utf8, "aé€😀".as_bytes()), Some(vec![0x61, 0xE9, 0x20AC, 0x1F600]));
    for bad in [&b"\xC0\x80"[..], b"\xE0\x80\x80", b"\xED\xA0\x80", b"\xF4\x90\x80\x80", b"\xF5\x80\x80\x80", b"\x80", b"\xFF"] {
        assert_eq!(all(Enc::Utf8, bad), None, "{bad:x?}");
    }
    assert_eq!(all(Enc::Utf8, b"\xEF\xBF\xBF"), Some(vec![0xFFFF]));
    assert_eq!(all(Enc::Utf8, b"\xF4\x8F\xBF\xBF"), Some(vec![0x10FFFF]));
}

#[test]
fn utf8_split_across_calls() {
    let mut st = MbState::INITIAL;
    let e = "€".as_bytes();
    assert_eq!(Enc::Utf8.mbrtowc(&e[..1], &mut st), (None, MB_INCOMPLETE));
    assert!(!st.is_initial());
    assert_eq!(Enc::Utf8.mbrtowc(&e[1..2], &mut st), (None, MB_INCOMPLETE));
    assert_eq!(Enc::Utf8.mbrtowc(&e[2..], &mut st), (Some(0x20AC), 1));
    assert!(st.is_initial());
    // NUL vraci 0.
    assert_eq!(Enc::Utf8.mbrtowc(b"\0x", &mut st), (Some(0), 0));
}

#[test]
fn c_locale_is_byte_transparent() {
    let bytes: alloc::vec::Vec<u8> = (1..=255u8).collect();
    let wide = all(Enc::C, &bytes).unwrap();
    assert_eq!(wide[0x41 - 1], 0x41);
    assert_eq!(wide[0xE9 - 1], 0xDFE9);
    let mut buf = [0u8; 4];
    for (&b, &wc) in bytes.iter().zip(&wide) {
        assert_eq!(Enc::C.encode(wc, &mut buf), Some(1));
        assert_eq!(buf[0], b);
    }
    assert_eq!(Enc::C.encode(0xE9, &mut buf), None, "U+00E9 v locale C nejde");
    assert_eq!(Enc::Utf8.encode(0xD800, &mut buf), None);
    assert_eq!(Enc::Utf8.encode(0x110000, &mut buf), None);
    assert_eq!(Enc::C.max_len(), 1);
    assert_eq!(Enc::Utf8.max_len(), 4);
}
