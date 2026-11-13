//! Prvky navrhu terminalu (design/terminal.html) nad mrizkou znaku: podle
//! roli radku (`term::ui::draw::Role`) okno dokresli levy pruh bloku, cas
//! vpravo, pilulku slozky, stitky klaves, ramecek chyby a linku pod
//! zahlavim tabulky. Text dal kresli mrizka; bunky, ktere prvek nahrazuje,
//! se preskoci (`skips`).

use alloc::string::String;
use alloc::vec::Vec;

use gfx::{Canvas, Rect};
use term::ui::draw::{Line, Mark, Role};

use crate::Look;

/// Mrizka v okne: pocatek a velikost bunky.
pub struct Grid {
    pub x: i32,
    pub y: i32,
    pub cw: u32,
    pub ch: u32,
    pub cols: usize,
}

impl Grid {
    fn row_y(&self, row: usize) -> i32 {
        self.y + (row as u32 * self.ch) as i32
    }

    fn col_x(&self, col: usize) -> i32 {
        self.x + (col as u32 * self.cw) as i32
    }

    fn right(&self) -> i32 {
        self.col_x(self.cols)
    }
}

/// Sloupce, ktere mrizka v radku nekresli (nahrazuje je prvek).
pub fn skips(lines: &[Line]) -> Vec<Vec<(usize, usize)>> {
    lines
        .iter()
        .map(|l| match &l.role {
            // Znacka bloku (pruh) a cas vpravo.
            Role::Head { meta, .. } => {
                let cols = l.part_cols();
                let mut v = alloc::vec![(0, 1)];
                if let (Some(a), Some(b)) = (cols.get(*meta), cols.last()) {
                    v.push((a.0, b.1));
                }
                v
            }
            // Slozka a oddelovac (pilulka).
            Role::Input { prefix, .. } => match l.part_cols().get(prefix.saturating_sub(1)) {
                Some(&(_, end)) => alloc::vec![(0, end)],
                None => Vec::new(),
            },
            // Napoveda klaves: cely radek jsou stitky.
            Role::Hints(_) => alloc::vec![(0, usize::MAX)],
            _ => Vec::new(),
        })
        .collect()
}

fn mark_color(look: &Look, m: Mark) -> u32 {
    let col = &look.t.color;
    match m {
        Mark::Run => col.accent,
        Mark::Ok => col.ok,
        Mark::Err => col.danger,
    }
}

/// Pod textem: ramecky chyb, pilulka slozky, linka nad radkem vstupu.
pub fn under(c: &mut Canvas<'_>, look: &mut Look, lines: &[Line], g: &Grid) {
    let t = look.t.clone();
    let col = &t.color;
    let mut row = 0;
    while row < lines.len() {
        match &lines[row].role {
            Role::Body { error: true, .. } => {
                // Souvisle radky hlaseni chyby = jeden ramecek.
                let first = row;
                let mut width = 0;
                while row < lines.len() && matches!(lines[row].role, Role::Body { error: true, .. }) {
                    width = width.max(lines[row].width());
                    row += 1;
                }
                let x = g.col_x(2) - 6;
                let w = ((width.saturating_sub(2)) as u32 * g.cw + 20).min((g.right() - x).max(0) as u32);
                let r = Rect::new(x, g.row_y(first) + 1, w, (row - first) as u32 * g.ch - 2);
                c.round_fill_bordered(r, t.radius.control, col.danger_soft, col.danger, 1);
                continue;
            }
            Role::Input { prefix, cwd } => {
                let y = g.row_y(row);
                c.fill(Rect::new(g.x, y - 3, (g.right() - g.x).max(0) as u32, 1), col.border);
                if let Some(&(_, end)) = lines[row].part_cols().get(prefix.saturating_sub(1)) {
                    let w = (end as u32 * g.cw).saturating_sub(g.cw);
                    let r = Rect::new(g.x, y + 1, w, g.ch - 2);
                    c.round_fill(r, (g.ch - 2) / 2, col.accent_soft);
                    let s = t.text.small;
                    let f = look.ui.face(t.text.body_strong.weight);
                    let base = r.y + f.baseline_in(s.size, r.h);
                    f.draw_ellipsized(c, r.x + 8, base, cwd, s.size, col.accent_ink, w.saturating_sub(16));
                }
            }
            _ => {}
        }
        row += 1;
    }
}

