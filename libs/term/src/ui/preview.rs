//! Vyzvy v okne terminalu: otazka ano/ne, vyber z voleb, dotaz na text
//! a nahled zmen (zamek nebezpecnych operaci, `preview`): seznam zmen
//! z detailu vyzvy a tlacitka "Prevzit N zmen" / "Zahodit"
//! (`design/terminal.html`, `.diffbox`). Text radku je uplny (konzole);
//! okno GUI podle role [`Role::Prompt`] dokresli ramecek a tlacitka
//! a klik na tlacitko posle [`App::prompt_pick`].

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use shlang::value::Value;

use super::draw::{Line, PromptRow, Role};
use super::{App, Key, short_path};
use crate::proto::{ClientMsg, Item};
use crate::theme::Style;

/// Kolik zmen nahled ukaze (zbytek jako "... a N dalsich").
pub const PREVIEW_ROWS: usize = 6;

/// Zmeny z detailu vyzvy: seznam zaznamu se sloupci `change` a `path`
/// (`shlang::vfs::diff::diff_value`). Jiny detail = `None`.
pub fn changes(detail: &Item) -> Option<Vec<(String, String)>> {
    let (Item::Value(Value::List(rows)) | Item::Diff(Value::List(rows))) = detail else { return None };
    if rows.is_empty() {
        return None;
    }
    rows.iter()
        .map(|r| match (r.field("change"), r.field("path")) {
            (Some(Value::Str(c)), Some(Value::Str(p))) => Some((c, p)),
            _ => None,
        })
        .collect()
}

impl App {
    /// Popisky tlacitek vyzvy (stejne poradi jako odpovedi: ano/ne,
    /// volby).
    pub fn prompt_labels(&self) -> Vec<String> {
        let Some(p) = &self.prompt else { return Vec::new() };
        if !p.choices.is_empty() {
            return p.choices.iter().enumerate().map(|(i, c)| format!("{} {c}", i + 1)).collect();
        }
        match changes(&p.detail) {
            Some(ch) => alloc::vec![format!("Převzít {} změn (a)", ch.len()), "Zahodit (n)".into()],
            None => alloc::vec!["Ano (a)".into(), "Ne (n)".into()],
        }
    }

    /// Odpoved tlacitkem `i` (klik v okne GUI): jako klavesa volby.
    pub fn prompt_pick(&mut self, i: usize) {
        let Some(p) = &self.prompt else { return };
        if p.input.is_some() {
            return;
        }
        let n = if p.choices.is_empty() { 2 } else { p.choices.len() };
        if i < n {
            self.answer_prompt(i);
        }
    }

    fn answer_prompt(&mut self, a: usize) {
        let Some(p) = self.prompt.take() else { return };
        if p.choices.is_empty() {
            self.out.push(ClientMsg::Answer { prompt: p.id, yes: a == 0 });
        } else if a == usize::MAX {
            self.out.push(ClientMsg::Interrupt { session: self.session });
        } else {
            self.out.push(ClientMsg::Choose { prompt: p.id, index: a as u32 });
        }
    }

    pub(super) fn key_prompt(&mut self, k: Key) {
        if self.prompt.as_ref().is_some_and(|p| p.input.is_some()) {
            return self.key_ask(k);
        }
        let Some(p) = self.prompt.as_mut() else { return };
        let n = if p.choices.is_empty() { 2 } else { p.choices.len() };
        let answer = match k {
            Key::Left | Key::Up | Key::BackTab => {
                p.sel = (p.sel + n - 1) % n;
                None
            }
            Key::Right | Key::Down | Key::Tab => {
                p.sel = (p.sel + 1) % n;
                None
            }
            Key::Char('a' | 'y' | 'A' | 'Y') if p.choices.is_empty() => Some(0),
            Key::Char('n' | 'N') if p.choices.is_empty() => Some(1),
            Key::Char(c) if c.is_ascii_digit() && !p.choices.is_empty() => {
                let i = c as usize - '1' as usize;
                (i < n).then_some(i)
            }
            Key::Esc | Key::Ctrl('c') => Some(if p.choices.is_empty() { 1 } else { usize::MAX }),
            Key::Enter => Some(p.sel),
            _ => None,
        };
        if let Some(a) = answer {
            self.answer_prompt(a);
        }
    }

