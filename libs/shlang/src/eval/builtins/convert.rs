//! `convert`: prevody jednotek, ciselnych soustav, kodovani textu
//! a casovych pasem. Formaty dat (json, text, radky) jsou `from` / `to`.
//!
//! - jednotky z tabulky `crate::units`: `1GiB | convert MB` = 1073.741824
//!   (cislo v cilove jednotce); cislo s `--from jednotka`: `5 | convert MiB
//!   --from GB`
//! - soustavy `hex`, `bin`, `oct`, `dec`: cele cislo -> text s predponou
//!   (`0xff`), text s cislem (`"0xff"`, `"ff" --from hex`) -> cislo nebo text
//! - kodovani `utf8`, `utf16le`, `utf16be`, `latin2`, `base64`, `hex`: text
//!   -> bajty (`base64`, `hex` z bajtu -> text); zpet `--from kodovani`
//! - casova pasma (`libs/tz`, jmena IANA, `UTC+2`, `+05:30`): cas -> mistni
//!   cas jako text (`2026-11-13 18:45:12 CET`)

use alloc::string::String;
use alloc::vec::Vec;

use super::super::call::Args;
use super::super::{Interp, R};
use crate::error::code;
use crate::host::Host;
use crate::units::{Dim, Unit};
use crate::value::Value;

const BASES: &[(&str, u32, &str)] = &[("hex", 16, "0x"), ("bin", 2, "0b"), ("oct", 8, "0o"), ("dec", 10, "")];
const ENCODINGS: &[&str] = &["utf8", "utf16le", "utf16be", "latin2", "base64", "hex"];

pub(crate) fn call(_it: &mut Interp, _host: &mut dyn Host, a: Args, input: Value) -> R<Value> {
    let target = match a.pos(0) {
        Some(Value::Str(s)) => Some(s.clone()),
        Some(v) => Some(v.to_text()),
        None => None,
    };
    let from = a.named("from").map(|v| v.to_text());
    let t = target.as_deref();
    let f = from.as_deref();
    // Casova pasma: vstup je okamzik.
    if let (Value::Time(ns), Some(zone)) = (&input, t) {
        return to_zone(&a, *ns, zone);
    }
    // Jednotky.
    if let Some(u) = t.and_then(Unit::parse) {
        return to_unit(&a, input, u, f);
    }
    // Ciselne soustavy (`hex` u textu, ktery neni cislo, je kodovani).
    if let Some(&(_, radix, prefix)) = t.and_then(|t| BASES.iter().find(|(n, ..)| *n == t)) {
        let number = match (&input, f) {
            (Value::Int(i), None) => Some(*i),
            (Value::Str(s), _) => parse_int(&a, s, f)?,
            _ => None,
        };
        if let Some(n) = number {
            return Ok(if radix == 10 { Value::Int(n) } else { Value::Str(format_int(n, radix, prefix)) });
        }
    }
    // Kodovani.
    if let Some(enc) = f.filter(|f| ENCODINGS.contains(f)) {
        if t.is_some() {
            return Err(a.err(code::ARGS, "s --from kódováním se cíl neuvádí (výsledek jsou bajty nebo text)"));
        }
        return decode(&a, input, enc);
    }
    if let Some(enc) = t.filter(|t| ENCODINGS.contains(t)) {
        return encode(&a, input, enc);
    }
    Err(a.err(
        code::ARGS,
        match t {
            Some(t) => alloc::format!("neznámý cíl převodu '{t}' pro {}", input.type_name()),
            None => String::from("chybí cíl převodu"),
        },
    )
    .map_hint(hint()))
}

trait MapHint {
    fn map_hint(self, h: String) -> Self;
}

impl MapHint for super::super::Flow {
    fn map_hint(self, h: String) -> Self {
        match self {
            super::super::Flow::Err(e) => super::super::Flow::Err(e.hint(h)),
            other => other,
        }
    }
}

