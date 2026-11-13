//! Vyber textu v mrizce mysi (jako Windows Terminal): tah = znaky,
//! dvojklik = slovo (hranice slova dodava okno: `ui::text::word_at`),
//! trojklik = radky. Ctrl+C pri
//! vyberu (a Ctrl+Shift+C) kopiruje a vyber zrusi; bez vyberu Ctrl+C
//! prerusi prikaz.
//!
//! Vyber drzi bunky obrazovky (radek, sloupec). Kdyz novy vystup obsah
//! posune, vyber se posune s textem (hleda se stejny text vybranych radku
//! o kus vys); kdyz se text vybranych radku zmeni, vyber se zrusi.

use alloc::string::String;
use alloc::vec::Vec;

use crate::screen::Screen;

/// Hranice slova kolem bajtove pozice: `[od, do)` v bajtech.
pub type WordFn = fn(&str, usize) -> (usize, usize);

/// Jak vyber roste pri tahu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Char,
    Word,
    Line,
}

/// Bunka: (radek, sloupec).
pub type Pos = (usize, usize);

#[derive(Debug, Clone)]
pub struct Selection {
    pub anchor: Pos,
    pub head: Pos,
    pub mode: Mode,
    /// Text vybranych radku pri posledni zmene (sledovani posunu).
    rows: Vec<String>,
}

/// Stav vyberu v okne.
#[derive(Debug)]
pub struct Selector {
    pub sel: Option<Selection>,
    /// Hranice slova (dvojklik); `None` = jen znak pod ukazatelem.
    pub word_at: Option<WordFn>,
    /// Prodleva vicenasobneho kliku (ns).
    pub multi_click: u64,
    /// Tlacitko drzene od stisku v mrizce.
    pub dragging: bool,
    /// Posledni klik: cas, bunka, pocet (dvoj- a trojklik).
    last: Option<(u64, Pos, u8)>,
}

/// Text radku obrazovky (bunky, bez koncovych mezer).
pub fn row_text(screen: &Screen, row: usize) -> String {
    let s: String = screen.row_cells(row).iter().map(|c| c.ch).collect();
    String::from(s.trim_end())
}

/// Sloupec -> bajtova pozice v textu radku (a zpet).
fn byte_of(s: &str, col: usize) -> usize {
    s.char_indices().nth(col).map_or(s.len(), |(p, _)| p)
}

fn col_of(s: &str, byte: usize) -> usize {
    s[..byte.min(s.len())].chars().count()
}

/// Slovo kolem sloupce `col` jako sloupce `[a, b)`.
fn word(screen: &Screen, word_at: WordFn, (row, col): Pos) -> (usize, usize) {
    let s = row_text(screen, row);
    if col >= s.chars().count() {
        return (col, col + 1);
    }
    let (a, b) = word_at(&s, byte_of(&s, col));
    (col_of(&s, a), col_of(&s, b).max(col + 1))
}

impl Selector {
    pub fn new(word_at: Option<WordFn>, multi_click: u64) -> Self {
        Self { sel: None, word_at, multi_click, dragging: false, last: None }
    }

    /// Stisk leveho tlacitka v mrizce (`shift` = rozsirit vyber).
    pub fn press(&mut self, screen: &Screen, at: Pos, time: u64, shift: bool) {
        if shift && let Some(s) = &mut self.sel {
            s.head = at;
            self.dragging = true;
            self.snapshot(screen);
            return;
        }
        let count = match self.last {
            Some((t, p, n)) if p == at && time.saturating_sub(t) <= self.multi_click => n % 3 + 1,
            _ => 1,
        };
        self.last = Some((time, at, count));
        let mode = match count {
            2 => Mode::Word,
            3 => Mode::Line,
            _ => Mode::Char,
        };
        self.sel = Some(Selection { anchor: at, head: at, mode, rows: Vec::new() });
        self.dragging = true;
        self.snapshot(screen);
    }

    /// Tah s drzenym tlacitkem; `true` = vyber se zmenil.
    pub fn drag(&mut self, screen: &Screen, at: Pos) -> bool {
        if !self.dragging {
            return false;
        }
        let Some(s) = &mut self.sel else { return false };
        if s.head == at {
            return false;
        }
        s.head = at;
        self.snapshot(screen);
        true
    }

    pub fn release(&mut self) {
        self.dragging = false;
        // Klik bez tahu ve znakovem rezimu nic nevybere.
        if self.sel.as_ref().is_some_and(|s| s.mode == Mode::Char && s.anchor == s.head) {
            self.sel = None;
        }
    }

    pub fn clear(&mut self) -> bool {
        self.dragging = false;
        self.sel.take().is_some()
    }

    /// Vybrane bunky po radcich: (radek, od, do) se sloupci `[od, do)`.
    pub fn spans(&self, screen: &Screen) -> Vec<(usize, usize, usize)> {
        let Some(s) = &self.sel else { return Vec::new() };
        let (mut a, mut b) = if s.anchor <= s.head { (s.anchor, s.head) } else { (s.head, s.anchor) };
        let cols = screen.cols;
        match s.mode {
            Mode::Char => b.1 += 1,
            Mode::Word => {
                let w = |p: Pos| match self.word_at {
                    Some(f) => word(screen, f, p),
                    None => (p.1, p.1 + 1),
                };
                a.1 = w(a).0;
                b.1 = w(b).1;
            }
            Mode::Line => {
                a.1 = 0;
                b.1 = cols;
            }
        }
        let mut out = Vec::new();
        for row in a.0..=b.0.min(screen.rows.saturating_sub(1)) {
            let from = if row == a.0 { a.1 } else { 0 };
            let to = if row == b.0 { b.1 } else { cols };
            if from < to {
                out.push((row, from, to.min(cols)));
            }
        }
        out
    }

    /// Vybrany text: radky oddelene `\n`, bez koncovych mezer.
    pub fn text(&self, screen: &Screen) -> String {
        let mut out = String::new();
        for (k, (row, from, to)) in self.spans(screen).into_iter().enumerate() {
            if k > 0 {
                out.push('\n');
            }
            let cells: String = screen.row_cells(row).iter().skip(from).take(to - from).map(|c| c.ch).collect();
            out.push_str(cells.trim_end());
        }
        out
    }

    fn snapshot(&mut self, screen: &Screen) {
        let spans = self.spans(screen);
        if let Some(s) = &mut self.sel {
            s.rows = spans.iter().map(|&(r, ..)| row_text(screen, r)).collect();
        }
    }

    /// Po novem snimku obrazovky: vyber se posune s textem, nebo zrusi.
    /// `true` = vyber se zrusil nebo posunul (prekreslit).
    pub fn follow(&mut self, screen: &Screen) -> bool {
        let Some(s) = &self.sel else { return false };
        if s.rows.is_empty() {
            return false;
        }
        let first = s.anchor.0.min(s.head.0);
        let same = |d: usize| s.rows.iter().enumerate().all(|(k, t)| first + k >= d && row_text(screen, first + k - d) == *t);
        if same(0) {
            return false;
        }
        // Novy vystup posouva obsah nahoru.
        match (1..=first).find(|&d| same(d)) {
            Some(d) if !self.dragging => {
                let s = self.sel.as_mut().unwrap();
                s.anchor.0 -= d;
                s.head.0 -= d;
            }
            _ => {
                self.sel = None;
                self.dragging = false;
            }
        }
        true
    }
}
