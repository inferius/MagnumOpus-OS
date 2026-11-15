//! `cargo xtask unicode-tables`: tabulky vlastnosti znaku pro `libs/uniprop`
//! (tridy `wctype.h`, jednoduche prevody velikosti pismen, sirka znaku)
//! z Unicode Character Database.
//!
//! Verze UCD je stejna jako `core::char::UNICODE_VERSION` prekladace, aby
//! C (`iswalpha`) a Rust (`char::is_alphabetic`) odpovidaly stejne. Soubory
//! se stahuji pres HTTPS a kontroluje se jejich SHA-256; vysledek
//! (`libs/uniprop/src/tables.rs`) je v repozitari, generator se pousti jen
//! pri zmene verze Unicode.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::process::Command;

use crate::cports::fetch::sha256;
use crate::{Result, target_dir, workspace_root};

/// Verze UCD (= `core::char::UNICODE_VERSION`, test v `libs/uniprop`).
pub const VERSION: (u32, u32, u32) = (18, 0, 0);

/// Soubory UCD a jejich SHA-256.
const FILES: &[(&str, &str)] = &[
    ("UnicodeData.txt", "0736451de439ae7baf1425136617da495e09ee5afbe6e394374db7009ea08950"),
    ("DerivedCoreProperties.txt", "09c928886a178fcafd93c29e4bd59073a058e5a100b716d425cb563ab50f68c9"),
    ("PropList.txt", "f438f532e8737bb8a2702126cdf9c4af5e357c58c7acf9d9eb2fc7c1a1d955d6"),
    ("EastAsianWidth.txt", "a0cf29eacd00cfcaec4381c6b7c281685f18dbb4e7ff82b4076ccb342ca839aa"),
];

const MAX: u32 = 0x10FFFF;

/// Stazeny a overeny soubor UCD (cache v `target/ucd-<verze>/`).
fn fetch(name: &str, hash: &str) -> Result<String> {
    let (a, b, c) = VERSION;
    let dir = target_dir().join(format!("ucd-{a}.{b}.{c}"));
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(name);
    if let Ok(data) = std::fs::read(&path)
        && sha256(&data) == hash
    {
        return Ok(String::from_utf8(data)?);
    }
    let url = format!("https://www.unicode.org/Public/{a}.{b}.{c}/ucd/{name}");
    println!("xtask: stahuji {url}");
    let tmp = dir.join(format!("{name}.part"));
    let ok = Command::new("curl").args(["-sSfL", "-o"]).arg(&tmp).arg(&url).status()?.success();
    if !ok {
        return Err(format!("stazeni {url} selhalo (curl)").into());
    }
    let data = std::fs::read(&tmp)?;
    let got = sha256(&data);
    if got != hash {
        let _ = std::fs::remove_file(&tmp);
        return Err(format!("{name}: SHA-256 {got} nesedi (ocekavano {hash})").into());
    }
    std::fs::rename(&tmp, &path)?;
    Ok(String::from_utf8(data)?)
}

/// Prikaz `unicode-tables`.
pub fn generate() -> Result<()> {
    let mut texts = Vec::new();
    for (name, hash) in FILES {
        texts.push(fetch(name, hash)?);
    }
    let out = render(&texts[0], &texts[1], &texts[2], &texts[3])?;
    let path = workspace_root().join("libs").join("uniprop").join("src").join("tables.rs");
    std::fs::write(&path, out)?;
    println!("xtask: zapsano {}", path.display());
    Ok(())
}

/// Obecna kategorie a jednoduche prevody velikosti pismen z `UnicodeData.txt`.
pub struct UnicodeData {
    /// Kategorie kazdeho kodoveho bodu (`Cn` = neprirazeno).
    pub gc: Vec<[u8; 2]>,
    pub upper: BTreeMap<u32, u32>,
    pub lower: BTreeMap<u32, u32>,
}

pub fn parse_unicode_data(text: &str) -> Result<UnicodeData> {
    let mut gc = vec![*b"Cn"; MAX as usize + 1];
    let mut upper = BTreeMap::new();
    let mut lower = BTreeMap::new();
    let mut first: Option<u32> = None;
    for line in text.lines().map(|l| l.trim_end_matches('\r')).filter(|l| !l.is_empty()) {
        let f: Vec<&str> = line.split(';').collect();
        if f.len() < 14 {
            return Err(format!("UnicodeData: kratky radek {line:?}").into());
        }
        let cp = u32::from_str_radix(f[0], 16)?;
        let cat: [u8; 2] = f[2].as_bytes().try_into().map_err(|_| format!("UnicodeData: kategorie {:?}", f[2]))?;
        // Rozsahy: "<CJK Ideograph, First>" ... "<CJK Ideograph, Last>".
        if f[1].ends_with(", First>") {
            first = Some(cp);
            continue;
        }
        let start = if f[1].ends_with(", Last>") { first.take().ok_or("UnicodeData: Last bez First")? } else { cp };
        for c in start..=cp {
            gc[c as usize] = cat;
        }
        if !f[12].is_empty() {
            upper.insert(cp, u32::from_str_radix(f[12], 16)?);
        }
        if !f[13].is_empty() {
            lower.insert(cp, u32::from_str_radix(f[13], 16)?);
        }
    }
    Ok(UnicodeData { gc, upper, lower })
}

