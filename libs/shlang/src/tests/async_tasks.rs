//! Testy soubeznosti v jazyce: zapis B a C, prevod, vyhodnoceni u hostitele
//! bez soubeznosti (uloha probehne hned na miste).

use super::host::{TestHost, interp, ok};
use super::roundtrip::check;
use crate::error::code;
use crate::parse::Style;
use crate::value::Value;

#[test]
fn notation_b_and_c_roundtrip() {
    for src in [
        "let a = async {\n    ls ~/Fotky -r | where ext == \"jpg\" | count\n}",
        "let a = async {\n    1\n}\nlet b = async {\n    2\n}\nlet (x, y) = await all [a, b]",
        "let a = async {\n    1\n}\nlet p = await any [a] timeout 5s",
        "let a = async {\n    1\n}\nlet r = try await a",
        "let a = async {\n    sleep 1s\n}\ncancel $a",
        "ls | each --parallel 8 f => f.size * 2",
    ] {
        check(src, Style::B);
    }
    // C: `a.await` je zkratka, ostatni stejna slova.
    let (b, c) = check("let a = async { 1 }\nlet x = a.await", Style::C);
    assert!(c.contains("let x = a.await"), "{c}");
    assert!(b.contains("let x = await a"), "{b}");
    let (_, c) = check("let a = async {\n    1\n}\nlet v = await all [a] timeout 2s", Style::B);
    assert!(c.contains("await all [a] timeout 2s"), "{c}");
}

#[test]
fn await_values_all_any_and_destructuring() {
    let out = ok("let a = async { 1 + 1 }\nlet b = async { \"x\" }\nlet (p, q) = await all [a, b]\necho \"{p} {q}\"");
    assert_eq!(out, vec![Value::str("2 x")]);
    assert_eq!(ok("let a = async { 7 }\nawait a"), vec![Value::Int(7)]);
    assert_eq!(ok("let a = async { 7 }\nlet b = async { 8 }\nawait any [a, b]"), vec![Value::Int(7)]);
    // Uloha vidi kopii promenne z doby vzniku.
    assert_eq!(ok("let x = 1\nlet a = async { x + 10 }\nx = 5\nawait a"), vec![Value::Int(11)]);
}

#[test]
fn errors_propagate_at_await_and_try_catches_them() {
    let mut h = TestHost::new();
    let mut it = interp();
    let o = it.run_line(&mut h, "let a = async { error \"spatne\" }\nawait a");
    assert_eq!(o.result.unwrap_err().code, code::USER);
    let o = it.run_line(&mut h, "let a = async { error \"spatne\" }\nlet r = try await a\nr.failed");
    o.result.unwrap();
    assert_eq!(h.out.last(), Some(&Value::Bool(true)));
    // await all: chyba jedne ulohy = chyba celku.
    let o = it.run_line(&mut h, "let a = async { 1 }\nlet b = async { error \"b\" }\nawait all [a, b]");
    assert_eq!(o.result.unwrap_err().code, code::USER);
    // Spatne pouziti.
    assert_eq!(it.run_line(&mut h, "await 5").result.unwrap_err().code, code::TYPE);
    assert_eq!(it.run_line(&mut h, "let a = async { 1 }\nawait any a").result.unwrap_err().code, code::TYPE);
    assert_eq!(it.run_line(&mut h, "let a = async { 1 }\nawait a timeout 5").result.unwrap_err().code, code::TYPE);
    assert_eq!(it.run_line(&mut h, "let (a, b) = [1]").result.unwrap_err().code, code::ARGS);
}

#[test]
fn async_changes_files_and_cancel_is_harmless_when_done() {
    let mut h = TestHost::new();
    let mut it = interp();
    it.run_line(&mut h, "let a = async { echo z > u.txt }\nawait a\ncancel $a").result.unwrap();
    assert_eq!(h.fs.get("/home/u.txt"), Some(&b"z"[..]));
    it.run_line(&mut h, "undo").result.unwrap();
    assert!(!h.fs.exists("/home/u.txt"));
}

#[test]
fn each_parallel_keeps_order() {
    let out = ok("[3, 1, 2] | each --parallel 2 x => x * 10");
    assert_eq!(out, vec![Value::List(vec![Value::Int(30), Value::Int(10), Value::Int(20)])]);
    let mut h = TestHost::new();
    let mut it = interp();
    let o = it.run_line(&mut h, "[1] | each --parallel 0 x => x");
    o.result.unwrap();
}

#[test]
fn async_and_await_are_keywords_in_highlight() {
    let h = crate::highlight::highlight("let a = async { ls }\nawait all [a] timeout 5s", None);
    let classes: Vec<&str> = h.spans.iter().map(|s| s.class.name()).collect();
    assert!(classes.iter().filter(|c| **c == "kw").count() >= 3, "{classes:?}");
}
