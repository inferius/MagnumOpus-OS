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
