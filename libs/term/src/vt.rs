//! Adapter na text s escape sekvencemi VT100/ANSI (seriova linka,
//! zachranna konzole, prevzate programy).
//!
//! Nativni protokol nese typovane hodnoty; tady se prevadi na text az na
//! konci: tabulky zarovnane (`shlang::table`), chyby cervene, poznamky
//! tlumene, odkazy jako OSC 8.

use alloc::string::String;

use crate::proto::{Item, ServerMsg};

const RESET: &str = "\x1b[0m";
const BOLD: &str = "\x1b[1m";
const DIM: &str = "\x1b[2m";
const RED: &str = "\x1b[31m";
const YELLOW: &str = "\x1b[33m";
const CYAN: &str = "\x1b[36m";

fn paint(out: &mut String, color: bool, sgr: &str, text: &str) {
    if color {
        out.push_str(sgr);
    }
    out.push_str(text);
    if color {
        out.push_str(RESET);
    }
}

/// Text polozky bloku.
pub fn item(it: &Item, color: bool) -> String {
    let mut out = String::new();
    match it {
        Item::Value(v) => out.push_str(&shlang::table::render(v)),
        Item::Note(n) => {
            paint(&mut out, color, DIM, n);
            out.push('\n');
        }
        Item::Warning(w) => {
            paint(&mut out, color, YELLOW, &alloc::format!("varovani: {w}"));
            out.push('\n');
        }
        Item::Error { report, .. } => {
            paint(&mut out, color, RED, report);
            out.push('\n');
        }
        Item::Diff(v) => {
            paint(&mut out, color, CYAN, "zmeny souboru (vratit: undo):");
            out.push('\n');
            out.push_str(&shlang::table::render(v));
        }
        Item::Progress { done, total, label } => {
            let pct = if *total == 0 { 100 } else { (done * 100 / total).min(100) };
            let full = (pct / 5) as usize;
            out.push('[');
            for i in 0..20 {
                out.push(if i < full { '#' } else { '-' });
            }
            out.push_str(&alloc::format!("] {pct}% {label}\r"));
        }
        Item::Link { text, target } => {
            if color {
                out.push_str(&alloc::format!("\x1b]8;;{target}\x1b\\{text}\x1b]8;;\x1b\\\n"));
            } else {
                out.push_str(&alloc::format!("{text} <{target}>\n"));
            }
        }
    }
    out
}

/// Text zpravy sluzby (pro terminal bez nativniho protokolu).
pub fn message(m: &ServerMsg, color: bool) -> String {
    let mut out = String::new();
    match m {
        ServerMsg::Welcome { version } => out.push_str(&alloc::format!("terminal: protokol v{version}\n")),
        ServerMsg::Attached { session, cwd, .. } => {
            paint(&mut out, color, DIM, &alloc::format!("relace {session} ({cwd})"));
            out.push('\n');
        }
        ServerMsg::BlockStart { line, .. } => {
            paint(&mut out, color, BOLD, &alloc::format!("> {line}"));
            out.push('\n');
        }
        ServerMsg::Item { item: it, .. } => out.push_str(&item(it, color)),
        ServerMsg::BlockEnd { ok, duration, .. } => {
            if !ok {
                paint(&mut out, color, RED, "(neuspech)");
                out.push(' ');
            }
            if *duration > 0 {
                paint(&mut out, color, DIM, &shlang::value::human_duration(*duration));
            }
            if !ok || *duration > 0 {
                out.push('\n');
            }
        }
        ServerMsg::Prompt { question, detail, .. } => {
            out.push_str(&item(detail, color));
            paint(&mut out, color, BOLD, &alloc::format!("{question} [a/n] "));
        }
        ServerMsg::Completions { items, .. } => {
            for (t, _, h) in items {
                out.push_str(t);
                if !h.is_empty() {
                    out.push_str("  ");
                    paint(&mut out, color, DIM, h);
                }
                out.push('\n');
            }
        }
        ServerMsg::SessionList { sessions } => {
            for (id, n, cwd) in sessions {
                out.push_str(&alloc::format!("{id}  {n} bloku  {cwd}\n"));
            }
        }
        ServerMsg::Formatted { text } => {
            out.push_str(text);
            out.push('\n');
        }
        ServerMsg::Fail { message } => {
            paint(&mut out, color, RED, message);
            out.push('\n');
        }
    }
    out
}

/// Odstrani escape sekvence (CSI a OSC).
pub fn strip(s: &str) -> String {
    let mut out = String::new();
    let mut it = s.chars().peekable();
    while let Some(c) = it.next() {
        if c != '\x1b' {
            out.push(c);
            continue;
        }
        match it.next() {
            Some('[') => {
                for d in it.by_ref() {
                    if ('@'..='~').contains(&d) {
                        break;
                    }
                }
            }
            Some(']') => {
                while let Some(d) = it.next() {
                    if d == '\x07' {
                        break;
                    }
                    if d == '\x1b' && it.peek() == Some(&'\\') {
                        it.next();
                        break;
                    }
                }
            }
            _ => {}
        }
    }
    out
}
