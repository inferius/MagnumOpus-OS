//! Vykresleni s barvami motivu do textu s ANSI (24bitove barvy).
//!
//! Konec cesty: protokol nese typovane hodnoty a tridy, sem az prijde
//! motiv. Pouziva ho adapter VT100 ([`crate::vt`]) a tim i obrazovka
//! ([`crate::screen`]) a seriova linka.
//!
//! - [`line`]: radek prikazu podle zvyrazneni ([`shlang::highlight`]),
//! - [`value`]: tabulky a zaznamy - jmena souboru podle druhu, velikost
//!   a stari v barevne skale, `true`/`false`, `null`, chyby.

use alloc::string::String;
use alloc::vec::Vec;

use shlang::highlight::{Class, Highlight};
use shlang::table::{cell_text, columns, is_numeric};
use shlang::value::Value;

use crate::theme::{FileClass, Rgb, Style, Theme};

pub const RESET: &str = "\x1b[0m";

/// Escape sekvence stylu.
pub fn sgr(s: Style) -> String {
    let mut out = alloc::format!("\x1b[0;38;2;{};{};{}", s.fg.0, s.fg.1, s.fg.2);
    if s.bold {
        out.push_str(";1");
    }
    if s.italic {
        out.push_str(";3");
    }
    if s.underline {
        out.push_str(";4");
    }
    out.push('m');
    out
}

fn paint(out: &mut String, s: Style, text: &str) {
    if text.is_empty() {
        return;
    }
    out.push_str(&sgr(s));
    out.push_str(text);
    out.push_str(RESET);
}

fn color(c: Rgb) -> Style {
    Style::fg(c)
}

/// Radek prikazu s barvami; misto chyby parseru podtrzene barvou chyby.
pub fn line(src: &str, h: &Highlight, t: &Theme) -> String {
    let mut out = String::new();
    let mut at = 0;
    for s in &h.spans {
        if s.start < at || s.end > src.len() {
            continue;
        }
        out.push_str(&src[at..s.start]);
        let mut style = t.syntax(s.class);
        if let Some(e) = &h.error
            && s.start < e.end
            && e.start < s.end
        {
            style.underline = true;
            style.fg = t.ui.err;
        }
        paint(&mut out, style, &src[s.start..s.end]);
        at = s.end;
    }
    out.push_str(&src[at..]);
    out
}

/// Velikost -> stupen skaly (0 < 1 MB, 1 < 100 MB, 2 < 1 GB, 3 vic).
pub fn size_level(bytes: i64) -> usize {
    match bytes {
        b if b < 1 << 20 => 0,
        b if b < 100 << 20 => 1,
        b if b < 1 << 30 => 2,
        _ => 3,
    }
}

/// Stari -> stupen skaly (0 < den, 1 < 30 dni, 2 < rok, 3 starsi).
pub fn age_level(mtime: i64, now: i64) -> usize {
    const DAY: i64 = 86_400 * 1_000_000_000;
    match now.saturating_sub(mtime) {
        d if d < DAY => 0,
        d if d < 30 * DAY => 1,
        d if d < 365 * DAY => 2,
        _ => 3,
    }
}

fn str_field(rec: &Value, name: &str) -> Option<String> {
    match rec.field(name) {
        Some(Value::Str(s)) => Some(s),
        _ => None,
    }
}

/// Druh zaznamu souboru (z `ls`, `find`, `glob`); `None` = neni soubor.
pub fn file_class(rec: &Value, t: &Theme) -> Option<FileClass> {
    let kind = str_field(rec, "kind")?;
    let name = str_field(rec, "name")?;
    let access = str_field(rec, "access").unwrap_or_default();
    if !access.is_empty() && !access.contains('r') {
        return Some(FileClass::NoAccess);
    }
    Some(match kind.as_str() {
        "dir" => FileClass::Dir,
        "dataset" => FileClass::Dataset,
        "symlink" => FileClass::Symlink,
        "file" => {
            if name.starts_with('.') {
                FileClass::Hidden
            } else if access.contains('x') {
                FileClass::Exec
            } else {
                let ext = str_field(rec, "ext").unwrap_or_default();
                t.ext_class(&ext).unwrap_or(FileClass::Plain)
            }
        }
        _ => return None,
    })
}

