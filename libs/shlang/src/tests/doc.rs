//! Referencni napoveda: uplnost metadat a spustitelne priklady.

use super::host::run_lines;
use crate::doc::{self, Check};
use crate::sig::{BUILTINS, GROUPS};

#[test]
fn every_command_has_complete_docs() {
    let mut bad = Vec::new();
    for s in BUILTINS {
        let Some(d) = doc::get(s.name) else {
            bad.push(format!("{}: chybi metadata", s.name));
            continue;
        };
        if doc::group_of(s.name).is_none() {
            bad.push(format!("{}: neni ve skupine", s.name));
        }
        if d.examples.is_empty() {
            bad.push(format!("{}: zadny priklad", s.name));
        }
        for t in [d.short_en, d.long_cs, d.long_en] {
            if t.is_empty() {
                bad.push(format!("{}: prazdny popis", s.name));
            }
        }
        for a in d.args {
            let known = a.name == "..." || s.params.iter().any(|p| p.name == a.name) || s.flags.iter().any(|f| f.long == a.name);
            if !known {
                bad.push(format!("{}: argument {} neni v podpisu", s.name, a.name));
            }
        }
        for r in d.related {
            if !BUILTINS.iter().any(|b| b.name == *r) {
                bad.push(format!("{}: souvisejici {r} neexistuje", s.name));
            }
        }
        for e in d.errors {
            if crate::explain::explain(e).is_none() {
                bad.push(format!("{}: chybu {e} explain nezna", s.name));
            }
        }
    }
    for d in doc::all() {
        if !BUILTINS.iter().any(|b| b.name == d.name) {
            bad.push(format!("{}: metadata bez prikazu", d.name));
        }
    }
    assert_eq!(GROUPS.len(), doc::GROUPS_EN.len());
    assert!(bad.is_empty(), "{}", bad.join("\n"));
}

/// Vystup prikladu jako v terminalu bez barev.
fn rendered(out: &[crate::value::Value]) -> String {
    let s: String = out.iter().map(crate::table::render).collect();
    s.lines().map(str::trim_end).collect::<Vec<_>>().join("\n").trim().to_string()
}

#[test]
fn doc_examples_run() {
    let mut bad = Vec::new();
    for d in doc::all() {
        for e in d.examples {
            if e.check == Check::Skip {
                continue;
            }
            let (h, _, o) = run_lines(&[e.code]);
            if let Err(err) = &o.result {
                bad.push(format!("{}: `{}` selhal: {}", d.name, e.code, err.report(e.code)));
                continue;
            }
            if let Check::Out(want) = e.check {
                let got = rendered(&h.out);
                // `DOC_BLESS=soubor`: skutecne vystupy (kod TAB vystup) k doplneni.
                if let Ok(f) = std::env::var("DOC_BLESS") {
                    use std::io::Write;
                    let mut o = std::fs::OpenOptions::new().create(true).append(true).open(f).unwrap();
                    writeln!(o, "{}\t{}", e.code, got.replace('\\', "\\\\").replace('\n', "\\n")).unwrap();
                }
                if got != want.trim() {
                    bad.push(format!("{}: `{}`\n--- ocekavano\n{}\n--- vystup\n{got}", d.name, e.code, want.trim()));
                }
            }
        }
    }
    assert!(bad.is_empty(), "{} prikladu neodpovida:\n{}", bad.len(), bad.join("\n\n"));
}

#[test]
fn help_shows_sections_and_markdown_has_anchors() {
    let s = BUILTINS.iter().find(|s| s.name == "ls").unwrap();
    let h = doc::help_text(s);
    for sec in ["Použití", "Argumenty a přepínače", "Roura", "Příklady", "Souvisí"] {
        assert!(h.contains(sec), "{sec}: {h}");
    }
    for en in [false, true] {
        let md = doc::markdown(en);
        for b in BUILTINS {
            assert!(md.contains(&format!("### `{}`", b.name)), "{}", b.name);
        }
    }
}
