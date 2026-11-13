//! Editace textu nezavisla na kresleni: hranice grafemu a slov, kurzor
//! s vyberem, vkladani a mazani, historie zpet / znovu.
//!
//! Pouziva ji textove pole knihovny i aplikace, ktere kresli samy (vyber
//! v mrizce terminalu). Pozice jsou bajtove offsety v UTF-8 a vzdy lezi
//! na hranici grafemu.
//!
//! Grafemy jsou zjednodusene UAX #29: zakladni znak a za nim kombinacni
//! znaky, variacni selektory, modifikatory barvy pleti, spojeni ZWJ
//! (emoji rodina), pary regionalnich indikatoru (vlajky) a CR LF jako
//! jeden znak. Hangul a indicka pisma bez shapingu nejsou.

use alloc::string::String;
use alloc::vec::Vec;

/// Pripojuje se znak `c` k predchozimu grafemu?
fn extends(c: char) -> bool {
    matches!(c as u32,
        0x0300..=0x036F   // kombinacni diakritika (hacek, carka jako samostatne znaky)
        | 0x0483..=0x0489
        | 0x0591..=0x05BD
        | 0x0610..=0x061A
        | 0x064B..=0x065F
        | 0x0E31 | 0x0E34..=0x0E3A | 0x0E47..=0x0E4E
        | 0x1AB0..=0x1AFF
        | 0x1DC0..=0x1DFF
        | 0x200C          // ZWNJ
        | 0x20D0..=0x20FF
        | 0xFE00..=0xFE0F // variacni selektory
        | 0xFE20..=0xFE2F
        | 0x1F3FB..=0x1F3FF // barva pleti
        | 0xE0020..=0xE007F // tagy (vlajky regionu)
        | 0xE0100..=0xE01EF)
}

const ZWJ: char = '\u{200D}';

fn regional(c: char) -> bool {
    (0x1F1E6..=0x1F1FF).contains(&(c as u32))
}

/// Konec grafemu, ktery zacina na `i` (`i` < delka).
pub fn next_boundary(s: &str, i: usize) -> usize {
    let mut it = s[i..].char_indices();
    let Some((_, first)) = it.next() else { return s.len() };
    let mut end = i + first.len_utf8();
    if first == '\r' && s[end..].starts_with('\n') {
        return end + 1;
    }
    if first == '\n' || first == '\r' {
        return end;
    }
    let mut prev = first;
    let mut ri = regional(first) as u32;
    for (o, c) in it {
        let join = extends(c) || c == ZWJ || prev == ZWJ || (regional(c) && ri % 2 == 1);
        if !join {
            break;
        }
        if regional(c) {
            ri += 1;
        }
        prev = c;
        end = i + o + c.len_utf8();
    }
    end
}

/// Zacatek grafemu, ktery konci na `i` (`i` > 0).
pub fn prev_boundary(s: &str, i: usize) -> usize {
    if i == 0 {
        return 0;
    }
    // Od zacatku radku dopredu (regionalni indikatory se paruji zleva).
    let start = s[..i].rfind('\n').map_or(0, |p| p + 1);
    let start = if start == i { s[..i - 1].rfind('\n').map_or(0, |p| p + 1) } else { start };
    let mut b = start;
    loop {
        let n = next_boundary(s, b);
        if n >= i {
            return b;
        }
        b = n;
    }
}

/// Hranice grafemu v `s` vcetne 0 a delky.
pub fn boundaries(s: &str) -> Vec<usize> {
    let mut v = alloc::vec![0];
    let mut i = 0;
    while i < s.len() {
        i = next_boundary(s, i);
        v.push(i);
    }
    v
}

/// Pocet grafemu (delka pro cloveka: "Čaj" = 3 i s rozlozenym hackem).
pub fn grapheme_count(s: &str) -> usize {
    boundaries(s).len() - 1
}

/// Druh znaku pro hranice slov.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Class {
    Space,
    Word,
    Punct,
}

fn class(c: char) -> Class {
    if c.is_whitespace() {
        Class::Space
    } else if c.is_alphanumeric() || c == '_' || extends(c) {
        Class::Word
    } else {
        Class::Punct
    }
}

fn class_at(s: &str, i: usize) -> Class {
    s[i..].chars().next().map_or(Class::Space, class)
}

/// Zacatek dalsiho slova (Ctrl+sipka vpravo): preskoci zbytek slova
/// a mezery za nim.
pub fn next_word(s: &str, mut i: usize) -> usize {
    if i >= s.len() {
        return s.len();
    }
    let c = class_at(s, i);
    while i < s.len() && class_at(s, i) == c && c != Class::Space {
        i = next_boundary(s, i);
    }
    while i < s.len() && class_at(s, i) == Class::Space {
        i = next_boundary(s, i);
    }
    i
}

