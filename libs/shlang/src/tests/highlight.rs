//! Testy zvyraznovace (spolecny pro terminal i dokumentaci).

use crate::highlight::{Class, Ctx, highlight};
use crate::sig::Builtins;

/// Useky jako "trida:text" (bez mezer a obycejneho textu).
fn show(src: &str, ctx: Option<&Ctx>) -> Vec<String> {
    highlight(src, ctx).spans.iter().map(|s| format!("{}:{}", s.class.name(), &src[s.start..s.end])).collect()
}

fn ctx_with<'a>(vars: &'a [String], cmds: &'a [String]) -> Ctx<'a> {
    Ctx { sigs: &Builtins, vars, commands: cmds }
}

#[test]
fn command_pipeline_b() {
    let v = show("ls -r ~/foto | where size > 5MB | sort mtime  # velke", None);
    assert_eq!(
        v,
        [
            "cmd:ls", "flag:-r", "path:~/foto", "pipe:|", "cmd:where", "field:size", "op:>", "num:5MB", "pipe:|",
            "cmd:sort", "comment:# velke"
        ]
    );
}

#[test]
fn where_condition_is_an_expression_until_the_pipe() {
    let v = show("ls -r | where size > 100MB && mtime < now - 1y | sort size", None);
    assert_eq!(
        v,
        [
            "cmd:ls", "flag:-r", "pipe:|", "cmd:where", "field:size", "op:>", "num:100MB", "op:&&", "field:mtime",
            "op:<", "field:now", "op:-", "num:1y", "pipe:|", "cmd:sort"
        ]
    );
}

#[test]
fn strings_interpolation_keywords_vars() {
    let vars = ["f".to_string()];
    let ctx = ctx_with(&vars, &[]);
    let v = show("for f in (ls *.jpg) { echo \"soubor {f.name}\" $f }", Some(&ctx));
    assert_eq!(
        v,
        [
            "kw:for", "var:f", "kw:in", "op:(", "cmd:ls", "path:*.jpg", "op:)", "op:{", "cmd:echo", "str:\"soubor ",
            "interp:{", "var:f", "op:.", "field:name", "interp:}", "str:\"", "var:$f", "op:}"
        ]
        .iter()
        .filter(|s| !s.starts_with("plain"))
        .map(|s| s.to_string())
        .collect::<Vec<_>>()
    );
}

#[test]
fn unknown_command_only_with_context() {
    // Lexikalne (okno pri psani): kazde slovo na miste prikazu je prikaz.
    assert_eq!(show("lss ~/foto", None)[0], "cmd:lss");
    // Vyznamove (sluzba): neznamy prikaz, znamy program, promenna.
    let cmds = ["tool".to_string()];
    let vars = ["x".to_string()];
    let ctx = ctx_with(&vars, &cmds);
    assert_eq!(show("lss ~/foto", Some(&ctx))[0], "unknown:lss");
    assert_eq!(show("tool -v", Some(&ctx))[0], "cmd:tool");
    assert_eq!(show("x", Some(&ctx))[0], "var:x");
}

#[test]
fn notation_c_calls_methods_and_numbers() {
    let ctx = ctx_with(&[], &[]);
    let v = show("ls(\"/\", recursive: true).where(it => it.size > 1.5KB).count()", Some(&ctx));
    assert!(v.contains(&"cmd:ls".to_string()), "{v:?}");
    assert!(v.contains(&"cmd:where".to_string()), "{v:?}");
    assert!(v.contains(&"cmd:count".to_string()), "{v:?}");
    assert!(v.contains(&"str:\"/\"".to_string()), "{v:?}");
    assert!(v.contains(&"kw:true".to_string()), "{v:?}");
    assert!(v.contains(&"var:it".to_string()), "{v:?}");
    assert!(v.contains(&"field:size".to_string()), "{v:?}");
    assert!(v.contains(&"num:1.5KB".to_string()), "{v:?}");
    assert!(v.contains(&"op:=>".to_string()), "{v:?}");
    assert_eq!(show("nezname(1)", Some(&ctx))[0], "unknown:nezname");
}

