//! Testy sirokych znaku ve formatovani (`%lc %ls`, `wprintf`).

use alloc::string::String;
use alloc::vec::Vec;

use super::{A, List};
use crate::format::{Buf, Eilseq, format_enc, wformat};
use crate::mb::Enc;

/// Bajtove formatovani s kodovanim `enc` (chyba = EILSEQ).
fn fe(enc: Enc, fmt: &str, args: &[A]) -> Result<Vec<u8>, Eilseq> {
    let mut buf = [0u8; 512];
    let mut out = Buf { buf: &mut buf, pos: 0, total: 0 };
    let mut list = List(args.to_vec(), 0);
    format_enc(&mut out, fmt.as_bytes(), &mut list, enc)?;
    let n = out.pos;
    Ok(buf[..n].to_vec())
}

/// Siroke formatovani (`wprintf`), vystup jako siroke znaky.
fn fw_raw(enc: Enc, fmt: &str, args: &[A]) -> Result<Vec<u32>, Eilseq> {
    let wfmt: Vec<u32> = fmt.chars().map(|c| c as u32).collect();
    let mut buf = [0u32; 512];
    let mut out = Buf { buf: &mut buf, pos: 0, total: 0 };
    let mut list = List(args.to_vec(), 0);
    wformat(&mut out, &wfmt, &mut list, enc)?;
    assert_eq!(list.1, args.len(), "nespotrebovane argumenty u {fmt}");
    let n = out.pos;
    Ok(buf[..n].to_vec())
}

fn fw(enc: Enc, fmt: &str, args: &[A]) -> Result<String, Eilseq> {
    fw_raw(enc, fmt, args).map(|v| v.iter().map(|&c| char::from_u32(c).unwrap()).collect())
}

fn w(s: &str) -> Vec<u32> {
    s.chars().map(|c| c as u32).chain([0]).collect()
}

#[test]
fn wide_args_in_byte_output() {
    let s = w("čaj");
    assert_eq!(fe(Enc::Utf8, "[%ls]", &[A::W(Some(&s))]).unwrap(), "[čaj]".as_bytes());
    // Presnost v bajtech, nikdy pul znaku: "č" ma 2 bajty.
    assert_eq!(fe(Enc::Utf8, "[%.1ls]", &[A::W(Some(&s))]).unwrap(), b"[]");
    assert_eq!(fe(Enc::Utf8, "[%.3ls]", &[A::W(Some(&s))]).unwrap(), "[ča]".as_bytes());
    assert_eq!(fe(Enc::Utf8, "[%5ls]", &[A::W(Some(&s))]).unwrap(), "[ čaj]".as_bytes(), "sirka v bajtech");
    assert_eq!(fe(Enc::Utf8, "%lc", &[A::I(0x20AC)]).unwrap(), "€".as_bytes());
    assert_eq!(fe(Enc::Utf8, "%ls", &[A::W(None)]).unwrap(), b"(null)");
    assert_eq!(fe(Enc::Utf8, "%S", &[A::W(Some(&w("ab")))]).unwrap(), b"ab");
    // Locale C: jen ASCII a bajty U+DF80..U+DFFF.
    assert_eq!(fe(Enc::C, "%ls", &[A::W(Some(&s))]), Err(Eilseq));
    assert_eq!(fe(Enc::C, "%lc", &[A::I(0xDFE9)]).unwrap(), [0xE9]);
}

#[test]
fn wide_output() {
    assert_eq!(fw(Enc::Utf8, "%d|%5.2f|%x", &[A::I(-42), A::D(3.14159), A::I(255)]).unwrap(), "-42| 3.14|ff");
    assert_eq!(fw(Enc::Utf8, "ž%sž", &[A::S(Some("čaj"))]).unwrap(), "žčajž");
    assert_eq!(fw(Enc::Utf8, "[%.2s]", &[A::S(Some("čaj"))]).unwrap(), "[ča]", "presnost v sirokych znacich");
    assert_eq!(fw(Enc::Utf8, "[%4s]", &[A::S(Some("čaj"))]).unwrap(), "[ čaj]");
    assert_eq!(fw(Enc::Utf8, "[%-4ls]", &[A::W(Some(&w("čaj")))]).unwrap(), "[čaj ]");
    assert_eq!(fw(Enc::Utf8, "%c%lc", &[A::I(b'a' as i64), A::I(0x1F600)]).unwrap(), "a😀");
    assert_eq!(fw(Enc::Utf8, "%q%%", &[]).unwrap(), "%q%");
    // Neplatne UTF-8 v %s.
    assert_eq!(fw(Enc::Utf8, "%s", &[A::B(b"a\xffb")]), Err(Eilseq));
    assert_eq!(fw(Enc::Utf8, "%s", &[A::B(b"a\xc3")]), Err(Eilseq), "retezec konci uprostred znaku");
    // Locale C: bajt 0xFF -> U+DFFF (vratne).
    assert_eq!(fw_raw(Enc::C, "%s", &[A::B(b"a\xffb")]).unwrap(), [0x61, 0xDFFF, 0x62]);
}