/// Rozsah `XXXX` nebo `XXXX..YYYY`.
fn parse_range(s: &str) -> Result<(u32, u32)> {
    let s = s.trim();
    Ok(match s.split_once("..") {
        Some((a, b)) => (u32::from_str_radix(a, 16)?, u32::from_str_radix(b, 16)?),
        None => {
            let v = u32::from_str_radix(s, 16)?;
            (v, v)
        }
    })
}

/// Kodove body s vlastnosti `prop` ze souboru ve tvaru `rozsah ; vlastnost`.
pub fn parse_property(text: &str, prop: &str) -> Result<Vec<bool>> {
    let mut set = vec![false; MAX as usize + 1];
    for line in text.lines() {
        let data = line.split('#').next().unwrap_or("").trim();
        let Some((range, value)) = data.split_once(';') else { continue };
        if value.trim() != prop {
            continue;
        }
        let (a, b) = parse_range(range)?;
        for c in a..=b {
            set[c as usize] = true;
        }
    }
    Ok(set)
}

/// Siroke znaky (East_Asian_Width W nebo F), vcetne vychozich hodnot
/// (`# @missing:` - neprirazene body v blocich CJK jsou siroke).
pub fn parse_wide(text: &str) -> Result<Vec<bool>> {
    let mut set = vec![false; MAX as usize + 1];
    let mut apply = |data: &str| -> Result<()> {
        let Some((range, value)) = data.split_once(';') else { return Ok(()) };
        let (a, b) = parse_range(range)?;
        let wide = matches!(value.trim(), "W" | "F");
        for c in a..=b {
            set[c as usize] = wide;
        }
        Ok(())
    };
    // Nejdriv vychozi hodnoty (v souboru jsou pred explicitnimi), pak
    // explicitni radky je prepisou.
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("# @missing:") {
            apply(rest.trim())?;
        }
    }
    for line in text.lines() {
        let data = line.split('#').next().unwrap_or("").trim();
        if !data.is_empty() {
            apply(data)?;
        }
    }
    Ok(set)
}

/// Mnozina jako serazene uzavrene rozsahy.
pub fn ranges(set: &[bool]) -> Vec<(u32, u32)> {
    let mut out: Vec<(u32, u32)> = Vec::new();
    for (c, &on) in set.iter().enumerate() {
        if !on {
            continue;
        }
        let c = c as u32;
        match out.last_mut() {
            Some(last) if last.1 + 1 == c => last.1 = c,
            _ => out.push((c, c)),
        }
    }
    out
}

/// Prevod velikosti jako behy `(zacatek, konec, krok, posun)`: znaky
/// `zacatek, zacatek+krok, ... konec` se prevadi pricitanim `posun`
/// (krok 2 = stridave velke a male, napr. U+0100..U+012F).
pub fn case_runs(map: &BTreeMap<u32, u32>) -> Vec<(u32, u32, u32, i32)> {
    let mut out: Vec<(u32, u32, u32, i32)> = Vec::new();
    for (&from, &to) in map {
        let delta = to as i64 - from as i64;
        let delta = delta as i32;
        if let Some(last) = out.last_mut() {
            let gap = from - last.1;
            let same = last.3 == delta;
            if same && last.0 == last.1 && (gap == 1 || gap == 2) {
                last.2 = gap;
                last.1 = from;
                continue;
            }
            if same && last.0 != last.1 && gap == last.2 {
                last.1 = from;
                continue;
            }
        }
        out.push((from, from, 1, delta));
    }
    out
}

fn table(out: &mut String, name: &str, doc: &str, r: &[(u32, u32)]) {
    let _ = writeln!(out, "/// {doc}");
    let _ = writeln!(out, "pub static {name}: &[(u32, u32)] = &[");
    // Ctyri rozsahy na radek (soubor zustane pod ~1500 radku).
    for chunk in r.chunks(4) {
        let items: Vec<String> = chunk.iter().map(|(a, b)| format!("(0x{a:04X}, 0x{b:04X})")).collect();
        let _ = writeln!(out, "    {},", items.join(", "));
    }
    let _ = writeln!(out, "];\n");
}

fn case_table(out: &mut String, name: &str, doc: &str, r: &[(u32, u32, u32, i32)]) {
    let _ = writeln!(out, "/// {doc}");
    let _ = writeln!(out, "pub static {name}: &[(u32, u32, u32, i32)] = &[");
    for chunk in r.chunks(3) {
        let items: Vec<String> = chunk.iter().map(|(a, b, s, d)| format!("(0x{a:04X}, 0x{b:04X}, {s}, {d})")).collect();
        let _ = writeln!(out, "    {},", items.join(", "));
    }
    let _ = writeln!(out, "];\n");
}

