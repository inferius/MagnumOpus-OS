//! Cesty: normalizace a porovnani.

use alloc::string::String;
use alloc::vec::Vec;

/// Absolutni normalizovana cesta z `p` vzhledem k `cwd` a domovske slozce
/// `home` (`~` jen na zacatku). Vysledek zacina `/`, nema `.`, `..` ani
/// dvojita lomitka.
pub fn resolve(cwd: &str, home: &str, p: &str) -> String {
    let full: String = if p == "~" {
        home.into()
    } else if let Some(rest) = p.strip_prefix("~/") {
        alloc::format!("{home}/{rest}")
    } else if p.starts_with('/') {
        p.into()
    } else {
        alloc::format!("{cwd}/{p}")
    };
    normalize(&full)
}

pub fn normalize(p: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();
    for c in p.split('/') {
        match c {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            other => parts.push(other),
        }
    }
    let mut out = String::new();
    for c in parts {
        out.push('/');
        out.push_str(c);
    }
    if out.is_empty() {
        out.push('/');
    }
    out
}

/// Klic pro porovnani (FS nerozlisuje velikost pismen).
pub fn key(p: &str) -> String {
    p.to_lowercase()
}

/// Nadrazena slozka (`/a/b` -> `/a`, `/a` -> `/`, `/` -> `None`).
pub fn parent(p: &str) -> Option<&str> {
    if p == "/" {
        return None;
    }
    match p.rfind('/') {
        Some(0) => Some("/"),
        Some(i) => Some(&p[..i]),
        None => None,
    }
}

/// Posledni cast cesty.
pub fn file_name(p: &str) -> &str {
    p.rsplit('/').next().unwrap_or(p)
}

pub fn join(dir: &str, name: &str) -> String {
    if dir == "/" { alloc::format!("/{name}") } else { alloc::format!("{dir}/{name}") }
}

/// Je `p` rovno `prefix` nebo lezi uvnitr (po celych castech, bez ohledu
/// na velikost pismen)?
pub fn within(p: &str, prefix: &str) -> bool {
    let (p, prefix) = (key(p), key(prefix));
    if prefix == "/" {
        return true;
    }
    p == prefix || (p.starts_with(&prefix) && p.as_bytes().get(prefix.len()) == Some(&b'/'))
}

/// Pripona souboru bez tecky (`a.tar.gz` -> `gz`, `.bashrc` -> ``).
pub fn ext(name: &str) -> &str {
    match name.rfind('.') {
        Some(0) | None => "",
        Some(i) => &name[i + 1..],
    }
}

/// Shoda jmena se vzorem (`*`, `?`, `[abc]`, `[a-z]`, `[!a]`), bez ohledu
/// na velikost pismen.
pub fn glob_match(pat: &str, name: &str) -> bool {
    let p: Vec<char> = pat.to_lowercase().chars().collect();
    let n: Vec<char> = name.to_lowercase().chars().collect();
    fn go(p: &[char], n: &[char]) -> bool {
        match p.first() {
            None => n.is_empty(),
            Some('*') => (0..=n.len()).any(|i| go(&p[1..], &n[i..])),
            Some('?') => !n.is_empty() && go(&p[1..], &n[1..]),
            Some('[') => {
                let Some(close) = p.iter().position(|&c| c == ']') else {
                    return n.first() == Some(&'[') && go(&p[1..], &n[1..]);
                };
                let Some(&c) = n.first() else { return false };
                let set = &p[1..close];
                let (neg, set) = match set.first() {
                    Some('!') | Some('^') => (true, &set[1..]),
                    _ => (false, set),
                };
                let mut hit = false;
                let mut i = 0;
                while i < set.len() {
                    if i + 2 < set.len() && set[i + 1] == '-' {
                        if set[i] <= c && c <= set[i + 2] {
                            hit = true;
                        }
                        i += 3;
                    } else {
                        if set[i] == c {
                            hit = true;
                        }
                        i += 1;
                    }
                }
                hit != neg && go(&p[close + 1..], &n[1..])
            }
            Some(&c) => n.first() == Some(&c) && go(&p[1..], &n[1..]),
        }
    }
    go(&p, &n)
}