/// Zacatek predchoziho slova (Ctrl+sipka vlevo).
pub fn prev_word(s: &str, mut i: usize) -> usize {
    while i > 0 && class_at(s, prev_boundary(s, i)) == Class::Space {
        i = prev_boundary(s, i);
    }
    if i == 0 {
        return 0;
    }
    let c = class_at(s, prev_boundary(s, i));
    while i > 0 && class_at(s, prev_boundary(s, i)) == c {
        i = prev_boundary(s, i);
    }
    i
}

/// Slovo (nebo souvisly usek mezer ci interpunkce) kolem `i` (dvojklik).
pub fn word_at(s: &str, i: usize) -> (usize, usize) {
    if s.is_empty() {
        return (0, 0);
    }
    let i = if i >= s.len() { prev_boundary(s, s.len()) } else { i };
    let c = class_at(s, i);
    let mut a = i;
    while a > 0 && class_at(s, prev_boundary(s, a)) == c {
        a = prev_boundary(s, a);
    }
    let mut b = i;
    while b < s.len() && class_at(s, b) == c {
        b = next_boundary(s, b);
    }
    (a, b)
}

/// Radek (logicky, podle `\n`) kolem `i` bez znaku konce radku.
pub fn line_at(s: &str, i: usize) -> (usize, usize) {
    let a = s[..i.min(s.len())].rfind('\n').map_or(0, |p| p + 1);
    let b = s[a..].find('\n').map_or(s.len(), |p| a + p);
    (a, b)
}

/// Druh posledni upravy (slucovani psani do jednoho kroku zpet).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EditKind {
    Type,
    Delete,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Snapshot {
    text: String,
    cursor: usize,
    anchor: usize,
}

/// Nejvic kroku historie (starsi se zahodi).
const MAX_UNDO: usize = 200;

/// Upravovany text s kurzorem, vyberem a historii.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextEdit {
    text: String,
    /// Kurzor (konec vyberu) a kotva (zacatek vyberu); bez vyberu stejne.
    cursor: usize,
    anchor: usize,
    multiline: bool,
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
    last: EditKind,
    /// Pozadovana vodorovna poloha pro pohyb nahoru a dolu (px), at kurzor
    /// pri pruchodu kratkym radkem neskace doleva.
    pub goal_x: Option<i32>,
}

