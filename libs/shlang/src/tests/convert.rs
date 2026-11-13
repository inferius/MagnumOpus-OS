//! `convert`: jednotky, soustavy, kodovani, casova pasma.

use super::host::{fails, ok};
use crate::error::code;
use crate::value::Value;

fn one(src: &str) -> Value {
    let out = ok(src);
    assert_eq!(out.len(), 1, "{src}: {out:?}");
    out.into_iter().next().unwrap()
}

#[test]
fn units() {
    assert_eq!(one("1GiB | convert MB"), Value::Float(1073.741824));
    assert_eq!(one("90m | convert h"), Value::Float(1.5));
    assert_eq!(one("2.4GHz | convert MHz"), Value::Float(2400.0));
    assert_eq!(one("5 | convert MB --from GB"), Value::Float(5000.0));
    // Ruzne veliciny a cislo bez jednotky.
    assert_eq!(fails("1GB | convert s").code, code::TYPE);
    let e = fails("5 | convert MB");
    assert_eq!(e.code, code::TYPE);
    assert!(e.hint.unwrap().contains("--from"));
}

#[test]
fn number_bases() {
    assert_eq!(one("255 | convert hex"), Value::str("0xff"));
    assert_eq!(one("5 | convert bin"), Value::str("0b101"));
    assert_eq!(one("-8 | convert oct"), Value::str("-0o10"));
    assert_eq!(one("\"0xff\" | convert dec"), Value::Int(255));
    assert_eq!(one("\"ff\" | convert dec --from hex"), Value::Int(255));
    assert_eq!(one("\"0b1010\" | convert hex"), Value::str("0xa"));
    assert_eq!(fails("\"0xzz\" | convert dec").code, code::ARGS);
}

#[test]
fn encodings_roundtrip() {
    let czech = "Příliš žluťoučký kůň";
    for enc in ["utf8", "utf16le", "utf16be", "latin2"] {
        let src = format!("\"{czech}\" | convert {enc} | convert --from {enc}");
        assert_eq!(one(&src), Value::str(czech), "{enc}");
    }
    // Znamy bajt: r s hackem je v latin2 0xF8.
    assert_eq!(one("\"ř\" | convert latin2"), Value::Bytes(vec![0xF8]));
    assert_eq!(one("\"ř\" | convert utf16le"), Value::Bytes(vec![0x59, 0x01]));
    for (text, b64) in [("a", "YQ=="), ("ah", "YWg="), ("aho", "YWhv"), ("ahoj", "YWhvag==")] {
        assert_eq!(one(&format!("\"{text}\" | convert base64")), Value::str(b64));
        assert_eq!(one(&format!("\"{b64}\" | convert --from base64")), Value::Bytes(text.as_bytes().to_vec()));
    }
    assert_eq!(one("\"ahoj\" | convert utf8 | convert hex"), Value::str("61686f6a"));
    assert_eq!(one("\"61686f6a\" | convert --from hex | convert --from utf8"), Value::str("ahoj"));
    assert_eq!(fails("\"€\" | convert latin2").code, code::ARGS);
    assert_eq!(fails("\"YQ=\" | convert --from base64").code, code::ARGS);
}

#[test]
fn time_zones() {
    // Testovaci hostitel zacina v 1 700 000 000 s (2023-11-14 22:13:20 UTC);
    // + 82 907 200 s = 2026-07-01 12:00:00 UTC.
    let summer = "(now) + 82907200s";
    assert_eq!(one(&format!("{summer} | convert Europe/Prague")), Value::str("2026-07-01 14:00:00 CEST"));
    assert_eq!(one(&format!("{summer} | convert UTC")), Value::str("2026-07-01 12:00:00 UTC"));
    assert_eq!(one(&format!("{summer} | convert +05:30")), Value::str("2026-07-01 17:30:00 +05:30"));
    // + 68 478 400 s = 2026-01-15 12:00:00 UTC (zimni cas).
    assert_eq!(one("(now) + 68478400s | convert Europe/Prague"), Value::str("2026-01-15 13:00:00 CET"));
    assert_eq!(fails("now | convert Mars/Olympus").code, code::ARGS);
}
