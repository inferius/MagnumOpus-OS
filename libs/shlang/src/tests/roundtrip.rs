//! Testy prevodu `B -> strom -> C -> strom` (a opacne): stejny strom.

use crate::fmt::{FmtOptions, format};
use crate::parse::{Options, Style, parse};
use crate::sig::Builtins;

pub fn tree(src: &str, style: Style) -> crate::ast::Script {
    match parse(src, &Options { style: Some(style), sigs: &Builtins, vars: vec![], script: false }) {
        Ok(p) => p.script,
        Err(e) => panic!("{}\n--- zdroj ({style:?}):\n{src}", e.report(src)),
    }
}

fn fmt(t: &crate::ast::Script, style: Style) -> String {
    format(t, &FmtOptions { style, sigs: &Builtins, vars: vec![] })
}

/// Overi prevod tam a zpet pro zdroj v zapisu `style`.
pub fn check(src: &str, style: Style) -> (String, String) {
    let t = tree(src, style);
    let b = fmt(&t, Style::B);
    let c = fmt(&t, Style::C);
    let tb = tree(&b, Style::B);
    let tc = tree(&c, Style::C);
    assert_eq!(t, tb, "B se lisi\n--- zdroj:\n{src}\n--- B:\n{b}");
    assert_eq!(t, tc, "C se lisi\n--- zdroj:\n{src}\n--- C:\n{c}");
    // Formatovani je ustalene.
    assert_eq!(fmt(&tb, Style::B), b);
    assert_eq!(fmt(&tc, Style::C), c);
    (b, c)
}

#[test]
fn basic_examples() {
    let (b, c) = check("ls ~/Dok -r | where size > 100MB", Style::B);
    assert_eq!(b.trim(), "ls ~/Dok -r | where size > 100MB");
    assert_eq!(c.trim(), "ls(\"~/Dok\", recursive: true).where(it => it.size > 100MB)");
}

/// Priklady v zapisu B (z dokumentace a dalsi okrajove pripady).
pub const B_EXAMPLES: &[&str] = &[
    "ls ~/Dokumenty -r | where size > 100MB && mtime < now() - 1y | sort size -d",
    "let velke = ls ~/Dokumenty -r | where size > 100MB",
    "for f in (ls ~/Fotky | where ext == \"jpg\") {\n    echo $f.name\n}",
    "if (df /).free < 5GB {\n    log \"Dochazi misto\"\n}",
    "ls ~/Dok -r | where (size > 100MB && ext == \"pdf\")",
    "ls | each f => rm $f.path",
    "ls | each f => f.size * 2",
    "try {\n    rm a.txt\n} else e => log \"neslo: {e.message}\"",
    "rm a.txt || echo \"nevadi\"",
    "let r = try rm a.txt",
    "mkdir x && cd x",
    "echo a b c > out.txt",
    "echo dalsi >> out.txt",
    "lines < data.txt",
    "open data.json | from json | where age >= 18 | select name age",
    "ls *.txt | count",
    "echo \"a {1 + 2} b\"",
    "echo ~/Fotky/zmensene/{1 + 1}",
    "let x = [1, 2, 3]",
    "let rec = {name: \"a\", size: 1KB, \"s mezerou\": true}",
    "fn double(x) {\n    x * 2\n}",
    "double 21",
    "let f = (a, b) => a + b",
    "let g = x => {\n    let y = x + 1\n    y * 2\n}",
    "while false {\n    break\n}",
    "sleep 30s &",
    "bg {\n    sleep 1s\n    echo hotovo\n}",
    "echo -5 -1.5MB \"-r\" --",
    "range 0 10 | where it % 2 == 0 | sum",
    "let limit = 1MB\nls | where size > limit",
    "let size = 5\nls | where f => f.size > size",
    "echo (1 + 2) \"text\" true null 3.5",
    "# komentar\nls # za prikazem",
    "if true {\n    echo a\n} else if false {\n    echo b\n} else {\n    echo c\n}",
    "ls | where name.ends_with(\".rs\")",
    "let s = \"abc\".upper()",
    "echo [1, 2].count()",
    "let r = try rm a.txt\nif r.failed {\n    echo $r.error.code\n}",
    "echo (try (error \"x\") else e => e.code)",
    "preview {\n    rm *.tmp\n}",
    "ls | where f => f.size > 1KB",
    "history | where line.contains(\"ls\") | first 5",
    "let n = -(3 + 4)",
    "let b = !(1 < 2) && true || false",
    "let v = [1, 2][0]",
    "ls | sort mtime | last 3 | select name mtime",
    "ls | group ext | each g => {ext: g.key, n: g.items.count()}",
    "x_unknown --flag -abc --v=5 pos",
];

