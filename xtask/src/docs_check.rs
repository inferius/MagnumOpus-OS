//! Kontroly obsahu dokumentace podle `docs/cs/dokumentace.md`.
//!
//! - Znacky stavu (`**[...]**`) jen z povolene sady jazyka - chyba.
//! - Cislo z mereni (ms, us, MB/s, fps...) bez revize sestaveni
//!   (hash commitu v backtickach na stejnem radku) - varovani.
//! - cs a en verze stranky maji jiny pocet nadpisu 2. urovne - varovani.
//!
//! Varovani nezastavi sestaveni (stara dokumentace je jeste neplni);
//! audit je postupne odstranuje.

/// Povolene znacky stavu podle jazyka.
const MARKS: [(&str, &[&str]); 2] = [
    ("cs", &["navrženo", "implementováno", "ověřeno v QEMU", "ověřeno na HW"]),
    ("en", &["designed", "implemented", "verified in QEMU", "verified on HW"]),
];

/// Jednotky, podle kterych se pozna cislo z mereni.
const UNITS: [&str; 9] = ["ms", "µs", "us", "ns", "MB/s", "MiB/s", "GB/s", "GiB/s", "fps"];

/// Zkontroluje text jedne stranky.
pub fn page(lang: &str, file: &str, md: &str, errors: &mut Vec<String>, warnings: &mut Vec<String>) {
    let allowed = MARKS.iter().find(|(l, _)| *l == lang).map_or(&[][..], |(_, m)| *m);
    let mut in_code = false;
    for (n, line) in md.lines().enumerate() {
        let t = line.trim_start();
        if t.starts_with("```") {
            in_code = !in_code;
            continue;
        }
        // Diagramy (SVG) a kod se nekontroluji.
        if in_code || t.starts_with('<') {
            continue;
        }
        let at = format!("{lang}/{file}:{}", n + 1);
        let mut rest = line;
        while let Some(i) = rest.find("**[") {
            let after = &rest[i + 3..];
            let Some(end) = after.find("]**") else { break };
            let mark = &after[..end];
            if !allowed.contains(&mark) {
                errors.push(format!("{at}: neznama znacka stavu [{mark}] (povolene: {})", allowed.join(", ")));
            }
            rest = &after[end + 3..];
        }
        if has_measurement(line) && !has_revision(line) {
            warnings.push(format!("{at}: mereni bez revize sestaveni"));
        }
    }
}

/// Porovna pocty nadpisu 2. urovne v cs a en verzi stranky.
pub fn pair(cs_file: &str, cs_h2: usize, en_file: &str, en_h2: usize, warnings: &mut Vec<String>) {
    if cs_h2 != en_h2 {
        warnings.push(format!("cs/{cs_file} ({cs_h2} kapitol) a en/{en_file} ({en_h2}) se lisi"));
    }
}

/// Cislo hned pred jednotkou mereni (`3,5 ms`, `120 MB/s`, `60fps`).
fn has_measurement(line: &str) -> bool {
    UNITS.iter().any(|u| {
        let mut s = line;
        while let Some(i) = s.find(u) {
            let before = s[..i].trim_end();
            let after = &s[i + u.len()..];
            let word_end = after.chars().next().is_none_or(|c| !c.is_alphanumeric());
            if word_end && before.chars().last().is_some_and(|c| c.is_ascii_digit()) {
                return true;
            }
            s = &s[i + u.len()..];
        }
        false
    })
}

/// Hash commitu (7-40 hex znaku, aspon jedna cislice a jedno pismeno)
/// v backtickach.
fn has_revision(line: &str) -> bool {
    line.split('`').skip(1).step_by(2).any(|t| {
        let t = t.trim_end_matches('*');
        (7..=40).contains(&t.len())
            && t.bytes().all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
            && t.bytes().any(|b| b.is_ascii_digit())
            && t.bytes().any(|b| b.is_ascii_alphabetic())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn marks_and_measurements() {
        let (mut e, mut w) = (Vec::new(), Vec::new());
        page("cs", "x.md", "a **[navrženo]** b **[designed]**\nskladani 3,5 ms\nskladani 2,1 ms (`b24c01f`)\n```\n9 ms\n```", &mut e, &mut w);
        assert_eq!(e.len(), 1, "{e:?}");
        assert!(e[0].contains("[designed]"));
        assert_eq!(w.len(), 1, "{w:?}");
        assert!(w[0].contains("x.md:2"));
        assert!(!has_measurement("forms a list"));
        assert!(has_measurement("60fps"));
        assert!(!has_revision("`cargo`"));
    }
}
