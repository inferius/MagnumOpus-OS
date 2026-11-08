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

    /// Viditelna sirka (escape sekvence uvnitr useku se nepocitaji).
    pub fn width(&self) -> usize {
        self.parts.iter().map(|(_, _, t)| take_visible(t, usize::MAX).1).sum()
    }

    /// Text bez stylu (testy).
    pub fn plain(&self) -> String {
        self.parts.iter().map(|(_, _, t)| t.as_str()).collect()
    }

    /// ANSI oriznute na `cols` znaku (s doplnenim pozadi radku). Delsi
    /// radek konci vypustkou (je videt, ze pokracuje).
    pub fn ansi(&self, cols: usize) -> String {
        let mut out = String::new();
        let over = self.width() > cols && cols > 0;
        let mut left = if over { cols - 1 } else { cols };
        let mut last = (None, None);
        for (st, bg, t) in &self.parts {
            if left == 0 {
                break;
            }
            last = (*st, bg.or(self.fill));
            let (take, n) = take_visible(t, left);
            left -= n;
            let bg = bg.or(self.fill);
            if st.is_some() || bg.is_some() {
                out.push_str(&esc(*st, bg));
                out.push_str(&take);
                out.push_str(RESET);
            } else {
                out.push_str(&take);
            }
        }
        if over {
            out.push_str(&esc(last.0, last.1));
            out.push('\u{2026}');
            out.push_str(RESET);
            return out;
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

/// Prvnich `n` viditelnych znaku `t` (escape sekvence se kopiruji, ale
/// nepocitaji); vrati text a pocet viditelnych znaku.
fn take_visible(t: &str, n: usize) -> (String, usize) {
    let mut out = String::new();
    let mut k = 0;
    let mut it = t.chars().peekable();
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
        if k == n {
            break;
        }
        out.push(c);
        k += 1;
    }
    (out, k)
}

/// Usek ANSI textu (z `render`) rozdeleny na radky jako [`Line`] bez
/// dalsiho stylu (styl je uz v textu).
pub fn lines_of(ansi: &str) -> Vec<String> {
    ansi.lines().map(String::from).collect()
}

/// Zalomi ANSI text na radky nejvys `cols` viditelnych znaku (radeji
/// v mezere). Posledni styl (SGR) se na dalsim radku zopakuje.
pub fn wrap(s: &str, cols: usize) -> Vec<String> {
    let cols = cols.max(1);
    // Tokeny: (escape?, text).
    let mut toks: Vec<(bool, String)> = Vec::new();
    let mut it = s.chars().peekable();
    while let Some(c) = it.next() {
        if c == '\x1b' {
            let mut e = String::from(c);
            if it.peek() == Some(&'[') {
                for d in it.by_ref() {
                    e.push(d);
                    if ('@'..='~').contains(&d) && d != '[' {
                        break;
                    }
                }
            }
            toks.push((true, e));
        } else {
            toks.push((false, String::from(c)));
        }
    }
    let mut out = Vec::new();
    let mut line = String::new();
    let mut sgr = String::new();
    let mut n = 0;
    // Posledni mezera v `line`: (bajt, znaku za ni, styl v ni).
    let mut space: Option<(usize, usize, String)> = None;
    for (esc, t) in toks {
        if esc {
            if t.ends_with('m') {
                sgr = t.clone();
            }
            line.push_str(&t);
            continue;
        }
        if n == cols {
            let (rest, rest_n) = match space.take() {
                Some((at, k, st)) if k < cols => (alloc::format!("{st}{}", line.split_off(at + 1)), k),
                _ => (sgr.clone(), 0),
            };
            line.push_str(RESET);
            out.push(core::mem::take(&mut line));
            line = rest;
            n = rest_n;
            if t == " " && n == 0 {
                continue;
            }
        }
        line.push_str(&t);
        n += 1;
        if t == " " {
            space = Some((line.len() - 1, 0, sgr.clone()));
        } else if let Some(sp) = space.as_mut() {
            sp.1 += 1;
        }
    }
    if !line.is_empty() || out.is_empty() {
        out.push(line);
    }
    out
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
            match it.peek() {
                // CSI: do koncoveho znaku.
                Some('[') => {
                    for d in it.by_ref() {
                        out.push(d);
                        if ('@'..='~').contains(&d) && d != '[' {
                            break;
                        }
                    }
                }
                // OSC (odkaz): do BEL nebo ESC \.
                Some(']') => {
                    while let Some(d) = it.next() {
                        out.push(d);
                        if d == '\x07' {
                            break;
                        }
                        if d == '\x1b' && it.peek() == Some(&'\\') {
                            out.push(it.next().unwrap());
                            break;
                        }
                    }
                }
                _ => {}
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