fn hint() -> String {
    alloc::format!(
        "jednotky ({}), soustavy hex bin oct dec, kódování {}, časové pásmo (Europe/Prague, UTC+2) u času",
        crate::units::hint(),
        ENCODINGS.join(" ")
    )
}

/// Hodnota v jednotce `u`: velicina stejneho rozmeru -> cislo v `u`;
/// cislo s `--from jednotka` -> velicina prevedena do `u`.
fn to_unit(a: &Args, input: Value, u: Unit, from: Option<&str>) -> R<Value> {
    let (dim, base) = match (&input, from) {
        (Value::Size(b), None) => (Dim::Size, *b as f64),
        (Value::Duration(d), None) => (Dim::Duration, *d as f64),
        (Value::Quantity(q), None) => (q.dim, q.value),
        (Value::Int(_) | Value::Float(_), Some(fu)) => {
            let fu = Unit::parse(fu).ok_or_else(|| a.err(code::ARGS, alloc::format!("--from: neznámá jednotka '{fu}'")))?;
            let x = match input {
                Value::Int(i) => i as f64,
                Value::Float(x) => x,
                _ => unreachable!(),
            };
            (fu.dim(), x * fu.scale())
        }
        (Value::Int(_) | Value::Float(_), None) => {
            return Err(a.err(code::TYPE, "číslo nemá jednotku").map_hint(alloc::format!(
                "napište jednotku k číslu (5GB | convert MiB) nebo --from: 5 | convert {} --from ...",
                u.name()
            )));
        }
        _ => return Err(a.err(code::TYPE, alloc::format!("{} nejde převést na {}", input.type_name(), u.name()))),
    };
    if dim != u.dim() {
        return Err(a.err(
            code::TYPE,
            alloc::format!("{} ({}) nejde převést na {} ({})", input.type_name(), dim.name_cs(), u.name(), u.dim().name_cs()),
        ));
    }
    Ok(Value::Float(base / u.scale()))
}

/// Cele cislo z textu: predpona `0x`, `0b`, `0o` nebo soustava z `--from`.
fn parse_int(a: &Args, s: &str, from: Option<&str>) -> R<Option<i64>> {
    let t = s.trim().replace('_', "");
    let (neg, t) = match t.strip_prefix('-') {
        Some(r) => (true, String::from(r)),
        None => (false, t),
    };
    let (radix, digits) = match from {
        Some(f) => match BASES.iter().find(|(n, ..)| *n == f) {
            Some(&(_, r, p)) => (r, t.strip_prefix(p).map(String::from).unwrap_or(t.clone())),
            None => return Ok(None),
        },
        None => {
            let lower = t.to_lowercase();
            match BASES.iter().find(|(_, r, p)| *r != 10 && lower.starts_with(p)) {
                Some(&(_, r, p)) => (r, String::from(&t[p.len()..])),
                None if !t.is_empty() && t.bytes().all(|b| b.is_ascii_digit()) => (10, t.clone()),
                None => return Ok(None),
            }
        }
    };
    let v = i64::from_str_radix(&digits, radix)
        .map_err(|_| a.err(code::ARGS, alloc::format!("'{s}' není číslo v soustavě o základu {radix}")))?;
    Ok(Some(if neg { -v } else { v }))
}

fn format_int(n: i64, radix: u32, prefix: &str) -> String {
    let sign = if n < 0 { "-" } else { "" };
    let m = n.unsigned_abs();
    let body = match radix {
        16 => alloc::format!("{m:x}"),
        2 => alloc::format!("{m:b}"),
        8 => alloc::format!("{m:o}"),
        _ => alloc::format!("{m}"),
    };
    alloc::format!("{sign}{prefix}{body}")
}

