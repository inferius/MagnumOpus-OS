//! Prace s cestami relace (`/a/b`): spojeni, rodic, jmeno, volne jmeno
//! pri konfliktu. Bez pristupu k FS.

use alloc::format;
use alloc::string::String;

/// `dir` + `/` + `name` (koren `/` bez zdvojeni lomitka).
pub fn join(dir: &str, name: &str) -> String {
    let d = dir.trim_end_matches('/');
    format!("{d}/{name}")
}

/// Rodic cesty (`/a/b` -> `/a`, `/a` -> `/`, `/` -> `/`).
pub fn parent(path: &str) -> &str {
    let p = path.trim_end_matches('/');
    match p.rfind('/') {
        Some(0) | None => "/",
        Some(i) => &p[..i],
    }
}

/// Posledni slozka cesty (`/a/b.txt` -> `b.txt`, `/` -> ``).
pub fn name(path: &str) -> &str {
    let p = path.trim_end_matches('/');
    p.rsplit('/').next().unwrap_or("")
}

/// Lezi `path` v `dir` nebo je to `dir` sam? (Kopie slozky do sebe.)
/// Porovnava se bez rozliseni velikosti pismen ASCII; FS porovnava jmena
/// po NFC a prevodu na mala pismena, takze tohle je jen predbezna
/// kontrola - konecne slovo ma FS.
pub fn is_within(path: &str, dir: &str) -> bool {
    let p = path.trim_end_matches('/');
    let d = dir.trim_end_matches('/');
    if d.is_empty() {
        return true;
    }
    p.len() >= d.len() && p[..d.len()].eq_ignore_ascii_case(d) && (p.len() == d.len() || p.as_bytes()[d.len()] == b'/')
}

/// Kandidat na volne jmeno: `zprava.txt` -> `zprava (2).txt`, slozka
/// `Fotky` -> `Fotky (2)`. Uz ocislovane jmeno se precisluje
/// (`a (2).txt`, n = 3 -> `a (3).txt`). Pripona jen u souboru a jen kdyz
/// jmeno nezacina teckou.
pub fn numbered(name: &str, n: u32, is_dir: bool) -> String {
    let (stem, ext) = match name.rfind('.') {
        Some(i) if !is_dir && i > 0 => (&name[..i], &name[i..]),
        _ => (name, ""),
    };
    let stem = strip_number(stem);
    format!("{stem} ({n}){ext}")
}

/// `a (2)` -> `a`; jinak beze zmeny.
fn strip_number(stem: &str) -> &str {
    if let Some(rest) = stem.strip_suffix(')')
        && let Some(i) = rest.rfind(" (")
        && rest[i + 2..].bytes().all(|b| b.is_ascii_digit())
        && !rest[i + 2..].is_empty()
    {
        return &stem[..i];
    }
    stem
}

/// Je jmeno platne pro novou polozku (ne prazdne, bez `/`, ne `.`/`..`,
/// bez ridicich znaku)? Zbytek (delka, smer textu) kontroluje FS.
pub fn valid_name(name: &str) -> bool {
    !name.is_empty() && name != "." && name != ".." && !name.contains('/') && !name.chars().any(char::is_control)
}