/// Styl bunky tabulky.
fn cell_style(col: &str, v: &Value, rec: &Value, t: &Theme, now: Option<i64>) -> Option<Style> {
    match v {
        Value::Size(b) => return Some(color(t.scale[size_level(*b)])),
        Value::Time(m) => return now.map(|n| color(t.scale[age_level(*m, n)])),
        Value::Bool(true) => return Some(color(t.ui.ok)),
        Value::Bool(false) => return Some(color(t.ui.err)),
        Value::Null | Value::Nothing => return Some(color(t.ui.muted)),
        Value::Error(_) => return Some(color(t.ui.err)),
        _ => {}
    }
    match col {
        "name" => file_class(rec, t).map(|c| t.file(c)),
        "kind" | "ext" | "access" if file_class(rec, t).is_some() => Some(color(t.ui.muted)),
        "change" => Some(color(match v {
            Value::Str(s) if s.contains("removed") => t.ui.err,
            Value::Str(s) if s.contains("added") => t.ui.ok,
            _ => t.ui.warn,
        })),
        _ => None,
    }
}

fn width(s: &str) -> usize {
    s.chars().count()
}

/// Hodnota s barvami (tabulka, zaznam, seznam, skalar).
pub fn value(v: &Value, t: &Theme, now: Option<i64>) -> String {
    match v {
        Value::List(rows) if !rows.is_empty() && rows.iter().all(|r| matches!(r, Value::Record(_))) => {
            table(rows, t, now)
        }
        Value::List(items) => {
            let mut out = String::new();
            for x in items {
                styled(&mut out, cell_style("", x, x, t, now), &cell_text(x));
                out.push('\n');
            }
            out
        }
        Value::Record(fields) => {
            let w = fields.iter().map(|(k, _)| width(k)).max().unwrap_or(0);
            let mut out = String::new();
            for (k, x) in fields {
                paint(&mut out, color(t.ui.muted), k);
                out.push_str(&" ".repeat(w - width(k)));
                out.push_str(": ");
                styled(&mut out, cell_style(k, x, v, t, now), &cell_text(x));
                out.push('\n');
            }
            out
        }
        Value::Nothing => String::new(),
        other => {
            let mut out = String::new();
            styled(&mut out, cell_style("", other, other, t, now), &cell_text(other));
            out.push('\n');
            out
        }
    }
}

fn styled(out: &mut String, s: Option<Style>, text: &str) {
    match s {
        Some(s) => paint(out, s, text),
        None => out.push_str(text),
    }
}

fn table(rows: &[Value], t: &Theme, now: Option<i64>) -> String {
    let cols = columns(rows);
    let cells: Vec<Vec<(String, bool, Option<Style>)>> = rows
        .iter()
        .map(|r| {
            cols.iter()
                .map(|c| match r.field(c) {
                    Some(v) => (cell_text(&v), is_numeric(&v), cell_style(c, &v, r, t, now)),
                    None => (String::new(), false, None),
                })
                .collect()
        })
        .collect();
    let mut w: Vec<usize> = cols.iter().map(|c| width(c)).collect();
    for row in &cells {
        for (i, (s, _, _)) in row.iter().enumerate() {
            w[i] = w[i].max(width(s));
        }
    }
    let mut out = String::new();
    let line = |out: &mut String, items: &[(String, bool, Option<Style>)]| {
        for (i, (s, right, st)) in items.iter().enumerate() {
            if i > 0 {
                out.push_str("  ");
            }
            let pad = " ".repeat(w[i] - width(s));
            if *right {
                out.push_str(&pad);
            }
            styled(out, *st, s);
            if !*right && i + 1 < items.len() {
                out.push_str(&pad);
            }
        }
        out.push('\n');
    };
    let head = Style { bold: true, ..color(t.ui.muted) };
    let header: Vec<(String, bool, Option<Style>)> = cols.iter().map(|c| (c.clone(), false, Some(head))).collect();
    line(&mut out, &header);
    for row in &cells {
        line(&mut out, row);
    }
    out
}

/// Tridy zvyrazneni, ktere radek pouziva (testy shody s dokumentaci).
pub fn classes(h: &Highlight) -> Vec<Class> {
    h.spans.iter().map(|s| s.class).collect()
}
