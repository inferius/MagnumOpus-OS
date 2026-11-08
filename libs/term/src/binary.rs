//! Binarni data v okne: misto celeho obsahu souhrn (velikost, druh podle
//! obsahu, nahled par radku hexa).
//!
//! Zprava kanalu ma nejvys 64 KiB; `open obrazek.bmp` (megabajty) by se
//! oknu nedala poslat a relace by spojeni ztratila. Okno navic obsah
//! binarniho souboru nepotrebuje - jen vedet, co to je.

use alloc::string::String;
use alloc::vec::Vec;

use shlang::value::Value;

/// Bajty do teto velikosti jdou oknu cele (kratke hodnoty, `<12 B: "ahoj">`).
pub const INLINE_MAX: usize = 1024;
/// Kolik radku hexa ukazat v nahledu (po 16 bajtech).
const PREVIEW_ROWS: usize = 4;

fn u16le(b: &[u8], i: usize) -> Option<u32> {
    Some(u16::from_le_bytes(b.get(i..i + 2)?.try_into().ok()?) as u32)
}

fn u32le(b: &[u8], i: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(i..i + 4)?.try_into().ok()?))
}

fn u32be(b: &[u8], i: usize) -> Option<u32> {
    Some(u32::from_be_bytes(b.get(i..i + 4)?.try_into().ok()?))
}

/// Druh dat podle obsahu (hlavicky), lidsky: "obrázek BMP 1920×1080".
pub fn kind(b: &[u8]) -> String {
    let dims = |what: &str, w: Option<u32>, h: Option<u32>| match (w, h) {
        (Some(w), Some(h)) => alloc::format!("{what} {w}×{h}"),
        _ => String::from(what),
    };
    if b.starts_with(b"BM") {
        // Vyska muze byt zaporna (radky shora dolu).
        let h = u32le(b, 22).map(|h| (h as i32).unsigned_abs());
        return dims("obrázek BMP", u32le(b, 18), h);
    }
    if b.starts_with(b"\x89PNG\r\n\x1a\n") {
        return dims("obrázek PNG", u32be(b, 16), u32be(b, 20));
    }
    if b.starts_with(b"GIF8") {
        return dims("obrázek GIF", u16le(b, 6), u16le(b, 8));
    }
    if b.starts_with(&[0xFF, 0xD8, 0xFF]) {
        return "obrázek JPEG".into();
    }
    if b.starts_with(b"RIFF") && b.get(8..12) == Some(b"WAVE") {
        return "zvuk WAV".into();
    }
    if b.starts_with(b"%PDF") {
        return "dokument PDF".into();
    }
    if b.starts_with(b"PK\x03\x04") {
        return "archiv ZIP".into();
    }
    if b.starts_with(b"\x7fELF") {
        return "program ELF".into();
    }
    match core::str::from_utf8(b) {
        Ok(s) => alloc::format!("text UTF-8, {} řádků (převod: from text)", s.lines().count()),
        Err(_) => "binární data".into(),
    }
}

/// Nahled: prvnich par radku hexa s textem (`00000000  42 4d ...  BM..`).
pub fn hex_preview(b: &[u8]) -> String {
    let mut out = String::new();
    for (row, chunk) in b.chunks(16).take(PREVIEW_ROWS).enumerate() {
        out.push_str(&alloc::format!("{:08x} ", row * 16));
        for i in 0..16 {
            match chunk.get(i) {
                Some(x) => out.push_str(&alloc::format!(" {x:02x}")),
                None => out.push_str("   "),
            }
        }
        out.push_str("  ");
        out.extend(chunk.iter().map(|&c| if (0x20..0x7f).contains(&c) { c as char } else { '.' }));
        out.push('\n');
    }
    out
}

/// Hodnota pro okno: velka binarni data jako souhrn (rekurzivne v seznamech
/// a zaznamech); ostatni beze zmeny.
pub fn for_window(v: Value) -> Value {
    match v {
        Value::Bytes(b) if b.len() > INLINE_MAX => Value::record(&[
            ("data", Value::str(&kind(&b))),
            ("size", Value::Size(b.len() as i64)),
            ("preview", Value::str(&hex_preview(&b))),
        ]),
        Value::List(items) => Value::List(items.into_iter().map(for_window).collect()),
        Value::Record(fields) => Value::Record(fields.into_iter().map(|(k, x)| (k, for_window(x))).collect::<Vec<_>>()),
        other => other,
    }
}