/// Obsah `tables.rs` ze ctyr souboru UCD.
pub fn render(unicode_data: &str, derived: &str, proplist: &str, east_asian: &str) -> Result<String> {
    let ud = parse_unicode_data(unicode_data)?;
    let alpha = parse_property(derived, "Alphabetic")?;
    let lowercase = parse_property(derived, "Lowercase")?;
    let uppercase = parse_property(derived, "Uppercase")?;
    let white = parse_property(proplist, "White_Space")?;
    let wide = parse_wide(east_asian)?;
    let n = MAX as usize + 1;
    let is = |c: usize, cats: &[&[u8; 2]]| cats.iter().any(|k| ud.gc[c] == **k);

    // Neviditelne pro tisk: ridici, nahradni polovicky, neprirazene,
    // oddelovace radku a odstavcu. Soukrome znaky (Co) se tisknou (ikony
    // fontu v terminalu).
    let nonprint: Vec<bool> = (0..n).map(|c| is(c, &[b"Cc", b"Cs", b"Cn", b"Zl", b"Zp"])).collect();
    let cntrl: Vec<bool> = (0..n).map(|c| is(c, &[b"Cc", b"Zl", b"Zp"])).collect();
    // Mezery bez zalomeni nejsou `isspace` (jako glibc a musl).
    let nbsp = [0x00A0usize, 0x2007, 0x202F];
    let space: Vec<bool> = (0..n).map(|c| white[c] && !nbsp.contains(&c)).collect();
    let blank: Vec<bool> = (0..n).map(|c| c == 0x09 || (is(c, &[b"Zs"]) && !nbsp.contains(&c))).collect();
    // Nulova sirka: nespacing a enclosing znamenka, format (krome mekkeho
    // rozdelovniku U+00AD, ten se zobrazuje), samohlasky a koncovky hangul
    // (skladaji se s predchozi slabikou), U+200B.
    let zero: Vec<bool> = (0..n)
        .map(|c| (is(c, &[b"Mn", b"Me", b"Cf"]) && c != 0x00AD) || (0x1160..=0x11FF).contains(&c) || (0xD7B0..=0xD7FF).contains(&c) || c == 0x200B)
        .collect();
    let wide: Vec<bool> = (0..n).map(|c| wide[c] && !zero[c] && !nonprint[c]).collect();

    let (a, b, c) = VERSION;
    let mut out = String::new();
    let _ = writeln!(out, "//! Vygenerovano `cargo xtask unicode-tables` z Unicode Character Database");
    let _ = writeln!(out, "//! {a}.{b}.{c} (licence Unicode-3.0, viz THIRD-PARTY.md). Needitovat rucne.\n");
    let _ = writeln!(out, "/// Verze UCD, ze ktere jsou tabulky.");
    let _ = writeln!(out, "pub const UNICODE_VERSION: (u8, u8, u8) = ({a}, {b}, {c});\n");
    table(&mut out, "ALPHABETIC", "Vlastnost Alphabetic (DerivedCoreProperties).", &ranges(&alpha));
    table(&mut out, "LOWERCASE", "Vlastnost Lowercase (DerivedCoreProperties).", &ranges(&lowercase));
    table(&mut out, "UPPERCASE", "Vlastnost Uppercase (DerivedCoreProperties).", &ranges(&uppercase));
    table(&mut out, "SPACE", "White_Space bez mezer bez zalomeni (U+00A0, U+2007, U+202F).", &ranges(&space));
    table(&mut out, "BLANK", "Tabulator a Zs bez mezer bez zalomeni.", &ranges(&blank));
    table(&mut out, "CNTRL", "Kategorie Cc, Zl, Zp.", &ranges(&cntrl));
    table(&mut out, "NONPRINT", "Kategorie Cc, Cs, Cn (neprirazeno), Zl, Zp.", &ranges(&nonprint));
    table(&mut out, "ZERO_WIDTH", "Nulova sirka: Mn, Me, Cf (krome U+00AD), jamo hangul, U+200B.", &ranges(&zero));
    table(&mut out, "WIDE", "Dvojita sirka: East_Asian_Width W nebo F (tisknutelne, nenulove).", &ranges(&wide));
    case_table(&mut out, "TO_UPPER", "Jednoduchy prevod na velka pismena: (zacatek, konec, krok, posun).", &case_runs(&ud.upper));
    case_table(&mut out, "TO_LOWER", "Jednoduchy prevod na mala pismena: (zacatek, konec, krok, posun).", &case_runs(&ud.lower));
    Ok(out)
}

#[cfg(test)]
#[path = "unicode_tests.rs"]
mod tests;