/// Nad textem: pruh bloku, cas vpravo, linka zahlavi tabulky, stitky klaves.
pub fn over(c: &mut Canvas<'_>, look: &mut Look, lines: &[Line], g: &Grid) {
    let t = look.t.clone();
    let col = &t.color;
    for (row, l) in lines.iter().enumerate() {
        let y = g.row_y(row);
        match &l.role {
            Role::Head { mark, meta } => {
                c.fill(Rect::new(g.x, y, 3, g.ch), mark_color(look, *mark));
                // Cas a vysledek vpravo pismem UI (neuspech barvou chyby).
                let text: String = (*meta..l.parts_len()).map(|i| l.part_text(i)).collect::<String>();
                let text = text.trim();
                if !text.is_empty() && term::activity::drawn(text.chars().next().unwrap_or(' ')).is_none() {
                    let s = t.text.caption;
                    let f = look.ui.face(s.weight);
                    let tw = f.width(text, s.size);
                    let color = if *mark == Mark::Err { col.danger } else { col.term_dim };
                    let base = y + f.baseline_in(s.size, g.ch);
                    f.draw(c, g.right() - tw as i32, base, text, s.size, color);
                } else if !text.is_empty() {
                    // Ukazatel cinnosti: kresli ho mrizka (posledni bunka).
                    let ch = text.chars().next().unwrap();
                    if let Some(d) = term::activity::drawn(ch) {
                        crate::draw_activity(c, Rect::new(g.right() - g.cw as i32, y, g.cw, g.ch), d, col.accent);
                    }
                }
            }
            Role::Body { mark, table_head, .. } => {
                c.fill(Rect::new(g.x, y, 3, g.ch), mark_color(look, *mark));
                if *table_head {
                    let w = (l.width().saturating_sub(2) as u32 * g.cw).min((g.right() - g.col_x(2)).max(0) as u32);
                    c.fill(Rect::new(g.col_x(2), y + g.ch as i32 - 2, w, 1), col.border);
                }
            }
            Role::Hints(chips) => {
                let s = t.text.caption;
                let mut x = g.x;
                let h = g.ch.saturating_sub(6);
                for (k, (key, label)) in chips.iter().enumerate() {
                    let fk = look.ui.face(t.text.body_strong.weight);
                    let kw = fk.width(key, s.size) + 10;
                    let fl = look.ui.face(s.weight);
                    let lw = if label.is_empty() { 0 } else { fl.width(label, s.size) + 4 };
                    if x + (kw + lw) as i32 > g.right() {
                        let fl = look.ui.face(s.weight);
                        let base = y + fl.baseline_in(s.size, g.ch);
                        if k > 0 {
                            fl.draw(c, x, base, "\u{2026}", s.size, col.term_dim);
                        }
                        break;
                    }
                    let r = Rect::new(x, y + 3, kw, h);
                    c.round_fill_bordered(r, t.radius.xs, col.surface, col.border, 1);
                    let fk = look.ui.face(t.text.body_strong.weight);
                    let base = r.y + fk.baseline_in(s.size, r.h);
                    fk.draw(c, x + 5, base, key, s.size, col.ink);
                    if !label.is_empty() {
                        let fl = look.ui.face(s.weight);
                        let base = y + fl.baseline_in(s.size, g.ch);
                        fl.draw(c, x + kw as i32 + 4, base, label, s.size, col.term_dim);
                    }
                    x += (kw + lw) as i32 + 12;
                }
            }
            _ => {}
        }
    }
}
