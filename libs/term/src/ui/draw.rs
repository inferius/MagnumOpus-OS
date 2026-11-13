//! Radky obrazovky ze stylovanych useku (ANSI 24bit vcetne pozadi),
//! s orezem na sirku.

use alloc::string::String;
use alloc::vec::Vec;

use crate::render::RESET;
use crate::theme::{Rgb, Style};

/// Stav bloku pro levy pruh (okno GUI ho kresli barvou stavu).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Mark {
    #[default]
    Run,
    Ok,
    Err,
}

/// Co radek znamena: okno GUI podle toho dokresli prvky navrhu (pruh
/// bloku, cas vpravo, pilulka slozky, stitky klaves, ramecek chyby).
/// Text radku je vzdy uplny (konzole a cizi terminal role nepotrebuji).
#[derive(Debug, Clone, Default, PartialEq)]
pub enum Role {
    #[default]
    Plain,
    /// Stavovy radek nahore.
    Status,
    /// Hlavicka bloku: prvni usek je znacka (`| `), od useku `meta` cas
    /// a vysledek (okno je kresli vpravo pismem UI).
    Head { mark: Mark, meta: usize },
    /// Radek obsahu bloku; `error` = hlaseni chyby, `table_head` = zahlavi
    /// tabulky.
    Body { mark: Mark, error: bool, table_head: bool },
    /// Radek vstupu: prvnich `prefix` useku je slozka a oddelovac (okno je
    /// nahradi pilulkou s textem `cwd`).
    Input { prefix: usize, cwd: String },
    /// Napoveda klaves: (klavesa, popis).
    Hints(Vec<(String, String)>),
}

#[derive(Debug, Clone, Default)]
pub struct Line {
    parts: Vec<(Option<Style>, Option<Rgb>, String)>,
    /// Pozadi celeho radku (doplni se do sirky).
    fill: Option<Rgb>,
    pub role: Role,
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
    pub fn role(mut self, r: Role) -> Self {
        self.role = r;
        self
    }

    /// Pocet useku (pro role, ktere se odkazuji na useky).
    pub fn parts_len(&self) -> usize {
        self.parts.len()
    }

    /// Sloupce, ktere useky zabiraji: (od, do) pro kazdy usek.
    pub fn part_cols(&self) -> Vec<(usize, usize)> {
        let mut at = 0;
        self.parts
            .iter()
            .map(|(_, _, t)| {
                let w = take_visible(t, usize::MAX).1;
                let r = (at, at + w);
                at += w;
                r
            })
            .collect()
    }

