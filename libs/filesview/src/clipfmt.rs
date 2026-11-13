//! Formaty schranky pro seznam cest (mezi okny a aplikacemi):
//!
//! - [`PATHS`] `application/x-mo-paths`: prvni radek `copy` nebo `move`
//!   (Kopirovat / Vyjmout), pak jedna absolutni cesta na radek. Nese
//!   i operaci, proto se pri vkladani bere prvni.
//! - [`URI_LIST`] `text/uri-list` (RFC 2483): `file:///cesta` s procentnim
//!   kodovanim, radky CRLF; komentare `#` se ignoruji. Vzdy kopie.
//! - text (`clipproto::TEXT`): cesty po radcich (vlozeni do terminalu,
//!   editoru).
//!
//! Kazdy format nejvys `clipproto::MAX_INLINE`; vic cest se do schranky
//! nevejde (rozhrani to rekne).

use alloc::string::String;
use alloc::vec::Vec;

use fileops::job::Op;

pub const PATHS: &str = "application/x-mo-paths";
pub const URI_LIST: &str = "text/uri-list";
/// Formaty, ktere spravce umi vlozit (v poradi preference).
pub const PASTE_FORMATS: [&str; 2] = [PATHS, URI_LIST];

/// Data pro `Effect::Copy`; `None` = nevejde se do schranky.
pub fn encode(op: Op, paths: &[String]) -> Option<Vec<(String, Vec<u8>)>> {
    let mut own = String::from(if op == Op::Move { "move" } else { "copy" });
    let mut uris = String::new();
    let mut text = String::new();
    for p in paths {
        own.push('\n');
        own.push_str(p);
        uris.push_str("file://");
        uris.push_str(&percent(p));
        uris.push_str("\r\n");
        if !text.is_empty() {
            text.push('\n');
        }
        text.push_str(p);
    }
    let max = clipproto::MAX_INLINE;
    if own.len() > max || uris.len() > max || text.len() > max {
        return None;
    }
    Some(alloc::vec![(String::from(PATHS), own.into_bytes()), (String::from(URI_LIST), uris.into_bytes()), (String::from(clipproto::TEXT), text.into_bytes())])
}

/// Vlozena data: operace a absolutni cesty (`None` = nic pouzitelneho).
pub fn decode(format: &str, data: &[u8]) -> Option<(Op, Vec<String>)> {
    let s = core::str::from_utf8(data).ok()?;
    let (op, paths): (Op, Vec<String>) = match format {
        PATHS => {
            let mut lines = s.lines();
            let op = match lines.next()? {
                "copy" => Op::Copy,
                "move" => Op::Move,
                _ => return None,
            };
            (op, lines.map(String::from).collect())
        }
        URI_LIST => (Op::Copy, s.lines().map(|l| l.trim_end_matches('\r')).filter(|l| !l.is_empty() && !l.starts_with('#')).filter_map(|l| unpercent(l.strip_prefix("file://")?)).collect()),
        _ => return None,
    };
    let paths: Vec<String> = paths.into_iter().filter(|p| p.starts_with('/') && !p.contains('\0')).collect();
    (!paths.is_empty()).then_some((op, paths))
}

fn percent(p: &str) -> String {
    let mut out = String::new();
    for b in p.bytes() {
        if b.is_ascii_alphanumeric() || b"/-._~".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&alloc::format!("%{b:02X}"));
        }
    }
    out
}

fn unpercent(s: &str) -> Option<String> {
    // `file://host/cesta`: jen mistni (prazdny host nebo localhost).
    let s = s.strip_prefix("localhost").unwrap_or(s);
    if !s.starts_with('/') {
        return None;
    }
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' {
            let h = core::str::from_utf8(b.get(i + 1..i + 3)?).ok()?;
            out.push(u8::from_str_radix(h, 16).ok()?);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}
