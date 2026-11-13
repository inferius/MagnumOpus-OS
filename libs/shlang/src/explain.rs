//! Vysvetleni chyb a hlasek z dokumentace (`explain NOT_FOUND`).
//!
//! Texty se berou primo z `docs/cs/chyby.md` (kody systemu, sekce
//! `### \`NAZEV\` (-N)`), z kapitoly Chyby shellu v `docs/cs/navrh-shell.md`
//! (kody `SH_*`, stejny tvar sekci) a z tabulek `docs/cs/hlasky.md`
//! (hlasky v logu). Dokumentace tak zustava jedinym zdrojem.

use alloc::string::String;

const ERRORS: &str = include_str!("../../../docs/cs/chyby.md");
const SHELL: &str = include_str!("../../../docs/cs/navrh-shell.md");
const MESSAGES: &str = include_str!("../../../docs/cs/hlasky.md");

/// Sekce `### \`CODE\`...` az do dalsiho nadpisu.
fn section(doc: &str, code: &str) -> Option<String> {
    let head = alloc::format!("### `{code}`");
    let start = doc.find(&head)?;
    let body_start = start + doc[start..].find('\n')? + 1;
    let rest = &doc[body_start..];
    let end = rest.find("\n#").map(|i| i + 1).unwrap_or(rest.len());
    let title = doc[start + 4..body_start].trim();
    Some(alloc::format!("{title}\n\n{}", rest[..end].trim()))
}

/// Radek tabulky hlasek, jehoz hlaska (v backtickach) je obsazena v `text`
/// nebo ho obsahuje.
fn message(text: &str) -> Option<String> {
    let t = text.trim();
    if t.len() < 4 {
        return None;
    }
    for line in MESSAGES.lines() {
        if !line.starts_with("| ") || line.starts_with("| Hláška") {
            continue;
        }
        let cells: alloc::vec::Vec<&str> = line.trim_matches('|').split(" | ").map(|c| c.trim()).collect();
        let Some(first) = cells.first() else { continue };
        let pat = first.trim_start_matches("panika ").trim_matches('`');
        let fixed = pat.split("...").next().unwrap_or(pat).trim();
        if fixed.len() >= 4 && (t.contains(fixed) || fixed.contains(t)) {
            return Some(alloc::format!("{first}\n\n{}", cells[1..].join(" - ")));
        }
    }
    None
}

/// Vysvetleni kodu chyby (`NOT_FOUND`, `SH_PARSE`) nebo hlasky z logu.
pub fn explain(what: &str) -> Option<String> {
    let w = what.trim();
    if let Some(n) = w.parse::<i64>().ok().and_then(abi::Error::from_code) {
        return section(ERRORS, n.name());
    }
    let upper = w.to_uppercase();
    section(ERRORS, &upper).or_else(|| section(SHELL, &upper)).or_else(|| message(w))
}

/// Kratky cesky vyznam kodu chyby systemu z katalogu (`**Význam:**`
/// v `docs/cs/chyby.md`) bez tecky na konci: do hlaseni "příčina: KOD:
/// vyznam". `None` = kod v katalogu neni.
pub fn meaning(code: &str) -> Option<String> {
    let head = alloc::format!("### `{code}`");
    let start = ERRORS.find(&head)?;
    let rest = &ERRORS[start + head.len()..];
    let end = rest.find("
#").unwrap_or(rest.len());
    let body = &rest[..end];
    let tag = "**Význam:**";
    let at = body.find(tag)? + tag.len();
    // Odstavec az do prazdneho radku, radky spojit mezerou.
    let para = body[at..].split("\n\n").next()?;
    let text: alloc::vec::Vec<&str> = para.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
    let text = text.join(" ");
    let text = text.trim().trim_end_matches('.');
    (!text.is_empty()).then(|| String::from(text))
}

/// Je kod shellu popsany v dokumentaci (pro test)?
pub fn documented(code: &str) -> bool {
    section(SHELL, code).is_some()
}
