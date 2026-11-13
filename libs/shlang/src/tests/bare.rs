//! Hola cesta jako prikaz: slozka = cd, skript .mos = run, program
//! = spusteni (prava jako u programu), jiny soubor = open, chyby.

use super::host::run_lines;
use crate::value::Value;

#[test]
fn folder_paths_change_directory() {
    let (_, it, o) = run_lines(&["mkdir d", "./d"]);
    o.result.unwrap();
    assert_eq!(it.cwd, "/home/d");
    let (_, it, o) = run_lines(&["mkdir d", "./d", ".."]);
    o.result.unwrap();
    assert_eq!(it.cwd, "/home");
    let (_, it, o) = run_lines(&["/", "~"]);
    o.result.unwrap();
    assert_eq!(it.cwd, "/home");
    let (_, it, o) = run_lines(&["/"]);
    o.result.unwrap();
    assert_eq!(it.cwd, "/");
}

#[test]
fn script_runs_and_file_opens() {
    let (h, _, o) = run_lines(&["echo \"echo ze skriptu\" | save s.mos", "./s.mos"]);
    o.result.unwrap();
    assert!(h.out.iter().any(|v| v.to_text() == "ze skriptu"), "{:?}", h.out);
    // Jiny soubor: obsah jako `open` (nic se nespousti).
    let (h, _, o) = run_lines(&["./a.txt"]);
    o.result.unwrap();
    assert_eq!(h.out, vec![Value::Bytes(b"alfa
".to_vec())]);
}

#[test]
fn errors_for_missing_paths_args_and_programs() {
    let (_, _, o) = run_lines(&["./neni"]);
    let e = o.result.unwrap_err();
    assert_eq!(e.code, "NOT_FOUND");
    assert!(e.hint.as_deref().is_some_and(|h| h.contains("cd")), "{e:?}");
    let (_, _, o) = run_lines(&["mkdir d", "./d x"]);
    assert_eq!(o.result.unwrap_err().code, "SH_ARGS");
    // Program (ELF): jde cestou programu; hostitel ho neumi spustit.
    let (_, _, o) = run_lines(&["\"\u{7f}ELF\" | save prog", "./prog"]);
    assert_eq!(o.result.unwrap_err().code, "NOT_SUPPORTED");
}

#[test]
fn bare_path_is_highlighted_as_path() {
    use crate::highlight::{Class, Ctx, highlight};
    let ctx = Ctx { sigs: &crate::sig::Builtins, vars: &[], commands: &[] };
    for src in ["./d", "~/Dokumenty", "/", ".."] {
        let h = highlight(src, Some(&ctx));
        assert_eq!(h.spans.first().map(|s| s.class), Some(Class::Path), "{src}: {h:?}");
    }
}
