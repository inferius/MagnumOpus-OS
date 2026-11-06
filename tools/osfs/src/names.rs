//! Preklad jmen mezi nasim FS a Windows.
//!
//! Pravidla podle `docs/cs/navrh-fs-v3.md`, kapitola "Ovladac pro Windows":
//!
//! | Jmeno v nasem FS                                  | Ve Windows                     |
//! |---------------------------------------------------|--------------------------------|
//! | platne i ve Windows                               | beze zmeny                     |
//! | obsahuje `: \ * ? " < > \|`                       | znak -> U+F000 + kod znaku     |
//! | uz samo obsahuje znak z U+F000-F0FF               | skryte                         |
//! | rezervovane jmeno (CON, NUL, COM1, ... i s priponou) | skryte                      |
//! | koncova tecka nebo mezera                         | skryte                         |
//! | delsi nez 255 znaku UTF-16                        | skryte                         |
//!
//! Klicova vlastnost: `from_windows(w) == Ok(n)` prave kdyz
//! `to_windows(n) == Some(w)`. Kazde zobrazene jmeno tedy vede zpet
//! presne na jmeno, ze ktereho vzniklo, a skryte jmeno nejde z Windows
//! zadat vubec (ani oklikou pres U+F0xx nebo koncovou teckou).
//!
//! Porovnani bez ohledu na velikost pismen dela nas FS (`fold_name`), ne
//! tento modul: jmeno z Windows se preda FS tak, jak prislo.

use std::fmt;

/// Znaky, ktere Windows ve jmenech zakazuji a nas FS povoluje. (`/` a
/// ridici znaky zakazuje uz nas FS, sem se nedostanou.)
pub const FORBIDDEN: [char; 8] = [':', '\\', '*', '?', '"', '<', '>', '|'];

/// Zacatek oblasti pro soukrome pouziti, kam se zakazane znaky posunou
/// (stejne jako WSL a Cygwin).
pub const PUA_BASE: u32 = 0xF000;

/// Nejdelsi jmeno ve Windows (jednotky UTF-16).
pub const WIN_NAME_MAX: usize = 255;

/// Proc jmeno z nasho FS ve Windows nejde zobrazit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hidden {
    /// Obsahuje znak z U+F000-F0FF (kolidoval by s prelozenym jmenem).
    PrivateUse,
    /// Rezervovane jmeno zarizeni (CON, NUL, COM1, ...).
    Reserved,
    /// Konci teckou nebo mezerou (Win32 je pri otevreni orizne).
    TrailingDotOrSpace,
    /// Delsi nez 255 znaku UTF-16.
    TooLong,
}

impl fmt::Display for Hidden {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Hidden::PrivateUse => "obsahuje znak z U+F000-F0FF",
            Hidden::Reserved => "rezervovane jmeno Windows",
            Hidden::TrailingDotOrSpace => "konci teckou nebo mezerou",
            Hidden::TooLong => "delsi nez 255 znaku UTF-16",
        })
    }
}

/// Proc jmeno zadane ve Windows nejde prevest.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BadName {
    /// Znak U+F0xx, ktery neodpovida zadnemu zakazanemu znaku.
    StrayPrivateUse,
    /// Primo zakazany znak (Windows ho normalne nepusti).
    Forbidden,
    /// Vysledek by byl ve Windows skryty (rezervovane jmeno, koncova
    /// tecka, prilis dlouhe).
    WouldBeHidden(Hidden),
    /// Nas FS jmeno odmita (prazdne, `.`, `..`, znaky smeru textu, ...).
    RejectedByFs,
}

fn is_pua(c: char) -> bool {
    (PUA_BASE..=PUA_BASE + 0xFF).contains(&(c as u32))
}

/// Rezervovana jmena zarizeni. Windows je hlida i s priponou (`CON.txt`)
/// a s mezerami pred priponou (`NUL .txt`). Navic proti tabulce v navrhu
/// skryvame i `COM`/`LPT` s hornim indexem (Windows 11 je bere jako
/// rezervovane) a `CONIN$`, `CONOUT$` - skryt vic je bezpecne.
pub fn is_reserved(name: &str) -> bool {
    let base = name.split('.').next().unwrap_or("");
    let base = base.trim_end_matches(' ');
    let upper: String = base.chars().map(|c| c.to_ascii_uppercase()).collect();
    match upper.as_str() {
        "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$" => true,
        _ => {
            let rest = upper
                .strip_prefix("COM")
                .or_else(|| upper.strip_prefix("LPT"));
            match rest {
                Some(r) => {
                    let mut it = r.chars();
                    matches!(
                        (it.next(), it.next()),
                        (Some('1'..='9' | '\u{B9}' | '\u{B2}' | '\u{B3}'), None)
                    )
                }
                None => false,
            }
        }
    }
}

/// Jmeno z nasho FS -> jmeno ve Windows, nebo duvod, proc je skryte.
pub fn to_windows_checked(name: &str) -> Result<String, Hidden> {
    if name.chars().any(is_pua) {
        return Err(Hidden::PrivateUse);
    }
    let out: String = name
        .chars()
        .map(|c| {
            if FORBIDDEN.contains(&c) {
                char::from_u32(PUA_BASE + c as u32).unwrap_or(c)
            } else {
                c
            }
        })
        .collect();
    if out.ends_with('.') || out.ends_with(' ') {
        return Err(Hidden::TrailingDotOrSpace);
    }
    if is_reserved(&out) {
        return Err(Hidden::Reserved);
    }
    if out.encode_utf16().count() > WIN_NAME_MAX {
        return Err(Hidden::TooLong);
    }
    Ok(out)
}

/// Jmeno z nasho FS -> jmeno ve Windows (`None` = skryte).
pub fn to_windows(name: &str) -> Option<String> {
    to_windows_checked(name).ok()
}

/// Jmeno zadane ve Windows -> jmeno v nasem FS.
pub fn from_windows(win: &str) -> Result<String, BadName> {
    let mut out = String::with_capacity(win.len());
    for c in win.chars() {
        if FORBIDDEN.contains(&c) {
            return Err(BadName::Forbidden);
        }
        if is_pua(c) {
            let orig = char::from_u32(c as u32 - PUA_BASE).unwrap_or('\0');
            if !FORBIDDEN.contains(&orig) {
                return Err(BadName::StrayPrivateUse);
            }
            out.push(orig);
        } else {
            out.push(c);
        }
    }
    fs::validate_name(&out).map_err(|_| BadName::RejectedByFs)?;
    // Zpetna kontrola: jen jmeno, ktere by se presne takto zobrazilo.
    match to_windows_checked(&out) {
        Ok(back) if back == win => Ok(out),
        Ok(_) => Err(BadName::RejectedByFs),
        Err(h) => Err(BadName::WouldBeHidden(h)),
    }
}

/// Cesta z Windows (`\a\b`, relativne ke koreni svazku) -> cesta v nasem
/// FS (`/a/b`). Prazdne slozky (`\\`) se preskoci.
pub fn path_from_windows(win: &str) -> Result<String, BadName> {
    let mut out = String::from("/");
    for comp in win.split('\\').filter(|c| !c.is_empty()) {
        if out.len() > 1 {
            out.push('/');
        }
        out.push_str(&from_windows(comp)?);
    }
    Ok(out)
}

/// Retezec UTF-16 (z Windows) -> String. Nesparovane surogaty nejsou
/// platne Unicode a nas FS je neumi ulozit -> `None`.
pub fn utf16_to_string(w: &[u16]) -> Option<String> {
    String::from_utf16(w).ok()
}
