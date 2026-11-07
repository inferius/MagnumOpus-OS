//! Testy cteni a zapisu TOML (`cargo test -p conf`).

extern crate std;

use alloc::string::String;
use alloc::vec;

use crate::{Table, Value, parse, parse_size};

#[test]
fn basic_document() {
    let t = parse(
        r#"
# komentar
title = "Firewall" # za hodnotou
count = 1_000
neg = -42
hex = 0xFF
on = true
off = false
list = ["a", 'b\c', "x\"y\u00e1"]
multi = [
  1,   # prvni
  2,
]
"quoted.key" = 'doslovne'
a.b.c = 5

[app]
id = "org.test.app"

[app.inner]
x = 1

[[rule]]
port = 443
[rule.extra]
note = "prvni"

[[rule]]
port = 80
[rule.extra]
note = "druhy"
"#,
    )
    .unwrap();
    assert_eq!(t.get_str("title"), Some("Firewall"));
    assert_eq!(t.get_int("count"), Some(1000));
    assert_eq!(t.get_int("neg"), Some(-42));
    assert_eq!(t.get_int("hex"), Some(255));
    assert_eq!(t.get_bool("on"), Some(true));
    assert_eq!(t.get_bool("off"), Some(false));
    assert_eq!(t.get_strs("list").unwrap(), vec!["a", "b\\c", "x\"y\u{e1}"]);
    assert_eq!(t.get("multi").unwrap().as_array().unwrap().len(), 2);
    assert_eq!(t.get_str("quoted.key"), None, "tecky v ceste se deli");
    assert_eq!(t.key("quoted.key").and_then(Value::as_str), Some("doslovne"));
    assert_eq!(t.get_int("a.b.c"), Some(5));
    assert_eq!(t.get_str("app.id"), Some("org.test.app"));
    assert_eq!(t.get_int("app.inner.x"), Some(1));
    let rules = t.tables("rule");
    assert_eq!(rules.len(), 2);
    assert_eq!(rules[0].get_int("port"), Some(443));
    assert_eq!(rules[1].get_str("extra.note"), Some("druhy"));
}

#[test]
fn inline_tables_and_nested_arrays() {
    let t = parse("x = { a = 1, b.c = \"d\" }\ny = [[1, 2], [3]]\nz = {}\n").unwrap();
    assert_eq!(t.get_int("x.a"), Some(1));
    assert_eq!(t.get_str("x.b.c"), Some("d"));
    assert_eq!(t.get("y").unwrap().as_array().unwrap()[1].as_array().unwrap().len(), 1);
    assert_eq!(t.get_table("z"), Some(&Table::new()));
}

#[test]
fn errors_have_lines() {
    let cases = [
        ("a = ", 1),
        ("a = 1\nb = \"neukonceny\n", 2),
        ("a = 1\na = 2\n", 2),
        ("[t]\n[t]\n", 2),
        ("a = 1 b\n", 1),
        ("a = 1.5\n", 1),
        ("[t\n", 1),
        ("a = \"\\q\"\n", 1),
        ("a = [1 2]\n", 1),
        ("x = 1\n[x]\n", 2),
    ];
    for (text, line) in cases {
        let e = parse(text).unwrap_err();
        assert_eq!(e.line, line, "{text:?}: {e}");
    }
    // Hluboke vnoreni neshodi zasobnik.
    let deep = String::from("a = ") + &"[".repeat(1000) + &"]".repeat(1000);
    assert!(parse(&deep).is_err());
}

#[test]
fn write_roundtrip() {
    let src = r#"
name = "x \"y\" \\ z"
n = 7
flags = [true, false]
inline = [{ a = 1 }, { b = "c" }]

[profile.home]
inbound = "listeners"

[profile.public]
inbound = "deny"

[[apps]]
id = "org.a"
[[apps.allow]]
target = "*.example.com"
ports = "443"

[[apps]]
id = "org.b"
"#;
    let t = parse(src).unwrap();
    let text = t.to_toml();
    let again = parse(&text).unwrap_or_else(|e| panic!("{e}\n{text}"));
    assert_eq!(again, t, "{text}");
}

#[test]
fn edit_api() {
    let mut t = Table::new();
    t.table_mut("net").set("profile", Value::Str(String::from("home")));
    t.array_mut("rule").push(Value::Table(Table::new()));
    t.set("n", Value::Int(1));
    t.set("n", Value::Int(2));
    assert_eq!(t.get_int("n"), Some(2));
    assert_eq!(t.get_str("net.profile"), Some("home"));
    assert_eq!(t.remove("n"), Some(Value::Int(2)));
    assert_eq!(parse(&t.to_toml()).unwrap(), t);
}

#[test]
fn sizes() {
    assert_eq!(parse_size(&Value::Int(4096)), Some(4096));
    assert_eq!(parse_size(&Value::Str(String::from("64K"))), Some(64 << 10));
    assert_eq!(parse_size(&Value::Str(String::from("16M"))), Some(16 << 20));
    assert_eq!(parse_size(&Value::Str(String::from("2G"))), Some(2 << 30));
    assert_eq!(parse_size(&Value::Str(String::from("x"))), None);
    assert_eq!(parse_size(&Value::Int(-1)), None);
}
