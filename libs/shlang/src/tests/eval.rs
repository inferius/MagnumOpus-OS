//! Testy interpretu: hodnoty, operatory, roury, chyby, funkce, ulohy.

use super::host::{TestHost, fails, interp, ok, run_lines};
use crate::error::code;
use crate::value::Value;

fn one(src: &str) -> Value {
    let out = ok(src);
    assert_eq!(out.len(), 1, "{src}: {out:?}");
    out.into_iter().next().unwrap()
}

#[test]
fn literals_and_arithmetic() {
    assert_eq!(one("1 + 2 * 3"), Value::Int(7));
    assert_eq!(one("(1 + 2) * 3"), Value::Int(9));
    assert_eq!(one("7 / 2"), Value::Int(3));
    assert_eq!(one("7.0 / 2"), Value::Float(3.5));
    assert_eq!(one("100MB + 1KB"), Value::Size(100_001_000));
    assert_eq!(one("1KiB * 2"), Value::Size(2048));
    assert_eq!(one("1h - 30min"), Value::Duration(1_800_000_000_000));
    assert_eq!(one("\"a\" + \"b\""), Value::str("ab"));
    assert_eq!(one("[1, 2] + [3]"), Value::List(vec![Value::Int(1), Value::Int(2), Value::Int(3)]));
    assert_eq!(one("-5 + 2"), Value::Int(-3));
    assert_eq!(one("!true || false"), Value::Bool(false));
    assert_eq!(one("10 % 3"), Value::Int(1));
}

#[test]
fn no_hidden_conversions() {
    let e = fails("100MB > 5");
    assert_eq!(e.code, code::TYPE);
    assert!(e.hint.unwrap().contains("100MB"));
    assert_eq!(fails("\"a\" + 1").code, code::TYPE);
    assert_eq!(fails("1MB == 5").code, code::TYPE);
    assert_eq!(fails("if 1 { echo x }").code, code::TYPE);
    assert_eq!(fails("1 / 0").code, code::ARITH);
    assert_eq!(fails("9223372036854775807 + 1").code, code::ARITH);
}

#[test]
fn variables_and_interpolation() {
    let (h, _, o) = run_lines(&["let x = 5", "let s = \"x je {x * 2}\"", "echo $s", "x = x + 1", "echo $x"]);
    o.result.unwrap();
    assert_eq!(h.text(), vec!["x je 10", "6"]);
    assert_eq!(fails("y = 1").code, code::UNKNOWN);
    assert_eq!(fails("echo $nic").code, code::UNKNOWN);
}

#[test]
fn bare_word_variable_hint() {
    let (_, _, o) = run_lines(&["let x = 1", "echo x"]);
    assert!(o.warnings.iter().any(|w| w.message.contains("myslel jsi $x?")), "{:?}", o.warnings);
}

#[test]
fn pipelines_over_files() {
    let out = ok("ls | where size > 5B | select name size");
    let names: Vec<String> = match &out[0] {
        Value::List(v) => v.iter().map(|r| r.field("name").unwrap().to_text()).collect(),
        other => panic!("{other:?}"),
    };
    assert_eq!(names, vec!["b.txt", "data.json"]);
    assert_eq!(one("ls -r | where ext == \"pdf\" | count"), Value::Int(2));
    assert_eq!(one("ls -a | count"), Value::Int(5));
    assert_eq!(one("ls | count"), Value::Int(4));
    assert_eq!(one("(ls docs | sort size -d | first).name"), Value::str("big.pdf"));
    assert_eq!(one("ls *.txt | count"), Value::Int(2));
    assert_eq!(one("ls().where(it => it.kind == \"dir\").count()"), Value::Int(1));
    // sloupec ze seznamu zaznamu
    assert_eq!(one("(ls *.txt).name"), Value::List(vec![Value::str("a.txt"), Value::str("b.txt")]));
}

#[test]
fn where_with_outer_variable() {
    let (h, _, o) = run_lines(&["let limit = 5B", "ls | where size > limit | count"]);
    o.result.unwrap();
    assert_eq!(h.out, vec![Value::Int(2)]);
}