/// Mistni cas v pasmu `zone` jako text (zkratka pasma nebo posun).
fn to_zone(a: &Args, ns: i64, zone: &str) -> R<Value> {
    let z = tz::Zone::parse(zone).ok_or_else(|| {
        a.err(code::ARGS, alloc::format!("neznámé časové pásmo '{zone}'"))
            .map_hint(String::from("jméno IANA (Europe/Prague, America/New_York) nebo posun UTC+2, +05:30"))
    })?;
    let t = tz::WallTime::new(ns.div_euclid(1_000_000_000), ns.rem_euclid(1_000_000_000));
    let l = z.to_local(t);
    let c = l.civil;
    let mark = if l.offset.abbrev.is_empty() {
        alloc::format!("{}", tz::FormatOffset(l.offset.secs))
    } else {
        String::from(l.offset.abbrev)
    };
    Ok(Value::Str(alloc::format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02} {mark}",
        c.year, c.month, c.day, c.hour, c.minute, c.second
    )))
}

/// Text (nebo bajty) v kodovani `enc`.
fn encode(a: &Args, input: Value, enc: &str) -> R<Value> {
    let text_in = |v: &Value| -> R<String> {
        match v {
            Value::Str(s) => Ok(s.clone()),
            other => Err(a.err(code::TYPE, alloc::format!("{enc} převádí text, dostal {}", other.type_name()))),
        }
    };
    Ok(match enc {
        "utf8" => Value::Bytes(text_in(&input)?.into_bytes()),
        "utf16le" | "utf16be" => {
            let s = text_in(&input)?;
            let mut out = Vec::new();
            for u in s.encode_utf16() {
                let b = if enc == "utf16le" { u.to_le_bytes() } else { u.to_be_bytes() };
                out.extend_from_slice(&b);
            }
            Value::Bytes(out)
        }
        "latin2" => {
            let s = text_in(&input)?;
            let mut out = Vec::new();
            for ch in s.chars() {
                let b = latin2_byte(ch).ok_or_else(|| a.err(code::ARGS, alloc::format!("znak '{ch}' v latin2 není")))?;
                out.push(b);
            }
            Value::Bytes(out)
        }
        _ => {
            // base64 a hex: z bajtu (text jako UTF-8) na text.
            let bytes = match input {
                Value::Bytes(b) => b,
                Value::Str(s) => s.into_bytes(),
                other => return Err(a.err(code::TYPE, alloc::format!("{enc} převádí bajty, dostal {}", other.type_name()))),
            };
            Value::Str(if enc == "base64" { base64(&bytes) } else { hex(&bytes) })
        }
    })
}

/// Bajty (nebo text base64/hex) z kodovani `enc` zpet.
fn decode(a: &Args, input: Value, enc: &str) -> R<Value> {
    match (enc, input) {
        ("base64", Value::Str(s)) => unbase64(&s).map(Value::Bytes).ok_or_else(|| a.err(code::ARGS, "text není platné base64")),
        ("hex", Value::Str(s)) => unhex(&s).map(Value::Bytes).ok_or_else(|| a.err(code::ARGS, "text není platné hex (dvojice číslic 0-9 a-f)")),
        ("utf8", Value::Bytes(b)) => String::from_utf8(b).map(Value::Str).map_err(|_| a.err(code::ARGS, "bajty nejsou platné UTF-8")),
        ("utf16le" | "utf16be", Value::Bytes(b)) => {
            if b.len() % 2 != 0 {
                return Err(a.err(code::ARGS, "UTF-16 má sudý počet bajtů"));
            }
            let units = b.chunks(2).map(|c| if enc == "utf16le" { u16::from_le_bytes([c[0], c[1]]) } else { u16::from_be_bytes([c[0], c[1]]) });
            char::decode_utf16(units)
                .collect::<Result<String, _>>()
                .map(Value::Str)
                .map_err(|_| a.err(code::ARGS, "bajty nejsou platné UTF-16"))
        }
        ("latin2", Value::Bytes(b)) => Ok(Value::Str(b.iter().map(|&x| latin2_char(x)).collect())),
        (_, other) => Err(a.err(code::TYPE, alloc::format!("--from {enc} čeká {}, dostal {}", if matches!(enc, "base64" | "hex") { "text" } else { "bajty" }, other.type_name()))),
    }
}

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

