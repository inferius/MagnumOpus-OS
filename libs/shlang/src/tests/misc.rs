//! Testy JSON, tabulek, doplnovani a dokumentace kodu chyb.

use crate::complete::{CandKind, Ctx, complete};
use crate::error::code;
use crate::sig::Builtins;
use crate::value::Value;
use crate::vfs::mem::MemFs;

#[test]
fn json_roundtrip() {
    let v = crate::json::parse(r#"{"a": [1, 2.5, "x\ná"], "b": null, "c": true}"#).unwrap();
    assert_eq!(v.field("b"), Some(Value::Null));
    let s = crate::json::to_json(&v, false).unwrap();
    assert_eq!(crate::json::parse(&s).unwrap(), v);
    assert!(crate::json::parse("[1,").is_err());
    assert!(crate::json::to_json(&Value::Bytes(vec![1]), false).is_err());
}

#[test]
fn table_rendering() {
    let rows = Value::List(vec![
        Value::record(&[("name", Value::str("a")), ("size", Value::Size(1500))]),
        Value::record(&[("name", Value::str("dlouhe")), ("size", Value::Size(2))]),
    ]);
    let t = crate::table::render(&rows);
    assert_eq!(t, "name    size\n------  ------\na       1.5 KB\ndlouhe     2 B\n");
}

fn ctx_complete(line: &str, vars: &[&str]) -> Vec<String> {
    let mut fs = MemFs::new();
    fs.put("/home/docs/x.txt", b"");
    fs.put("/home/dalsi.txt", b"");
    fs.put("/home/.skryty", b"");
    fs.put("/home/moje dokumenty/a.txt", b"");
    let mut ctx = Ctx {
        sigs: &Builtins,
        vars: vars.iter().map(|s| s.to_string()).collect(),
        fns: vec!["muj".into()],
        fs: &mut fs,
        cwd: "/home".into(),
        home: "/home".into(),
    };
    let (_, c) = complete(line, line.len(), &mut ctx);
    c.into_iter().map(|c| c.text).collect()
}

#[test]
fn completion_by_type() {
    assert!(ctx_complete("l", &[]).contains(&"ls".into()));
    assert!(ctx_complete("file-l", &[]).contains(&"file-list".into()));
    assert!(ctx_complete("mu", &[]).contains(&"muj".into()));
    assert_eq!(ctx_complete("ls --r", &[]), vec!["--recursive"]);
    assert_eq!(ctx_complete("ls d", &[]), vec!["dalsi.txt", "docs/"]);
    assert_eq!(ctx_complete("ls docs/", &[]), vec!["docs/x.txt"]);
    assert_eq!(ctx_complete("echo $li", &["limit"]), vec!["$limit"]);
    let f = ctx_complete("ls | where s", &["sx"]);
    assert_eq!(f, vec!["size", "sx"]);
    assert_eq!(ctx_complete("ls | sort m", &[]), vec!["mtime"]);
    assert!(ctx_complete("ls | first ", &[]).is_empty());
    let mut fs = MemFs::new();
    let mut ctx = Ctx { sigs: &Builtins, vars: vec![], fns: vec![], fs: &mut fs, cwd: "/".into(), home: "/".into() };
    let (_, c) = complete("ec", 2, &mut ctx);
    assert_eq!(c[0].kind, CandKind::Command);
}

#[test]
fn every_shell_error_code_is_documented() {
    for c in code::ALL {
        assert!(crate::explain::documented(c), "kod {c} chybi v docs/cs/navrh-shell.md (kapitola Chyby shellu)");
    }
    let en = include_str!("../../../../docs/en/shell-design.md");
    for c in code::ALL {
        assert!(en.contains(&format!("### `{c}`")), "kod {c} chybi v docs/en/shell-design.md");
    }
}

#[test]
fn explain_log_message() {
    let t = crate::explain::explain("initfs nenalezen (bla), jadro nespusti zadny proces").unwrap();
    assert!(t.contains("initfs"));
}

#[test]
fn path_completion_in_pipeline_absolute_home_and_quotes() {
    // Za rourou u argumentu typu cesta (save), absolutni cesta.
    assert_eq!(ctx_complete("ls | select name | to json | save /ho", &[]), vec!["/home/"]);
    assert_eq!(ctx_complete("ls | to json | save /home/d", &[]), vec!["/home/dalsi.txt", "/home/docs/"]);
    // Relativni a ~.
    assert_eq!(ctx_complete("open | x; cp do", &[]), vec!["docs/"]);
    assert_eq!(ctx_complete("cd ~/do", &[]), vec!["~/docs/"]);
    // Obsah adresare po doplneni.
    assert_eq!(ctx_complete("open docs/", &[]), vec!["docs/x.txt"]);
    // Jmeno s mezerou v uvozovkach (slozka zustane otevrena, soubor se uzavre).
    assert_eq!(ctx_complete("open mo", &[]), vec!["\"moje dokumenty/"]);
    assert_eq!(ctx_complete("open \"moje dokumenty/", &[]), vec!["\"moje dokumenty/a.txt\""]);
}

#[test]
fn bare_file_names_with_dash_dot_and_digits_are_one_word() {
    use super::host::run_lines;
    for line in [
        "open boot-000001.log",
        "open snimek-000001-2026-11-08.bmp",
        "cp boot-000001.log /tmp/x-1.2.log",
        "ls docs-2.0",
        "cd 2026-11-13",
        "run skript-1.mos",
    ] {
        let p = crate::parse(line, &crate::Options { style: None, sigs: &crate::sig::Builtins, vars: Vec::new(), script: false });
        assert!(p.is_ok(), "{line}: {:?}", p.err());
    }
    let _ = run_lines;
}

#[test]
fn run_of_non_script_says_so() {
    use super::host::{TestHost, interp};
    let mut h = TestHost::new();
    h.fs.put("/home/boot-000001.log", b"[    0.000000] cpu0 I sysinfo: stroj\n");
    let mut it = interp();
    let o = it.run_line(&mut h, "run boot-000001.log");
    let e = o.result.unwrap_err();
    assert_eq!(e.code, crate::error::code::ARGS);
    assert!(e.message.contains("není skript shellu") && e.message.contains(".mos"), "{}", e.message);
}

/// Rada u preklepu nikdy nenabidne nicivy prikaz bez skoro shody.
#[test]
fn typo_hint_never_offers_destructive_commands() {
    let near = |n: &str| crate::eval::call::nearest(n, crate::sig::BUILTINS.iter().map(|b| b.name), |_| 0);
    assert_eq!(near("fmt"), None, "fmt -> rm by svadelo k mazani");
    assert_eq!(near("rn"), Some("rm"), "skoro shoda se stejnym zacatkem");
    assert_eq!(near("lss"), Some("ls"));
    assert_eq!(near("histroy"), Some("history"));
    assert_ne!(near("kil"), None);
    assert_eq!(near("xill"), None, "kill jen se stejnym zacatkem");
    // Pri stejne vzdalenosti vyhraje casteji pouzivany.
    let n = crate::eval::call::nearest("ech", ["echo", "each"].into_iter(), |c| if c == "each" { 5 } else { 0 });
    assert_eq!(n, Some("each"));
    // Hlaska bez rady odkaze na help.
    let e = super::host::fails("fmt x");
    assert!(e.hint.unwrap_or_default().contains("help"));
}

/// `cd -` / `cd -N` zpet, `cd +` vpred, `cd --history` posledni slozky.
#[test]
fn cd_back_forward_and_history() {
    use super::host::run_lines;
    let pwd = |lines: &[&str]| {
        let (h, it, o) = run_lines(lines);
        o.result.unwrap();
        let _ = h;
        it.cwd
    };
    assert_eq!(pwd(&["cd docs", "cd /tmp", "cd -"]), "/home/docs");
    assert_eq!(pwd(&["cd docs", "cd /tmp", "cd -2"]), "/home");
    assert_eq!(pwd(&["cd docs", "cd /tmp", "cd -", "cd +"]), "/tmp");
    assert_eq!(pwd(&["cd docs", "cd /tmp", "cd -2", "cd +"]), "/home/docs");
    let (h, _, o) = run_lines(&["cd docs", "cd /tmp", "cd --history"]);
    o.result.unwrap();
    let paths: Vec<String> = match h.out.last() {
        Some(crate::value::Value::List(rows)) => rows.iter().map(|r| r.field("path").unwrap().to_text()).collect(),
        o => panic!("{o:?}"),
    };
    assert_eq!(paths, vec!["/home/docs", "/home"]);
    let e = super::host::fails("cd -");
    assert!(e.message.contains("není kam jít zpět"), "{}", e.message);
}
