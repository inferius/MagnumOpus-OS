//! Editor radku: text, kurzor, historie, napoveda z historie (seda
//! "duchova" dopln), zvyrazneni ve dvou urovnich.
//!
//! Lexikalni zvyrazneni se pocita hned pri kazde klavese
//! (`shlang::highlight` bez kontextu). Vyznamove posle sluzba
//! (`Highlighted` s `seq`); pouzije se jen pro text, ke kteremu patri
//! (starsi odpoved se zahodi), takze pismena na odpoved nikdy necekaji.

use alloc::string::String;
use alloc::vec::Vec;

use shlang::highlight::{Class, ErrorMark, Highlight, Span, highlight};

#[derive(Debug, Default)]
pub struct Editor {
    pub text: String,
    /// Bajtova pozice kurzoru (vzdy na hranici znaku).
    pub cursor: usize,
    history: Vec<String>,
    /// Pozice v historii pri listovani (`None` = rozepsany radek).
    hist_pos: Option<usize>,
    draft: String,
    /// Poradi verze textu (roste s kazdou zmenou).
    pub seq: u32,
    /// Vyznamove zvyrazneni od sluzby a `seq`, ke kteremu patri.
    semantic: Option<(u32, Highlight)>,
}

fn prev_boundary(s: &str, i: usize) -> usize {
    s[..i].char_indices().next_back().map_or(0, |(k, _)| k)
}

fn next_boundary(s: &str, i: usize) -> usize {
    s[i..].chars().next().map_or(i, |c| i + c.len_utf8())
}

impl Editor {
    pub fn new() -> Self {
        Self::default()
    }

    fn changed(&mut self) {
        self.seq = self.seq.wrapping_add(1);
        self.hist_pos = None;
    }

    pub fn set(&mut self, text: &str) {
        self.text = text.into();
        self.cursor = self.text.len();
        self.changed();
    }

    pub fn insert(&mut self, s: &str) {
        self.text.insert_str(self.cursor, s);
        self.cursor += s.len();
        self.changed();
    }

    pub fn backspace(&mut self) {
        if self.cursor > 0 {
            let p = prev_boundary(&self.text, self.cursor);
            self.text.replace_range(p..self.cursor, "");
            self.cursor = p;
            self.changed();
        }
    }

    pub fn delete(&mut self) {
        if self.cursor < self.text.len() {
            let n = next_boundary(&self.text, self.cursor);
            self.text.replace_range(self.cursor..n, "");
            self.changed();
        }
    }

    pub fn left(&mut self) {
        self.cursor = prev_boundary(&self.text, self.cursor);
    }

    pub fn right(&mut self) {
        if self.cursor == self.text.len() {
            self.accept_suggestion();
        } else {
            self.cursor = next_boundary(&self.text, self.cursor);
        }
    }

    pub fn home(&mut self) {
        self.cursor = 0;
    }

    pub fn end(&mut self) {
        if self.cursor == self.text.len() {
            self.accept_suggestion();
        }
        self.cursor = self.text.len();
    }

    /// Ctrl-W: smaze slovo pred kurzorem.
    pub fn kill_word(&mut self) {
        let before = &self.text[..self.cursor];
        let trimmed = before.trim_end();
        let start = trimmed.rfind([' ', '|', ';']).map_or(0, |i| i + 1);
        self.text.replace_range(start..self.cursor, "");
        self.cursor = start;
        self.changed();
    }

    /// Ctrl-U: smaze radek.
    pub fn clear(&mut self) {
        self.text.clear();
        self.cursor = 0;
        self.changed();
    }

    /// Enter: vrati radek a ulozi ho do historie.
    pub fn take(&mut self) -> String {
        let t = core::mem::take(&mut self.text);
        self.cursor = 0;
        if !t.trim().is_empty() && self.history.last() != Some(&t) {
            self.history.push(t.clone());
        }
        self.changed();
        t
    }

    /// Sipka nahoru: starsi radek z historie (rozepsany se zapamatuje).
    pub fn history_prev(&mut self) {
        if self.history.is_empty() {
            return;
        }
        let pos = match self.hist_pos {
            None => {
                self.draft = self.text.clone();
                self.history.len() - 1
            }
            Some(0) => 0,
            Some(p) => p - 1,
        };
        self.text = self.history[pos].clone();
        self.cursor = self.text.len();
        self.seq = self.seq.wrapping_add(1);
        self.hist_pos = Some(pos);
    }

    pub fn history_next(&mut self) {
        let Some(p) = self.hist_pos else { return };
        if p + 1 < self.history.len() {
            self.text = self.history[p + 1].clone();
            self.hist_pos = Some(p + 1);
        } else {
            self.text = core::mem::take(&mut self.draft);
            self.hist_pos = None;
        }
        self.cursor = self.text.len();
        self.seq = self.seq.wrapping_add(1);
    }

    pub fn history(&self) -> &[String] {
        &self.history
    }

    /// Napoveda z historie: zbytek nejnovejsiho radku, ktery zacina
    /// rozepsanym textem (jen s kurzorem na konci).
    pub fn suggestion(&self) -> Option<&str> {
        if self.text.is_empty() || self.cursor != self.text.len() || self.hist_pos.is_some() {
            return None;
        }
        self.history.iter().rev().find(|h| h.len() > self.text.len() && h.starts_with(&self.text)).map(|h| &h[self.text.len()..])
    }

    pub fn accept_suggestion(&mut self) {
        if let Some(s) = self.suggestion().map(String::from) {
            self.insert(&s);
        }
    }

    /// Prijmout vyznamove zvyrazneni od sluzby (jen pro aktualni text).
    pub fn set_semantic(&mut self, seq: u32, h: Highlight) {
        if seq == self.seq {
            self.semantic = Some((seq, h));
        }
    }

    /// Zvyrazneni aktualniho textu: vyznamove, kdyz patri k tomuto textu,
    /// jinak lexikalni (okamzite).
    pub fn highlight(&self) -> Highlight {
        match &self.semantic {
            Some((s, h)) if *s == self.seq => h.clone(),
            _ => highlight(&self.text, None),
        }
    }

    /// Zvyrazneni z protokolu (`Highlighted`).
    pub fn decode_spans(spans: &[(u32, u32, u8)], error: &Option<(u32, u32, String)>) -> Highlight {
        let spans = spans
            .iter()
            .filter_map(|(a, n, c)| {
                Some(Span { start: *a as usize, end: (*a + *n) as usize, class: Class::from_u8(*c)? })
            })
            .collect();
        let error = error.as_ref().map(|(a, n, m)| ErrorMark { start: *a as usize, end: (*a + *n) as usize, message: m.clone() });
        Highlight { spans, error }
    }
}