/// Priklady v zapisu C.
pub const C_EXAMPLES: &[&str] = &[
    "ls(\"~/Dok\", recursive: true).where(f => f.size > 100MB)",
    "let n = ls().count()",
    "ls(\"~\")\n    .where(it => it.ext == \"txt\")\n    .sort(\"size\", desc: true)",
    "for f in ls().where(it => it.size > 1MB) {\n    rm(f.path)\n}",
    "let r = try rm(\"a.txt\") else e => e.code",
    "fn sq(x) {\n    return x * x\n}",
    "let y = sq(4)?",
    "file_list(\"/\").count()",
    "echo(glob(\"*.rs\"))",
    "let f = it => it.size > 1
ls().where(f)",
    "fn f2(a, x) {
    a + x
}
f2(\"a\" + \"b\", x: 2)",
    "bg {\n    sleep(1s)\n}",
    "let t = now() - 1y",
    "ls().each((a) => {\n    log(a.name)\n})",
];

/// Spusti text na cistem testovacim FS; vrati vystup a kod chyby.
fn exec(src: &str) -> (Vec<crate::value::Value>, Option<String>, Vec<String>) {
    let (h, _, o) = super::host::run_lines(&[src]);
    let mut files: Vec<String> = Vec::new();
    for p in ["/home/out.txt", "/home/a.txt", "/home/x", "/home/n.txt"] {
        if h.fs.exists(p) {
            files.push(p.into());
        }
    }
    (h.out, o.result.err().map(|e| e.code), files)
}

/// Stejny strom i stejny vysledek pro zdroj, tvar B i tvar C.
fn check_all(src: &str, style: Style) {
    let (b, c) = check(src, style);
    let r0 = exec(src);
    assert_eq!(r0, exec(&b), "vysledek B se lisi
{src}
--- B:
{b}");
    assert_eq!(r0, exec(&c), "vysledek C se lisi
{src}
--- C:
{c}");
}

#[test]
fn b_examples_roundtrip() {
    for src in B_EXAMPLES {
        check_all(src, Style::B);
    }
}

#[test]
fn c_examples_roundtrip() {
    for src in C_EXAMPLES {
        check_all(src, Style::C);
    }
}

#[test]
fn b_accepts_c_statements() {
    // Rozhodnuti 4A: prikaz ve tvaru C jde napsat i v zapisu B.
    for src in C_EXAMPLES {
        let t = tree(src, Style::C);
        let tb = tree(src, Style::B);
        assert_eq!(t, tb, "{src}");
    }
}

#[test]
fn row_collision_is_error() {
    // Promenna se jmenem pole radku v podmince: chyba, ne tiche pouziti.
    let src = "let size = 5
ls | where size > 1";
    let e = parse(src, &Options { style: None, sigs: &Builtins, vars: vec![], script: false }).unwrap_err();
    assert!(e.message.contains("promenna i pole"), "{e:?}");
    assert!(e.hint.unwrap().contains("it.size"));
}

/// Bloky ```shell-b a ```shell-c z dokumentace.
fn doc_blocks(md: &str) -> Vec<(String, Style)> {
    let mut out = Vec::new();
    let mut cur: Option<(String, Style)> = None;
    for line in md.lines() {
        match &mut cur {
            None => {
                let t = line.trim_start();
                if t == "```shell-b" {
                    cur = Some((String::new(), Style::B));
                } else if t == "```shell-c" {
                    cur = Some((String::new(), Style::C));
                }
            }
            Some((buf, st)) => {
                if line.trim_start() == "```" {
                    out.push((core::mem::take(buf), *st));
                    cur = None;
                } else {
                    buf.push_str(line);
                    buf.push('\n');
                }
            }
        }
    }
    out
}

#[test]
fn documentation_examples_roundtrip() {
    for (name, md) in [
        ("cs/navrh-shell.md", include_str!("../../../../docs/cs/navrh-shell.md")),
        ("en/shell-design.md", include_str!("../../../../docs/en/shell-design.md")),
    ] {
        let blocks = doc_blocks(md);
        assert!(!blocks.is_empty() || name.starts_with("en"), "{name}: zadne priklady");
        for (src, style) in blocks {
            check(&src, style);
        }
    }
}