    /// Text useku bez stylu (okno kresli nektere useky samo).
    pub fn part_text(&self, i: usize) -> String {
        self.parts.get(i).map(|(_, _, t)| String::from(t)).unwrap_or_default()
    }

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
///
/// Konce radku podle pravidla systemu (architektura, Text a konce
/// radku): `\n` i `\r\n` je jeden konec radku, osamocene `\r` na konci se
/// nezobrazuje, `\r` uvnitr radku vraci na jeho zacatek (dalsi text
/// prepise drivejsi, jako ukazatel postupu).
pub fn lines_of(ansi: &str) -> Vec<String> {
    ansi.lines().map(carriage).collect()
}

/// Radek s `\r` uvnitr: vysledek prepisovani od zacatku.
fn carriage(line: &str) -> String {
    let line = line.trim_end_matches('\r');
    if !line.contains('\r') {
        return line.into();
    }
    if line.contains('\x1b') {
        // Se styly: jen posledni usek (prepis po znacich by rozbil escape).
        return line.rsplit('\r').next().unwrap_or("").into();
    }
    let mut cells: Vec<char> = Vec::new();
    let mut x = 0;
    for c in line.chars() {
        if c == '\r' {
            x = 0;
            continue;
        }
        if x < cells.len() {
            cells[x] = c;
        } else {
            cells.push(c);
        }
        x += 1;
    }
    cells.into_iter().collect()
}

/// Styl (posledni SGR) aktivni na konci textu: prazdne, kdyz posledni
/// SGR styl zrusil (`ESC[0m`). Pro prenos stylu na dalsi radek.
pub fn active_sgr(s: &str) -> String {
    let mut cur = String::new();
    let mut it = s.char_indices().peekable();
    while let Some((i, c)) = it.next() {
        if c != '\x1b' || it.peek().map(|p| p.1) != Some('[') {
            continue;
        }
        let mut end = i;
        for (j, d) in it.by_ref() {
            end = j + d.len_utf8();
            if ('@'..='~').contains(&d) && d != '[' {
                break;
            }
        }
        let seq = &s[i..end];
        if seq.ends_with('m') {
            cur = if seq == "\x1b[0m" || seq == "\x1b[m" { String::new() } else { String::from(seq) };
        }
    }
    cur
}

/// Odsazeni pokracovani zalomeneho radku: u radku tabulky (sloupce
/// oddelene aspon dvema mezerami) pod posledni bunku, jinak pod zacatek
/// textu. Nejvys polovina sirky (jinak by na popis nezbylo misto).
fn hang(visible: &str, cols: usize) -> usize {
    let chars: Vec<char> = visible.chars().collect();
    let lead = chars.iter().take_while(|c| **c == ' ').count();
    let mut col = lead;
    let mut i = lead;
    while i + 1 < chars.len() {
        if chars[i] == ' ' && chars[i + 1] == ' ' {
            let mut j = i;
            while j < chars.len() && chars[j] == ' ' {
                j += 1;
            }
            if j < chars.len() {
                col = j;
            }
            i = j;
        } else {
            i += 1;
        }
    }
    if col * 2 > cols { lead.min(cols / 2) } else { col }
}

/// Zalomi ANSI text na radky nejvys `cols` viditelnych znaku (radeji
/// v mezere). Posledni styl (SGR) se na dalsim radku zopakuje;
/// pokracovani je odsazene (u tabulky pod bunku popisu, viz [`hang`]).
pub fn wrap(s: &str, cols: usize) -> Vec<String> {
    let cols = cols.max(1);
    let visible: String = {
        let (t, _) = take_visible(s, usize::MAX);
        let mut v = String::new();
        let mut it = t.chars().peekable();
        while let Some(c) = it.next() {
            if c == '\x1b' {
                if it.peek() == Some(&'[') {
                    for d in it.by_ref() {
                        if ('@'..='~').contains(&d) && d != '[' {
                            break;
                        }
                    }
                }
            } else {
                v.push(c);
            }
        }
        v
    };
    let indent = " ".repeat(hang(&visible, cols));
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
            let ind = indent.chars().count();
            let (rest, rest_n) = match space.take() {
                Some((at, k, st)) if k + ind < cols => (alloc::format!("{indent}{st}{}", line.split_off(at + 1)), ind + k),
                _ => (alloc::format!("{indent}{sgr}"), ind),
            };
            line.push_str(RESET);
            out.push(core::mem::take(&mut line));
            line = rest;
            n = rest_n;
            if t == " " && n == indent.chars().count() {
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

/// Napoveda klaves ("Tab doplnit · Esc zavrit") jako stitky: (klavesa,
/// popis). Klavesa je prvni slovo, "sipka nahoru" a "sipky" jako sipky.
pub fn hint_chips(text: &str) -> Vec<(String, String)> {
    text.split(" \u{b7} ")
        .map(|item| {
            let item = item.trim();
            for (prefix, key) in [("šipka nahoru ", "\u{2191}"), ("šipky ", "\u{2191}\u{2193}"), ("kolečko", "kolečko")] {
                if let Some(rest) = item.strip_prefix(prefix) {
                    return (String::from(key), String::from(rest.trim()));
                }
            }
            match item.split_once(' ') {
                Some((k, l)) => (String::from(k), String::from(l)),
                None => (String::from(item), String::new()),
            }
        })
        .collect()
}