#[test]
fn redirect_and_word_interpolation() {
    let vars = ["x".to_string()];
    let ctx = ctx_with(&vars, &[]);
    let v = show("echo hi > out-{x}.txt && kitty out-{x}.txt", Some(&ctx));
    assert_eq!(v[0], "cmd:echo");
    assert!(v.contains(&"op:>".to_string()));
    assert!(v.contains(&"path:out-".to_string()), "{v:?}");
    assert!(v.contains(&"interp:{".to_string()));
    assert!(v.contains(&"op:&&".to_string()));
    assert!(v.iter().any(|s| s == "unknown:kitty"), "kitty neni prikaz: {v:?}");
}

#[test]
fn parse_error_is_marked_but_not_unfinished_input() {
    let ctx = ctx_with(&[], &[]);
    // Nedopsany radek (otevreny retezec, roura na konci): zadna chyba.
    assert!(highlight("echo \"ahoj", Some(&ctx)).error.is_none());
    assert!(highlight("ls |", Some(&ctx)).error.is_none());
    // Opravdova chyba uprostred: oznacena.
    let h = highlight("ls ) | count", Some(&ctx));
    let e = h.error.expect("chyba parseru");
    assert_eq!(&"ls ) | count"[e.start..e.end], ")");
}

#[test]
fn spans_are_sorted_and_never_overlap() {
    for src in [
        "ls -r | where size > 5MB",
        "let r = try { open a.txt } else e => e.code",
        "echo \"a {1 + 2} b {\"x\"}\" | save ~/x-{1}.txt",
        "bg { sleep 2s; echo hotovo > /tmp/b.txt }",
        "while true { let x = 1 }",
    ] {
        let h = highlight(src, None);
        for w in h.spans.windows(2) {
            assert!(w[0].end <= w[1].start, "{src}: {:?}", w);
        }
        for s in &h.spans {
            assert!(src.is_char_boundary(s.start) && src.is_char_boundary(s.end));
            assert_ne!(s.class, Class::Plain);
        }
    }
}

#[test]
fn locals_keys_and_assignment() {
    let v = show("let r = try rm stary.txt\nif r.failed { echo \"{r.error.code}\" }", None);
    assert_eq!(&v[..4], ["kw:let", "var:r", "op:=", "kw:try"]);
    assert!(v.contains(&"cmd:rm".to_string()), "{v:?}");
    assert!(v.contains(&"var:r".to_string()) && v.contains(&"field:failed".to_string()), "{v:?}");
    assert!(v.contains(&"field:code".to_string()), "promenna i ve vlozenem vyrazu: {v:?}");
    let v = show("let velke = ls ~/Dok -r", None);
    assert_eq!(v[3], "cmd:ls", "za prirazenim je prikaz: {v:?}");
    let v = show("ls | where f => f.size > 1MB", None);
    assert!(v.contains(&"var:f".to_string()) && v.contains(&"field:size".to_string()), "{v:?}");
    let v = show("needs {\n    read: [\"~/a\"]\n}", None);
    assert!(v.contains(&"field:read".to_string()), "klic manifestu: {v:?}");
    let v = show("try { rm x } else e => log e.message", None);
    assert!(v.contains(&"var:e".to_string()) && v.contains(&"cmd:log".to_string()), "{v:?}");
    let v = show("let limit = 1MB\nls().where(it => it.size > limit)", None);
    assert!(v.contains(&"var:limit".to_string()) && !v.iter().any(|x| x.starts_with("path")), "{v:?}");
    let v = show("needs {\n    net: none\n}", None);
    assert!(!v.contains(&"cmd:none".to_string()), "{v:?}");
    let v = show("fn f(x) {\n    return ls -r\n}", None);
    assert!(v.contains(&"cmd:ls".to_string()), "za return je prikaz: {v:?}");
}

#[test]
fn class_numbers_roundtrip() {
    for c in Class::ALL {
        assert_eq!(Class::from_u8(*c as u8), Some(*c));
    }
}
