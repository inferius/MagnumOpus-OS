//! Hledani knihoven (DL-4c): kde zavadec hleda zavislost z `DT_NEEDED`
//! nebo cestu z `dlopen`.
//!
//! - Jmeno s lomitkem: jen absolutni cesta (po normalizaci); pod
//!   `/system/lib/` = systemova knihovna od libd, jinak soubor na disku.
//!   Relativni cesta s lomitkem se odmita (zavadec nema pracovni adresar).
//! - Hole jmeno: adresare z `DT_RUNPATH` modulu, ktery zavislost chce
//!   (oddelene `:`, `$ORIGIN` / `${ORIGIN}` = adresar toho modulu), pak
//!   `/system/lib`. Polozka s `$ORIGIN` u modulu bez adresare (program
//!   z initfs, knihovna od libd) se preskoci; jine promenne (`$LIB`,
//!   `$PLATFORM`) se nepodporuji a polozka se preskoci.
//!
//! `DT_RUNPATH` plati jen pro primo zavislosti modulu (jako glibc), zastaraly
//! `DT_RPATH` cte `libs/elf` misto nej, kdyz `DT_RUNPATH` chybi.

use alloc::string::String;
use alloc::vec::Vec;

/// Systemove knihovny (initfs `lib/`, od libd).
pub const SYSTEM_LIB: &str = "/system/lib";

/// Kde zkusit knihovnu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    /// Jmeno v `/system/lib` (od libd podle jmena).
    System(String),
    /// Absolutni normalizovana cesta souboru na disku (cte zavadec relaci
    /// FS procesu).
    Disk(String),
}

/// Normalizuje absolutni cestu: slouci `//`, vynecha `.`, `..` vrati
/// o adresar zpet (nad korenem zustane v koreni). `None` = cesta neni
/// absolutni nebo obsahuje nulu.
pub fn normalize(path: &str) -> Option<String> {
    if !path.starts_with('/') || path.contains('\0') {
        return None;
    }
    let mut parts: Vec<&str> = Vec::new();
    for p in path.split('/') {
        match p {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            p => parts.push(p),
        }
    }
    let mut out = String::new();
    for p in &parts {
        out.push('/');
        out.push_str(p);
    }
    if out.is_empty() {
        out.push('/');
    }
    Some(out)
}

/// Adresar souboru (normalizovana absolutni cesta): `/a/b/c` -> `/a/b`,
/// `/c` -> `/`.
pub fn dir_of(path: &str) -> &str {
    match path.rfind('/') {
        Some(0) => "/",
        Some(i) => &path[..i],
        None => path,
    }
}

/// Posledni slozka cesty (jmeno souboru).
pub fn file_name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

/// Rozvine `$ORIGIN` a `${ORIGIN}` v polozce `DT_RUNPATH`. `None` = polozku
/// preskocit (modul nema adresar, neznama promenna, vysledek neni
/// absolutni).
pub fn expand(entry: &str, origin: Option<&str>) -> Option<String> {
    let mut out = String::new();
    let mut rest = entry;
    while let Some(i) = rest.find('$') {
        out.push_str(&rest[..i]);
        let after = &rest[i + 1..];
        let (var, tail) = if let Some(b) = after.strip_prefix('{') {
            let end = b.find('}')?;
            (&b[..end], &b[end + 1..])
        } else {
            let end = after.find(|c: char| !(c.is_ascii_alphanumeric() || c == '_')).unwrap_or(after.len());
            (&after[..end], &after[end..])
        };
        if var != "ORIGIN" {
            return None;
        }
        out.push_str(origin?);
        rest = tail;
    }
    out.push_str(rest);
    normalize(&out)
}

/// Mista, kde hledat zavislost `needed` modulu s `DT_RUNPATH` `runpath`
/// a adresarem `origin`, v poradi zkouseni (bez duplicit). `Err` = vadne
/// jmeno (relativni cesta s lomitkem, prazdne jmeno, `.`/`..`).
pub fn candidates(needed: &str, runpath: Option<&str>, origin: Option<&str>) -> Result<Vec<Source>, &'static str> {
    if needed.is_empty() || needed == "." || needed == ".." {
        return Err("prazdne jmeno knihovny");
    }
    if needed.contains('/') {
        let path = normalize(needed).ok_or("relativni cesta knihovny (jen absolutni nebo jmeno)")?;
        return Ok(alloc::vec![classify(&path)]);
    }
    let mut out: Vec<Source> = Vec::new();
    for entry in runpath.unwrap_or("").split(':').filter(|e| !e.is_empty()) {
        let Some(dir) = expand(entry, origin) else { continue };
        let mut path = dir;
        if !path.ends_with('/') {
            path.push('/');
        }
        path.push_str(needed);
        let s = classify(&path);
        if !out.contains(&s) {
            out.push(s);
        }
    }
    let sys = Source::System(String::from(needed));
    if !out.contains(&sys) {
        out.push(sys);
    }
    Ok(out)
}

/// Normalizovana absolutni cesta -> systemova knihovna (primo v
/// `/system/lib`) nebo soubor na disku.
fn classify(path: &str) -> Source {
    match path.strip_prefix(SYSTEM_LIB).and_then(|r| r.strip_prefix('/')) {
        Some(name) if !name.is_empty() && !name.contains('/') => Source::System(String::from(name)),
        _ => Source::Disk(String::from(path)),
    }
}
