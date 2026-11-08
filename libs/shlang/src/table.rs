//! Prevod hodnot na text pro cloveka (`to text`, textovy terminal).
//!
//! Seznam zaznamu je tabulka se zarovnanymi sloupci, zaznam je seznam
//! `klic: hodnota`, ostatni hodnoty jejich textovy tvar.

use alloc::string::String;
use alloc::vec::Vec;

use crate::value::Value;

/// Text bunky tabulky.
pub fn cell_text(v: &Value) -> String {
    cell(v)
}

fn cell(v: &Value) -> String {
    match v {
        Value::Str(s) => s.replace('\n', " "),
        Value::Nothing => String::new(),
        other => other.to_text(),
    }
}

fn width(s: &str) -> usize {
    s.chars().count()
}

/// Je hodnota sloupec cisla (zarovnat doprava)?
pub fn is_numeric(v: &Value) -> bool {
    numeric(v)
}

fn numeric(v: &Value) -> bool {
    matches!(v, Value::Int(_) | Value::Float(_) | Value::Size(_) | Value::Duration(_))
}

/// Sloupce tabulky: sjednoceni klicu ve poradi vyskytu.
pub fn columns(rows: &[Value]) -> Vec<String> {
    let mut cols: Vec<String> = Vec::new();
    for r in rows {
        if let Value::Record(f) = r {
            for (k, _) in f {
                if !cols.contains(k) {
                    cols.push(k.clone());
                }
            }
        }
    }
    cols
}

pub fn render(v: &Value) -> String {
    match v {
        Value::List(rows) if !rows.is_empty() && rows.iter().all(|r| matches!(r, Value::Record(_))) => table(rows),
        Value::List(items) => {
            let mut out = String::new();
            for x in items {
                out.push_str(&cell(x));
                out.push('\n');
            }
            out
        }
        Value::Record(fields) => {
            let w = fields.iter().map(|(k, _)| width(k)).max().unwrap_or(0);
            let mut out = String::new();
            for (k, x) in fields {
                out.push_str(k);
                for _ in width(k)..w {
                    out.push(' ');
                }
                out.push_str(": ");
                out.push_str(&cell(x));
                out.push('\n');
            }
            out
        }
        Value::Nothing => String::new(),
        // Samostatny text i s konci radku (bunka tabulky je jednoradkova).
        Value::Str(s) => {
            let mut out = s.clone();
            if !out.ends_with('\n') {
                out.push('\n');
            }
            out
        }
        other => {
            let mut s = cell(other);
            s.push('\n');
            s
        }
    }
}

fn table(rows: &[Value]) -> String {
    let cols = columns(rows);
    let cells: Vec<Vec<(String, bool)>> = rows
        .iter()
        .map(|r| {
            cols.iter()
                .map(|c| match r.field(c) {
                    Some(v) => (cell(&v), numeric(&v)),
                    None => (String::new(), false),
                })
                .collect()
        })
        .collect();
    let mut w: Vec<usize> = cols.iter().map(|c| width(c)).collect();
    for row in &cells {
        for (i, (s, _)) in row.iter().enumerate() {
            w[i] = w[i].max(width(s));
        }
    }
    let mut out = String::new();
    let line = |out: &mut String, items: &[(String, bool)]| {
        for (i, (s, right)) in items.iter().enumerate() {
            if i > 0 {
                out.push_str("  ");
            }
            let pad = w[i] - width(s);
            if *right {
                for _ in 0..pad {
                    out.push(' ');
                }
                out.push_str(s);
            } else {
                out.push_str(s);
                if i + 1 < items.len() {
                    for _ in 0..pad {
                        out.push(' ');
                    }
                }
            }
        }
        out.push('\n');
    };
    let header: Vec<(String, bool)> = cols.iter().map(|c| (c.clone(), false)).collect();
    line(&mut out, &header);
    let sep: Vec<(String, bool)> = w.iter().map(|n| ("-".repeat(*n), false)).collect();
    line(&mut out, &sep);
    for row in &cells {
        line(&mut out, row);
    }
    out
}