    /// Psani odpovedi na dotaz na text.
    fn key_ask(&mut self, k: Key) {
        let Some(p) = self.prompt.as_mut() else { return };
        let Some((_, text)) = p.input.as_mut() else { return };
        match k {
            Key::Char(c) => text.push(c),
            Key::Backspace => {
                text.pop();
            }
            Key::Enter => {
                let p = self.prompt.take().unwrap();
                let text = p.input.map(|(_, t)| t).unwrap_or_default();
                self.out.push(ClientMsg::Text { prompt: p.id, text });
            }
            Key::Esc | Key::Ctrl('c') => {
                self.prompt = None;
                self.out.push(ClientMsg::Interrupt { session: self.session });
            }
            _ => {}
        }
    }

    /// Radky vyzvy (dole nad radkem vstupu).
    pub(super) fn draw_prompt(&self, bottom: &mut Vec<Line>) {
        let t = &self.theme;
        let Some(p) = &self.prompt else { return };
        let bold = Style { bold: true, ..Style::fg(t.ui.fg) };
        // Systemova vyzva: odznak zdroje (program ho svym vystupem
        // nenakresli, vyzvu posila jen sluzba relaci).
        let question = |q: &str| {
            let mut l = Line::new();
            if !p.label.is_empty() {
                l.push_bg(Some(Style { bold: true, ..Style::fg(t.ui.bg) }), t.ui.warn, &format!(" {} ", p.label));
                l.push(None, " ");
            }
            l.text(Some(bold), q)
        };
        if let Some((secret, text)) = &p.input {
            bottom.push(question(&p.question));
            let shown: String = if *secret { text.chars().map(|_| '*').collect() } else { text.clone() };
            bottom.push(Line::new().text(Some(Style::fg(t.ui.fg)), &format!("> {shown}_")));
            return;
        }
        let ch = if p.choices.is_empty() { changes(&p.detail) } else { None };
        let preview = ch.is_some();
        bottom.push(question(&p.question).role(Role::Prompt(PromptRow::Question { preview })));
        if let Some(ch) = &ch {
            // Druh zmeny barvou (smazano cervene, nove zelene, jinak zlute),
            // cesta s domovem jako `~`.
            let w = ch.iter().take(PREVIEW_ROWS).map(|(c, _)| c.chars().count()).max().unwrap_or(0);
            let room = self.cols.saturating_sub(w + 5).max(12);
            for (c, path) in ch.iter().take(PREVIEW_ROWS) {
                let col = match c.as_str() {
                    "removed" | "dir-removed" => t.ui.err,
                    "added" | "dir-added" => t.ui.ok,
                    _ => t.ui.warn,
                };
                let mut l = Line::new();
                l.push(None, "  ");
                l.push(Some(Style::fg(col)), &format!("{c:<w$}"));
                l.push(Some(Style::fg(t.ui.fg)), &format!("  {}", short_path(path, room)));
                bottom.push(l.role(Role::Prompt(PromptRow::Change)));
            }
            if ch.len() > PREVIEW_ROWS {
                let more = ch.len() - PREVIEW_ROWS;
                bottom.push(Line::new().text(Some(Style::fg(t.ui.muted)), &format!("  \u{2026} a {more} dalších")).role(Role::Prompt(PromptRow::Change)));
            }
        }
        let labels = self.prompt_labels();
        let mut l = Line::new();
        for (i, o) in labels.iter().enumerate() {
            l.push(None, " ");
            let st = Style::fg(if i == p.sel { t.ui.fg } else { t.ui.muted });
            if i == p.sel {
                l.push_bg(Some(st), t.ui.select, &format!(" {o} "));
            } else {
                l.push(Some(st), &format!(" {o} "));
            }
        }
        bottom.push(l.role(Role::Prompt(PromptRow::Buttons { labels, sel: p.sel, preview })));
        // Mezera pod tlacitky (okno je kresli vyssi nez radek textu).
        bottom.push(Line::new());
    }
}