#[test]
fn bytes_are_not_records() {
    let e = fails("open data.json | where age > 18");
    assert_eq!(e.code, code::TYPE);
    assert!(e.hint.unwrap().contains("from json"));
    let out = one("open data.json | from json | where age >= 18 | each p => p.name");
    assert_eq!(out, Value::List(vec![Value::str("Eva")]));
    // zaznamy do souboru jen vyslovne
    let e = fails("ls > list.txt");
    assert_eq!(e.code, code::TYPE);
    assert!(e.hint.unwrap().contains("to text"));
}

#[test]
fn redirect_and_append() {
    let (mut h, _, o) = run_lines(&["echo a > /tmp/o.txt", "echo b >> /tmp/o.txt", "open /tmp/o.txt | from text"]);
    o.result.unwrap();
    assert_eq!(h.out, vec![Value::str("ab")]);
    assert_eq!(crate::host::Fs::read(&mut h.fs, "/tmp/o.txt").unwrap(), b"ab");
    let out = ok("lines < b.txt");
    assert_eq!(out, vec![Value::List(vec![Value::str("beta beta")])]);
}

#[test]
fn errors_propagate_and_stop() {
    let (h, _, o) = run_lines(&["echo pred\nrm neni.txt\necho po"]);
    let e = o.result.unwrap_err();
    assert_eq!(e.code, "NOT_FOUND");
    assert_eq!(h.text(), vec!["pred"]);
}

#[test]
fn try_forms() {
    assert_eq!(one("try { rm neni.txt } else e => e.code"), Value::str("NOT_FOUND"));
    assert_eq!(one("rm neni.txt || echo nevadi"), Value::str("nevadi"));
    let (h, _, o) = run_lines(&["let r = try rm neni.txt", "echo $r.failed $r.error.code"]);
    o.result.unwrap();
    assert_eq!(h.text(), vec!["true NOT_FOUND"]);
    // `?` rozbali vysledek nebo chybu vyvola
    assert_eq!(fails("(try rm neni.txt)?").code, "NOT_FOUND");
    assert_eq!(one("(try 5)?"), Value::Int(5));
    assert_eq!(one("try error \"x\" --code MOJE else e => e.code"), Value::str("MOJE"));
    // chyba ma pricinu
    assert_eq!(one("(try rm neni.txt).error.cause.code"), Value::str("NOT_FOUND"));
}

#[test]
fn and_or_semantics() {
    assert_eq!(one("mkdir /tmp/x && exists /tmp/x"), Value::Bool(true));
    assert_eq!(fails("rm neni.txt && echo ne").code, "NOT_FOUND");
    assert_eq!(one("false || 3"), Value::Int(3));
    assert_eq!(one("true && 3"), Value::Int(3));
}

#[test]
fn functions_and_closures() {
    let (h, _, o) = run_lines(&[
        "fn fact(n) { if n <= 1 { return 1 }\n n * (fact (n - 1)) }",
        "fact 10",
        "fact(5)",
        "let add = (a, b) => a + b",
        "add(2, 3)",
        "let k = 10",
        "let f = x => x + k",
        "k = 0",
        "f(1)",
        "[1, 2, 3] | each x => x * x",
        "fn g(a, b) { a - b }\ng --b=1 --a=5",
    ]);
    o.result.unwrap();
    assert_eq!(
        h.out,
        vec![
            Value::Int(3628800),
            Value::Int(120),
            Value::Int(5),
            Value::Int(11),
            Value::List(vec![Value::Int(1), Value::Int(4), Value::Int(9)]),
            Value::Int(4),
        ]
    );
    assert_eq!(fails("fn r(n) { r(n) }\nr(1)").code, code::DEPTH);
}

#[test]
fn loops_and_blocks() {
    let (h, _, o) = run_lines(&[
        "for i in (range 0 3) {\n echo $i\n}",
        "let n = 0\nwhile n < 10 { n = n + 1\n if n == 3 { break } }\necho $n",
    ]);
    o.result.unwrap();
    assert_eq!(h.text(), vec!["0", "1", "2", "3"]);
    assert_eq!(one("let x = if 1 > 2 { \"a\" } else { \"b\" }\necho $x"), Value::str("b"));
}

#[test]
fn records_and_fields() {
    assert_eq!(one("{a: 1, b: {c: 2}}.b.c"), Value::Int(2));
    let e = fails("{a: 1}.b");
    assert_eq!(e.code, code::UNKNOWN);
    assert!(e.hint.unwrap().contains("a"));
    assert_eq!(one("[10, 20, 30][-1]"), Value::Int(30));
    assert_eq!(one("ls | group ext | count"), Value::Int(3));
    assert_eq!(one("[3, 1, 2] | sort | first"), Value::Int(1));
    assert_eq!(one("[1, 2, 3] | sum"), Value::Int(6));
    assert_eq!(one("ls docs | sum size"), Value::Size(3001));
}