fn base64(b: &[u8]) -> String {
    let mut out = String::new();
    for c in b.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        for i in 0..4 {
            if i <= c.len() {
                out.push(B64[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

fn unbase64(s: &str) -> Option<Vec<u8>> {
    let clean: Vec<u8> = s.bytes().filter(|b| !b.is_ascii_whitespace()).collect();
    if clean.len() % 4 != 0 {
        return None;
    }
    let mut out = Vec::new();
    for c in clean.chunks(4) {
        let mut n = 0u32;
        let mut pad = 0;
        for &x in c {
            let v = match x {
                b'=' => {
                    pad += 1;
                    0
                }
                _ if pad > 0 => return None,
                _ => B64.iter().position(|&y| y == x)? as u32,
            };
            n = n << 6 | v;
        }
        let bytes = [(n >> 16) as u8, (n >> 8) as u8, n as u8];
        out.extend_from_slice(&bytes[..3 - pad.min(2)]);
    }
    Some(out)
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| alloc::format!("{x:02x}")).collect()
}

fn unhex(s: &str) -> Option<Vec<u8>> {
    let clean: Vec<u8> = s.bytes().filter(|b| !b.is_ascii_whitespace()).collect();
    if clean.len() % 2 != 0 {
        return None;
    }
    clean.chunks(2).map(|c| u8::from_str_radix(core::str::from_utf8(c).ok()?, 16).ok()).collect()
}

/// ISO-8859-2: znaky 0xA0-0xFF (0x00-0x9F = stejny kod v Unicode).
const LATIN2: [u16; 96] = [
    0x00A0, 0x0104, 0x02D8, 0x0141, 0x00A4, 0x013D, 0x015A, 0x00A7, 0x00A8, 0x0160, 0x015E, 0x0164, 0x0179, 0x00AD, 0x017D, 0x017B,
    0x00B0, 0x0105, 0x02DB, 0x0142, 0x00B4, 0x013E, 0x015B, 0x02C7, 0x00B8, 0x0161, 0x015F, 0x0165, 0x017A, 0x02DD, 0x017E, 0x017C,
    0x0154, 0x00C1, 0x00C2, 0x0102, 0x00C4, 0x0139, 0x0106, 0x00C7, 0x010C, 0x00C9, 0x0118, 0x00CB, 0x011A, 0x00CD, 0x00CE, 0x010E,
    0x0110, 0x0143, 0x0147, 0x00D3, 0x00D4, 0x0150, 0x00D6, 0x00D7, 0x0158, 0x016E, 0x00DA, 0x0170, 0x00DC, 0x00DD, 0x0162, 0x00DF,
    0x0155, 0x00E1, 0x00E2, 0x0103, 0x00E4, 0x013A, 0x0107, 0x00E7, 0x010D, 0x00E9, 0x0119, 0x00EB, 0x011B, 0x00ED, 0x00EE, 0x010F,
    0x0111, 0x0144, 0x0148, 0x00F3, 0x00F4, 0x0151, 0x00F6, 0x00F7, 0x0159, 0x016F, 0x00FA, 0x0171, 0x00FC, 0x00FD, 0x0163, 0x02D9,
];

fn latin2_char(b: u8) -> char {
    if b < 0xA0 { b as char } else { char::from_u32(LATIN2[(b - 0xA0) as usize] as u32).unwrap_or('\u{FFFD}') }
}

fn latin2_byte(ch: char) -> Option<u8> {
    let c = ch as u32;
    if c < 0xA0 {
        return Some(c as u8);
    }
    LATIN2.iter().position(|&x| x as u32 == c).map(|i| 0xA0 + i as u8)
}
