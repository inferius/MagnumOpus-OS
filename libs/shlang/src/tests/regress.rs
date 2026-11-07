//! Regresni testy k nalezum z review (ztrata dat, pady, prevod zapisu).

use super::host::{TestHost, interp, run_lines};
use super::roundtrip::check;
use crate::error::code;
use crate::parse::{Options, Style, parse};
use crate::sig::Builtins;

#[test]
fn copy_onto_itself_keeps_source() {
    for line in ["cp a.txt a.txt", "cp a.txt .", "mv a.txt a.txt"] {
        let (h, _, o) = run_lines(&[line]);
        assert!(o.result.is_err(), "{line}");
        assert_eq!(h.fs.get("/home/a.txt"), Some(&b"alfa\n"[..]), "{line}");
    }
    // do sve podslozky: chyba, ne nekonecna rekurze
    for line in ["cp -r docs docs", "cp -r docs docs/sub"] {
        let (h, _, o) = run_lines(&[line]);
        assert!(o.result.is_err(), "{line}");
        assert!(h.fs.exists("/home/docs/big.pdf"));
    }
}

#[test]
fn replacing_file_by_dir_and_back_is_undoable() {
    let mut h = TestHost::new();
    let mut it = interp();
    it.run_line(&mut h, "rm a.txt && mkdir a.txt").result.unwrap();
    assert!(h.fs.is_dir("/home/a.txt"));
    it.run_line(&mut h, "undo").result.unwrap();
    assert_eq!(h.fs.get("/home/a.txt"), Some(&b"alfa\n"[..]));

    it.run_line(&mut h, "rm -r docs && echo hi > docs").result.unwrap();
    assert_eq!(h.fs.get("/home/docs"), Some(&b"hi"[..]));
    it.run_line(&mut h, "undo").result.unwrap();
    assert!(h.fs.is_dir("/home/docs"));
    assert_eq!(h.fs.get("/home/docs/big.pdf").map(|d| d.len()), Some(3000));
}

#[test]
fn arithmetic_never_panics() {
    let min = "(-9223372036854775807B - 1B)";
    for src in [format!("{min} / -1"), format!("-{min}"), "(-9223372036854775807s - 1s) / -1".into()] {
        let (_, _, o) = run_lines(&[&src]);
        assert_eq!(o.result.unwrap_err().code, code::ARITH, "{src}");
    }
    // desetinne cislo mimo rozsah f64 je chyba, ne nekonecno
    let e = parse(&format!("let x = {}.0", "9".repeat(400)), &opts()).unwrap_err();
    assert_eq!(e.code, code::PARSE);
}

fn opts() -> Options<'static> {
    Options { style: None, sigs: &Builtins, vars: vec![], script: false }
}

#[test]
fn deep_nesting_is_an_error_not_a_crash() {
    let srcs = [
        format!("{}true", "!".repeat(100_000)),
        format!("let f = {}1", "x => ".repeat(50_000)),
        format!("{}{{ 2 }}", "if true { 1 } else ".repeat(20_000)),
        format!("{}1{}", "(".repeat(50_000), ")".repeat(50_000)),
    ];
    for s in srcs {
        let e = parse(&s, &opts()).unwrap_err();
        assert_eq!(e.code, code::DEPTH);
    }
}

#[test]
fn completion_cursor_inside_char() {
    let mut fs = crate::vfs::mem::MemFs::new();
    let mut ctx = crate::complete::Ctx { sigs: &Builtins, vars: vec![], fns: vec![], fs: &mut fs, cwd: "/".into(), home: "/".into() };
    let _ = crate::complete::complete("čč", 1, &mut ctx);
    let _ = crate::complete::complete("ls čč", 4, &mut ctx);
}

#[test]
fn review_roundtrips() {
    for src in [
        "ls | where (f => f.size > 1)",
        "echo \"=a\"",
        "echo =a",
        "./run.sh x",
        "~/bin/tool -v",
        "my-tool a",
        "my_tool a",
        "exec a",
        "let y = 1\n./config/run",
        "ls | each f => {\n    echo \":x\"\n}",
        "each f => {\n    ~/bin/run\n}",
        "try {\n    ~/bin/run\n}",
    ] {
        check(src, Style::B);
    }
    for src in ["ls().each(f => { echo(\":x\") })", "ls().where(it => (y => y))", "exec(\"./x\", 1, v: true)"] {
        check(src, Style::C);
    }
    // `.pole` na dalsim radku neni pokracovani vyrazu
    let p = parse("let y = 1\n.config/run", &opts()).unwrap();
    assert_eq!(p.script.body.stmts.len(), 2);
    // `where (f => ...)` je jedna closure
    let (h, _, o) = run_lines(&["ls | where (f => f.size > 5B) | count"]);
    o.result.unwrap();
    assert_eq!(h.out, vec![crate::value::Value::Int(2)]);
}
