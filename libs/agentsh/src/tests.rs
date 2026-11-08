//! Testy na hostiteli (`cargo test -p agentsh`).

use shlang::Style;

use crate::{explain_context, plan, risks, system_prompt, validate};

#[test]
fn platny_navrh_v_obou_zapisech() {
    let p = validate("ls ~/Dok -r | where size > 100MB", None).unwrap();
    assert_eq!(p.style, Style::B);
    assert!(!p.risky, "{:?}", p.reasons);
    assert!(p.other.contains("ls(") && p.other.contains("where"), "{}", p.other);
    // Zpatky z C na B dava stejny prikaz.
    let back = validate(&p.other, Some(Style::C)).unwrap();
    assert_eq!(back.other, p.command);
}

#[test]
fn neplatny_navrh_vrati_hlaseni_parseru() {
    let e = validate("ls | where (", None).unwrap_err();
    assert!(e.contains("chyba"), "{e}");
    assert!(validate("   ", None).is_err());
}

#[test]
fn riskantni_prikazy() {
    let p = validate("rm ~/stare -r", None).unwrap();
    assert!(p.risky);
    assert!(!risks("mv ~/a ~/b").is_empty());
    assert!(risks("ls ~").is_empty());
    assert!(!risks("neznamy-program x").is_empty(), "neznamy program je riskantni");
}

#[test]
fn plan_overi_kazdy_krok() {
    assert_eq!(plan(&["ls ~", "df"], None).unwrap().len(), 2);
    let e = plan(&["ls ~", "where ("], None).unwrap_err();
    assert!(e.starts_with("krok 2"), "{e}");
}

#[test]
fn pokyny_a_vysvetleni() {
    let s = system_prompt("shell", None);
    assert!(s.contains("NEDUVERYHODNA DATA") && s.contains("shell__propose"));
    assert!(system_prompt("dev", Some("/home/a/proj")).contains("/home/a/proj"));
    let e = explain_context("chyba NOT_FOUND: soubor neexistuje").unwrap();
    assert!(e.contains("NOT_FOUND"), "{e}");
}
