//! Zapis TOML.
//!
//! Jednoduche hodnoty tabulky jdou na zacatek (pod jeji hlavicku),
//! vnorene tabulky jako `[cesta]` a pole tabulek jako `[[cesta]]` za ne.
//! Pole jednoduchych hodnot a tabulky uvnitr poli hodnot jdou na jeden
//! radek (`[1, 2]`, `{ a = 1 }`).

use alloc::string::String;
use alloc::vec::Vec;
use core::fmt::Write;

use crate::{Table, Value};

pub fn write(t: &Table) -> String {
    let mut out = String::new();
    write_table(&mut out, t, &mut Vec::new(), false);
    out
}

fn is_table_array(v: &Value) -> bool {
    matches!(v, Value::Array(a) if !a.is_empty() && a.iter().all(|x| matches!(x, Value::Table(_))))
}

fn write_table(out: &mut String, t: &Table, path: &mut Vec<String>, array_item: bool) {
    let simple: Vec<&(String, Value)> = t
        .entries
        .iter()
        .filter(|(_, v)| !matches!(v, Value::Table(_)) && !is_table_array(v))
        .collect();
    let has_nested = simple.len() < t.entries.len();
    if !path.is_empty() && (array_item || !simple.is_empty() || !has_nested) {
        if !out.is_empty() {
            out.push('\n');
        }
        let header = path.iter().map(|k| key(k)).collect::<Vec<_>>().join(".");
        if array_item {
            let _ = writeln!(out, "[[{header}]]");
        } else {
            let _ = writeln!(out, "[{header}]");
        }
    }
    for (k, v) in &simple {
        let _ = writeln!(out, "{} = {}", key(k), inline(v));
    }
    for (k, v) in &t.entries {
        match v {
            Value::Table(sub) => {
                path.push(k.clone());
                write_table(out, sub, path, false);
                path.pop();
            }
            Value::Array(items) if is_table_array(v) => {
                path.push(k.clone());
                for item in items {
                    if let Value::Table(sub) = item {
                        write_table(out, sub, path, true);
                    }
                }
                path.pop();
            }
            _ => {}
        }
    }
}

/// Klic: holy, kdyz to jde, jinak v uvozovkach.
fn key(k: &str) -> String {
    if !k.is_empty()
        && k
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
    {
        String::from(k)
    } else {
        quote(k)
    }
}

fn quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            c if (c as u32) < 0x20 || c == '\u{7f}' => {
                let _ = write!(out, "\\u{:04X}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn inline(v: &Value) -> String {
    match v {
        Value::Str(s) => quote(s),
        Value::Int(n) => alloc::format!("{n}"),
        Value::Float(f) => {
            // Vzdy s teckou nebo exponentem, aby se cislo precetlo zpet
            // jako desetinne.
            let s = alloc::format!("{:?}", f.0);
            if s.contains(['.', 'e', 'E']) { s } else { alloc::format!("{s}.0") }
        }
        Value::Bool(b) => String::from(if *b { "true" } else { "false" }),
        Value::Array(a) => {
            let items: Vec<String> = a.iter().map(inline).collect();
            alloc::format!("[{}]", items.join(", "))
        }
        Value::Table(t) => {
            if t.entries.is_empty() {
                return String::from("{}");
            }
            let items: Vec<String> = t
                .entries
                .iter()
                .map(|(k, v)| alloc::format!("{} = {}", key(k), inline(v)))
                .collect();
            alloc::format!("{{ {} }}", items.join(", "))
        }
    }
}
