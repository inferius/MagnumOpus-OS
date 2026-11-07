//! Radky obrazovky ze stylovanych useku (ANSI 24bit vcetne pozadi),
//! s orezem na sirku.

use alloc::string::String;
use alloc::vec::Vec;

use crate::render::RESET;
use crate::theme::{Rgb, Style};

#[derive(Debug, Clone, Default)]
pub struct Line {
    parts: Vec<(Option<Style>, Option<Rgb>, String)>,
    /// Pozadi celeho radku (doplni se do sirky).
    fill: Option<Rgb>,
}

fn esc(style: Option<Style>, bg: Option<Rgb>) -> String {
    let mut out = String::from("\x1b[0");
    if let Some(s) = style {
        out.push_str(&alloc::format!(";38;2;{};{};{}", s.fg.0, s.fg.1, s.fg.2));
        if s.bold {
            out.push_str(";1");
        }
        if s.italic {
            out.push_str(";3");
        }
        if s.underline {
            out.push_str(";4");
        }
    }
    if let Some(b) = bg {
        out.push_str(&alloc::format!(";48;2;{};{};{}", b.0, b.1, b.2));
    }
    out.push('m');
    out
}

impl Line {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn text(mut self, style: Option<Style>, text: &str) -> Self {
        self.push(style, text);
        self
    }

    pub fn push(&mut self, style: Option<Style>, text: &str) {
        self.parts.push((style, None, text.into()));
    }

    pub fn push_bg(&mut self, style: Option<Style>, bg: Rgb, text: &str) {
        self.parts.push((style, Some(bg), text.into()));
    }

    /// Pozadi celeho radku (vybrany radek, listy).
    pub fn fill(mut self, bg: Rgb) -> Self {
        self.fill = Some(bg);
        self
    }

    pub fn width(&self) -> usize {
        self.parts.iter().map(|(_, _, t)| t.chars().count()).sum()
    }

    /// Text bez stylu (testy).
    pub fn plain(&self) -> String {
        self.parts.iter().map(|(_, _, t)| t.as_str()).collect()
    }

    /// ANSI oriznute na `cols` znaku (s doplnenim pozadi radku).
    pub fn ansi(&self, cols: usize) -> String {
        let mut out = String::new();
        let mut left = cols;
        for (st, bg, t) in &self.parts {
            if left == 0 {
                break;
            }
            let take: String = t.chars().take(left).collect();
            left -= take.chars().count();
            let bg = bg.or(self.fill);
            if st.is_some() || bg.is_some() {
                out.push_str(&esc(*st, bg));
                out.push_str(&take);
                out.push_str(RESET);
            } else {
                out.push_str(&take);
            }
        }
        if let Some(bg) = self.fill
            && left > 0
        {
            out.push_str(&esc(None, Some(bg)));
            out.push_str(&" ".repeat(left));
            out.push_str(RESET);
        }
        out
    }
}

/// Usek ANSI textu (z `render`) rozdeleny na radky jako [`Line`] bez
/// dalsiho stylu (styl je uz v textu).
pub fn lines_of(ansi: &str) -> Vec<String> {
    ansi.lines().map(String::from).collect()
}

/// Orez ANSI textu na `cols` viditelnych znaku (escape sekvence se
/// nepocitaji).
pub fn clip(s: &str, cols: usize) -> String {
    let mut out = String::new();
    let mut n = 0;
    let mut it = s.chars().peekable();
    while let Some(c) = it.next() {
        if c == '\x1b' {
            out.push(c);
            if it.peek() == Some(&'[') {
                for d in it.by_ref() {
                    out.push(d);
                    if ('@'..='~').contains(&d) && d != '[' {
                        break;
                    }
                }
            }
            continue;
        }
        if n < cols {
            out.push(c);
            n += 1;
        }
    }
    out.push_str(RESET);
    out
}
