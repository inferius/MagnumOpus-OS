//! Binarni data v okne: misto celeho obsahu souhrn (velikost, druh podle
//! obsahu, nahled par radku hexa).
//!
//! Zprava kanalu ma nejvys 64 KiB; `open obrazek.bmp` (megabajty) by se
//! oknu nedala poslat a relace by spojeni ztratila. Okno navic obsah
//! binarniho souboru nepotrebuje - jen vedet, co to je.

use alloc::string::String;
use alloc::vec::Vec;

pub use shlang::filekind::kind as kind_of;

use shlang::value::Value;

/// Bajty do teto velikosti jdou oknu cele (kratke hodnoty, `<12 B: "ahoj">`).
pub const INLINE_MAX: usize = 1024;
/// Kolik radku hexa ukazat v nahledu (po 16 bajtech).
const PREVIEW_ROWS: usize = 4;

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
            ("data", Value::str(&shlang::filekind::kind(&b, true))),
            ("size", Value::Size(b.len() as i64)),
            ("preview", Value::str(&hex_preview(&b))),
        ]),
        // Soubor nacteny jen podle potreby: souhrn bez obsahu.
        Value::File(f) => Value::record(&[("data", Value::str(&f.kind)), ("size", Value::Size(f.size as i64)), ("path", Value::str(&f.path))]),
        Value::List(items) => Value::List(items.into_iter().map(for_window).collect()),
        Value::Record(fields) => Value::Record(fields.into_iter().map(|(k, x)| (k, for_window(x))).collect::<Vec<_>>()),
        other => other,
    }
}
