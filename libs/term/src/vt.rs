//! Adapter na text s escape sekvencemi VT100/ANSI (seriova linka,
//! zachranna konzole, prevzate programy).
//!
//! Nativni protokol nese typovane hodnoty; tady se prevadi na text az na
//! konci: tabulky zarovnane (`shlang::table`), s motivem barevne podle
//! typu ([`crate::render`]), chyby a poznamky barvou motivu, odkazy jako
//! OSC 8.

use alloc::string::String;

use crate::proto::{Item, ServerMsg};
use crate::render::{self, RESET, sgr};
use crate::theme::{Rgb, Style, Theme};

fn paint(out: &mut String, theme: Option<&Theme>, style: impl Fn(&Theme) -> Style, text: &str) {
    match theme {
        Some(t) => {
            out.push_str(&sgr(style(t)));
            out.push_str(text);
            out.push_str(RESET);
        }
        None => out.push_str(text),
    }
}

fn fg(c: impl Fn(&Theme) -> Rgb) -> impl Fn(&Theme) -> Style {
    move |t| Style::fg(c(t))
}

/// Text polozky bloku. `theme = None` = bez barev (log, test).
pub fn item(it: &Item, theme: Option<&Theme>) -> String {
    item_at(it, theme, None)
}

/// Jako [`item`]; `now` (ns od 1970) obarvi stari casu.
pub fn item_at(it: &Item, theme: Option<&Theme>, now: Option<i64>) -> String {
    let mut out = String::new();
    match it {
        Item::Value(v) => match theme {
            Some(t) => out.push_str(&render::value(v, t, now)),
            None => out.push_str(&shlang::table::render(v)),
        },
        Item::Note(n) => {
            paint(&mut out, theme, fg(|t| t.ui.muted), n);
            out.push('\n');
        }
        Item::Warning(w) => {
            paint(&mut out, theme, fg(|t| t.ui.warn), &alloc::format!("varovani: {w}"));
            out.push('\n');
        }
        Item::Error { report, .. } => {
            paint(&mut out, theme, fg(|t| t.ui.err), report);
            out.push('\n');
        }
        Item::Diff(v) => {
            paint(&mut out, theme, fg(|t| t.ui.accent), "zmeny souboru (vratit: undo):");
            out.push('\n');
            match theme {
                Some(t) => out.push_str(&render::value(v, t, now)),
                None => out.push_str(&shlang::table::render(v)),
            }
        }
        Item::Progress { done, total, label } => {
            let pct = if *total == 0 { 100 } else { (done * 100 / total).min(100) };
            let full = (pct / 5) as usize;
            let bar: String = (0..20).map(|i| if i < full { '#' } else { '-' }).collect();
            out.push('[');
            paint(&mut out, theme, fg(|t| t.ui.accent), &bar);
            out.push_str(&alloc::format!("] {pct}% {label}\r"));
        }
        Item::Link { text, target } => {
            if theme.is_some() {
                out.push_str(&alloc::format!("\x1b]8;;{target}\x1b\\{text}\x1b]8;;\x1b\\\n"));
            } else {
                out.push_str(&alloc::format!("{text} <{target}>\n"));
            }
        }
    }
    out
}

/// Text zpravy sluzby (pro terminal bez nativniho protokolu).
pub fn message(m: &ServerMsg, theme: Option<&Theme>) -> String {
    let mut out = String::new();
    match m {
        ServerMsg::Welcome { version } => out.push_str(&alloc::format!("terminal: protokol v{version}\n")),
        ServerMsg::Attached { session, cwd, .. } => {
            paint(&mut out, theme, fg(|t| t.ui.muted), &alloc::format!("relace {session} ({cwd})"));
            out.push('\n');
        }
        ServerMsg::BlockStart { line, .. } => {
            paint(&mut out, theme, fg(|t| t.ui.muted), "> ");
            match theme {
                // Radek bloku: lexikalni zvyrazneni (bez znalosti relace).
                Some(t) => out.push_str(&render::line(line, &shlang::highlight::highlight(line, None), t)),
                None => out.push_str(line),
            }
            out.push('\n');
        }
        ServerMsg::Item { item: it, .. } => out.push_str(&item(it, theme)),
        ServerMsg::BlockEnd { ok, duration, .. } => {
            if !ok {
                paint(&mut out, theme, fg(|t| t.ui.err), "(neuspech)");
                out.push(' ');
            }
            if *duration > 0 {
                paint(&mut out, theme, fg(|t| t.ui.muted), &shlang::value::human_duration(*duration));
            }
            if !ok || *duration > 0 {
                out.push('\n');
            }
        }
        ServerMsg::Prompt { question, detail, choices, .. } => {
            out.push_str(&item(detail, theme));
            let opts = if choices.is_empty() {
                String::from("[a/n]")
            } else {
                choices.iter().enumerate().map(|(i, c)| alloc::format!("{}) {c}", i + 1)).collect::<alloc::vec::Vec<_>>().join("  ")
            };
            paint(&mut out, theme, |t| Style { bold: true, ..Style::fg(t.ui.fg) }, &alloc::format!("{question} {opts} "));
        }
        ServerMsg::Completions { items, .. } => {
            for (t, _, h) in items {
                out.push_str(t);
                if !h.is_empty() {
                    out.push_str("  ");
                    paint(&mut out, theme, fg(|t| t.ui.muted), h);
                }
                out.push('\n');
            }
        }
        ServerMsg::SessionList { sessions } => {
            for (id, n, cwd) in sessions {
                out.push_str(&alloc::format!("{id}  {n} bloku  {cwd}\n"));
            }
        }
        ServerMsg::Formatted { text, .. } => {
            out.push_str(text);
            out.push('\n');
        }
        ServerMsg::Fail { message } => {
            paint(&mut out, theme, fg(|t| t.ui.err), message);
            out.push('\n');
        }
        // Zvyrazneni je pro editor radku v okne, v textu se neukazuje.
        ServerMsg::Highlighted { .. } => {}
        // Zmena na miste: pruh se prepise (`\r`), jine polozky znovu.
        ServerMsg::ItemUpdate { item: it, .. } => out.push_str(&item(it, theme)),
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