impl TextEdit {
    pub fn new(text: &str, multiline: bool) -> Self {
        let text = if multiline { String::from(text) } else { single_line(text) };
        let n = text.len();
        Self { text, cursor: n, anchor: n, multiline, undo: Vec::new(), redo: Vec::new(), last: EditKind::Other, goal_x: None }
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    pub fn anchor(&self) -> usize {
        self.anchor
    }

    pub fn multiline(&self) -> bool {
        self.multiline
    }

    /// Vyber jako rozsah (zacatek < konec), `None` = jen kurzor.
    pub fn selection(&self) -> Option<(usize, usize)> {
        (self.cursor != self.anchor).then(|| (self.cursor.min(self.anchor), self.cursor.max(self.anchor)))
    }

    pub fn selected_text(&self) -> &str {
        self.selection().map_or("", |(a, b)| &self.text[a..b])
    }

    /// Text zmenila aplikace (ne psani): prevezme ho, kurzor a vyber
    /// zustanou, jak to jde. Historie se zahodi jen pri uplne jine
    /// hodnote, ne pri drobne uprave aplikaci (napr. odfiltrovany znak).
    pub fn set_text(&mut self, t: &str) {
        let t = if self.multiline { String::from(t) } else { single_line(t) };
        if t == self.text {
            return;
        }
        self.text = t;
        self.cursor = self.snap(self.cursor.min(self.text.len()));
        self.anchor = self.snap(self.anchor.min(self.text.len()));
        self.last = EditKind::Other;
    }

    /// Nejblizsi hranice grafemu na `i` nebo pred nim.
    fn snap(&self, i: usize) -> usize {
        let mut i = i.min(self.text.len());
        while !self.text.is_char_boundary(i) {
            i -= 1;
        }
        if i == 0 || i == self.text.len() {
            return i;
        }
        let b = prev_boundary(&self.text, i);
        if next_boundary(&self.text, b) == i { i } else { b }
    }

    /// Kurzor na `i` (`extend` = rozsirit vyber, jinak zrusit).
    pub fn set_cursor(&mut self, i: usize, extend: bool) {
        self.cursor = self.snap(i);
        if !extend {
            self.anchor = self.cursor;
        }
        self.last = EditKind::Other;
    }

    /// Vyber rozsahu (dvojklik slovo, trojklik radek, Ctrl+A).
    pub fn select(&mut self, a: usize, b: usize) {
        self.anchor = self.snap(a);
        self.cursor = self.snap(b);
        self.last = EditKind::Other;
    }

    pub fn select_all(&mut self) {
        self.select(0, self.text.len());
    }

    /// Pohyb kurzoru. Bez `extend` zrusi vyber a pri vyberu skoci na jeho
    /// okraj (sipka vlevo na zacatek, vpravo na konec).
    pub fn move_to(&mut self, m: Motion, extend: bool) {
        let s = &self.text;
        if !extend
            && let Some((a, b)) = self.selection()
            && matches!(m, Motion::Left | Motion::Right)
        {
            let i = if m == Motion::Left { a } else { b };
            self.set_cursor(i, false);
            return;
        }
        let c = self.cursor;
        let i = match m {
            Motion::Left => prev_boundary(s, c),
            Motion::Right if c < s.len() => next_boundary(s, c),
            Motion::Right => c,
            Motion::WordLeft => prev_word(s, c),
            Motion::WordRight => next_word(s, c),
            Motion::LineStart => line_at(s, c).0,
            Motion::LineEnd => line_at(s, c).1,
            Motion::Start => 0,
            Motion::End => s.len(),
        };
        self.set_cursor(i, extend);
        if !matches!(m, Motion::LineStart | Motion::LineEnd) {
            self.goal_x = None;
        }
    }

    fn save(&mut self, kind: EditKind) {
        // Psani a mazani po sobe = jeden krok zpet; mezera ukonci slovo.
        let merge = kind != EditKind::Other && kind == self.last;
        if !merge {
            self.undo.push(Snapshot { text: self.text.clone(), cursor: self.cursor, anchor: self.anchor });
            if self.undo.len() > MAX_UNDO {
                self.undo.remove(0);
            }
        }
        self.redo.clear();
        self.last = kind;
    }

    /// Nahradi vyber (nebo vlozi na kurzor) textem `t`. Jednoradkove pole
    /// konce radku nahradi mezerou (vlozeni viceradkoveho textu).
    pub fn insert(&mut self, t: &str) {
        let t = if self.multiline { String::from(t) } else { single_line(t) };
        if t.is_empty() && self.selection().is_none() {
            return;
        }
        let typing = t.chars().count() == 1 && !t.starts_with(char::is_whitespace);
        self.save(if typing && self.selection().is_none() { EditKind::Type } else { EditKind::Other });
        let (a, b) = self.selection().unwrap_or((self.cursor, self.cursor));
        self.text.replace_range(a..b, &t);
        self.cursor = a + t.len();
        self.anchor = self.cursor;
        self.goal_x = None;
        if !typing {
            self.last = EditKind::Other;
        }
    }

    /// Smaze vyber, jinak grafem / slovo pred kurzorem (`back`) nebo za
    /// nim. Vraci, zda se neco zmenilo.
    pub fn delete(&mut self, back: bool, word: bool) -> bool {
        let (a, b) = match self.selection() {
            Some(r) => r,
            None => {
                let c = self.cursor;
                let s = &self.text;
                match (back, word) {
                    (true, false) => (prev_boundary(s, c), c),
                    (true, true) => (prev_word(s, c), c),
                    (false, false) if c < s.len() => (c, next_boundary(s, c)),
                    (false, true) => (c, next_word(s, c)),
                    _ => (c, c),
                }
            }
        };
        if a == b {
            return false;
        }
        let single = self.selection().is_none() && !word;
        self.save(if single { EditKind::Delete } else { EditKind::Other });
        self.text.replace_range(a..b, "");
        self.cursor = a;
        self.anchor = a;
        self.goal_x = None;
        true
    }

    /// Krok zpet. Vraci, zda se neco zmenilo.
    pub fn undo(&mut self) -> bool {
        let Some(s) = self.undo.pop() else { return false };
        self.redo.push(Snapshot { text: core::mem::take(&mut self.text), cursor: self.cursor, anchor: self.anchor });
        self.restore(s);
        true
    }

    pub fn redo(&mut self) -> bool {
        let Some(s) = self.redo.pop() else { return false };
        self.undo.push(Snapshot { text: core::mem::take(&mut self.text), cursor: self.cursor, anchor: self.anchor });
        self.restore(s);
        true
    }

    fn restore(&mut self, s: Snapshot) {
        self.text = s.text;
        self.cursor = s.cursor;
        self.anchor = s.anchor;
        self.last = EditKind::Other;
        self.goal_x = None;
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }
}

/// Pohyb kurzoru.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Motion {
    Left,
    Right,
    WordLeft,
    WordRight,
    /// Zacatek a konec (logickeho) radku: Home a End.
    LineStart,
    LineEnd,
    /// Zacatek a konec textu: Ctrl+Home, Ctrl+End.
    Start,
    End,
}

/// Jednoradkova hodnota: konce radku a tabulatory na mezery.
fn single_line(t: &str) -> String {
    let t = t.replace("\r\n", " ");
    t.chars().map(|c| if c == '\n' || c == '\r' || c == '\t' { ' ' } else { c }).collect()
}
