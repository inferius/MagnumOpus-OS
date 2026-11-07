//! Testy kodovani (na hostiteli: `cargo test -p wire`).

extern crate std;

use abi::Error;

use crate::{Reader, Writer};

#[test]
fn roundtrip() {
    let mut w = Writer::op(7);
    w.u8(1).u16(2).i32(-3).u64(4).str("ahoj").bytes(b"xyz");
    let b = w.finish();
    let mut r = Reader::new(&b);
    assert_eq!(r.u32(), Ok(7));
    assert_eq!(r.u8(), Ok(1));
    assert_eq!(r.u16(), Ok(2));
    assert_eq!(r.i32(), Ok(-3));
    assert_eq!(r.u64(), Ok(4));
    assert_eq!(r.str().as_deref(), Ok("ahoj"));
    assert_eq!(r.rest(), b"xyz");
    assert!(r.is_empty());
}

#[test]
fn short_message_is_error_not_panic() {
    let mut r = Reader::new(&[1, 2, 3]);
    assert_eq!(r.u32(), Err(Error::InvalidArgs));
    // Delka retezce za koncem zpravy.
    let mut r = Reader::new(&[0xFF, 0xFF, b'a']);
    assert_eq!(r.str(), Err(Error::InvalidArgs));
    let mut r = Reader::new(&[]);
    assert_eq!(r.take(usize::MAX), Err(Error::InvalidArgs));
}

#[test]
fn long_string_is_cut_at_char_boundary() {
    let s = "ž".repeat(40_000); // 80 000 B
    let mut w = Writer::new();
    w.str(&s);
    let b = w.finish();
    let back = Reader::new(&b).str().unwrap();
    assert!(back.len() <= u16::MAX as usize);
    assert!(s.starts_with(&back));
}
