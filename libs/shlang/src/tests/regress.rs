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
    let (h, _, o) = run_lines(&["ls | where (f => f.kind == \"file\" && f.size > 5B) | count"]);
    o.result.unwrap();
    assert_eq!(h.out, vec![crate::value::Value::Int(2)]);
}

// ---- druhe review: needs, zamek a undo u programu ----

#[test]
fn background_job_keeps_script_needs() {
    let mut h = TestHost::new();
    let mut it = interp();
    h.fs.put("/home/bg.sh", b"lang 1\nneeds {\n    read: [\"~/docs\"]\n}\nbg {\n    rm ~/a.txt\n}\n");
    it.run_line(&mut h, "run bg.sh").result.unwrap();
    // Skript skoncil; uloha dobehne az ted, ale stale s jeho omezenim.
    let done = it.run_pending_jobs(&mut h);
    assert_eq!(done.len(), 1);
    assert_eq!(done[0].1.as_ref().unwrap_err().code, code::NEEDS);
    assert_eq!(h.fs.get("/home/a.txt"), Some(&b"alfa\n"[..]));
    // Totez pres `wait` v interaktivni relaci.
    h.fs.put("/home/bg2.sh", b"lang 1\nneeds {\n    read: [\"~/docs\"]\n}\nrm ~/b.txt &\n");
    it.run_line(&mut h, "run bg2.sh").result.unwrap();
    assert_eq!(it.run_line(&mut h, "wait 2").result.unwrap_err().code, code::NEEDS);
    assert!(h.fs.exists("/home/b.txt"));
}

#[test]
fn nested_script_gets_at_most_callers_rights() {
    let mut h = TestHost::new();
    let mut it = interp();
    // B chce zapis tam, kde A smi jen cist: odmitnuto.
    h.fs.put("/home/b.sh", b"lang 1\nneeds {\n    write: [\"~\"]\n}\necho x > ~/a.txt\n");
    h.fs.put("/home/a.sh", b"lang 1\nneeds {\n    read: [\"~\"]\n}\nrun ~/b.sh\n");
    assert_eq!(it.run_line(&mut h, "run a.sh").result.unwrap_err().code, code::NEEDS);
    assert_eq!(h.fs.get("/home/a.txt"), Some(&b"alfa\n"[..]));
    // B chce sit, A ji nema: odmitnuto.
    h.fs.put("/home/n.sh", b"lang 1\nneeds {\n    read: [\"~\"]\n    net: [\"example.org\"]\n}\necho x\n");
    h.fs.put("/home/a2.sh", b"lang 1\nneeds {\n    read: [\"~\"]\n    net: none\n}\nrun ~/n.sh\n");
    assert_eq!(it.run_line(&mut h, "run a2.sh").result.unwrap_err().code, code::NEEDS);
    // B chce cist uvnitr cesty, kam A smi zapisovat: povoleno (prusecik).
    h.fs.put("/home/c.sh", b"lang 1\nneeds {\n    read: [\"/tmp\"]\n}\nls /tmp | count\n");
    h.fs.put("/home/a3.sh", b"lang 1\nneeds {\n    read: [\"~\"]\n    write: [\"/tmp\"]\n}\nrun ~/c.sh\n");
    it.run_line(&mut h, "run a3.sh").result.unwrap();
}

#[test]
fn danger_lock_applies_before_external_program() {
    let mut h = TestHost::new();
    let mut it = interp();
    it.lock_threshold = 3;
    h.answer = false;
    let o = it.run_line(&mut h, "for i in (range 0 5) { echo $i > f{i}.txt }\ntool");
    assert_eq!(o.result.unwrap_err().code, code::REJECTED);
    assert!(h.execs.is_empty(), "program se po odmitnuti nespusti");
    assert!(!h.fs.exists("/home/f0.txt"));
    assert_eq!(h.asked.len(), 1);
}

#[test]
fn undo_after_read_only_program_reverts_previous_command() {
    let mut h = TestHost::new();
    let mut it = interp();
    it.run_line(&mut h, "rm a.txt").result.unwrap();
    it.run_line(&mut h, "ro-tool").result.unwrap();
    it.run_line(&mut h, "undo").result.unwrap();
    assert_eq!(h.fs.get("/home/a.txt"), Some(&b"alfa\n"[..]), "undo vrati rm, ne prazdny zaznam programu");
    // Program s plnymi pravy, ktery zapise mimo domov, jde vratit take.
    it.run_line(&mut h, "tool").result.unwrap();
    assert!(h.fs.exists("/home/tool.out"));
    it.run_line(&mut h, "undo").result.unwrap();
    assert!(!h.fs.exists("/home/tool.out"));
}