#[test]
fn text_commands() {
    assert_eq!(one("\"a,b\" | split \",\" | join \"-\""), Value::str("a-b"));
    assert_eq!(one("\"  x \".trim().upper()"), Value::str("X"));
    assert_eq!(one("\"abc\" | contains b"), Value::Bool(true));
    assert_eq!(one("\"abc\".starts_with(\"ab\")"), Value::Bool(true));
    assert_eq!(one("[1, 2] | contains 2"), Value::Bool(true));
    assert_eq!(one("\"aXa\" | replace X Y"), Value::str("aYa"));
    assert_eq!(one("{a: 1} | to json"), Value::str("{\n  \"a\": 1\n}"));
    assert_eq!(one("1MB | describe"), Value::str("size"));
}

#[test]
fn unknown_command_suggests() {
    let e = fails("lss");
    assert_eq!(e.code, code::UNKNOWN);
    assert!(e.hint.unwrap().contains("ls"));
}

#[test]
fn ctrl_c_interrupts_and_cannot_be_caught() {
    let mut h = TestHost::new();
    let mut it = interp();
    h.interrupt_after = Some(5);
    let o = it.run_line(&mut h, "try { while true { let z = 1 } } else e => echo chyceno");
    assert_eq!(o.result.unwrap_err().code, code::INTERRUPTED);
    assert!(h.out.is_empty());
}

#[test]
fn background_jobs() {
    let (mut h, mut it, o) = run_lines(&["let x = 2", "x * 21 &", "jobs | count"]);
    o.result.unwrap();
    assert_eq!(h.out.last(), Some(&Value::Int(1)));
    let done = it.run_pending_jobs(&mut h);
    assert_eq!(done.len(), 1);
    assert_eq!(done[0].1.as_ref().unwrap(), &Value::Int(42));
    let o = it.run_line(&mut h, "wait 1");
    o.result.unwrap();
    assert_eq!(h.out.last(), Some(&Value::Int(42)));
    let o = it.run_line(&mut h, "bg { 1 }\nkill 2\n(jobs | last).state");
    o.result.unwrap();
    assert_eq!(h.out.last(), Some(&Value::str("killed")));
}

#[test]
fn history_with_context() {
    let (_, it, _) = run_lines(&["cd docs", "ls", "rm neni"]);
    let e = &it.history.entries;
    assert_eq!(e.len(), 3);
    assert_eq!(e[1].cwd, "/home/docs");
    assert!(e[1].ok && !e[2].ok);
    assert!(e[1].duration.is_some());
    assert_eq!(it.history.search("LS", None).len(), 1);
}

#[test]
fn help_and_explain() {
    let t = one("help ls").to_text();
    assert!(t.contains("file-list") && t.contains("--recursive"));
    let t = one("explain NOT_FOUND").to_text();
    assert!(t.contains("NOT_FOUND"));
    let t = one("explain SH_PARSE").to_text();
    assert!(t.contains("SH_PARSE"));
    let t = one("try { rm x } else e => explain $e").to_text();
    assert!(t.contains("NOT_FOUND"));
    assert_eq!(fails("explain NECO_DIVNEHO").code, code::UNKNOWN);
}

#[test]
fn stack_per_call_is_small() {
    let measure = |n: i64| {
        crate::eval::STACK_MIN.with(|m| m.set(usize::MAX));
        let x = 0u8;
        let top = &x as *const u8 as usize;
        let src = format!("fn r(n) {{ if n == 0 {{ 0 }} else {{ r(n - 1) }} }}
r({n})");
        let _ = ok(&src);
        top - crate::eval::STACK_MIN.with(|m| m.get())
    };
    let a = measure(2);
    let b = measure(12);
    // Hlida, ze interpret zustava prekladany s optimalizaci (Cargo.toml,
    // profile.dev.package.shlang) a ze jedna uroven volani je mala.
    assert!((b - a) / 10 < 16 * 1024, "jedna uroven volani zabira {} B zasobniku", (b - a) / 10);
}
