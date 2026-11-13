//! Prvky navrhu terminalu (design/terminal.html) nad mrizkou znaku: podle
//! roli radku (`term::ui::draw::Role`) okno dokresli levy pruh bloku, cas
//! vpravo, pilulku slozky, stitky klaves, ramecek chyby a linku pod
//! zahlavim tabulky. Text dal kresli mrizka; bunky, ktere prvek nahrazuje,
//! se preskoci (`skips`).

use alloc::string::String;
use alloc::vec::Vec;

use gfx::{Canvas, Rect};
use term::ui::draw::{Line, Mark, PromptRow, Role};

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
            // Napoveda klaves a volby vyzvy: cely radek kresli okno.
            Role::Hints(_) | Role::Prompt(PromptRow::Buttons { .. }) => alloc::vec![(0, usize::MAX)],
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

/// Ramecek nahledu zmen: nejvys sirka `diffbox` z navrhu (680 px).
const PREVIEW_W: u32 = 680;

fn preview_row(r: &Role) -> bool {
    matches!(
        r,
        Role::Prompt(PromptRow::Question { preview: true } | PromptRow::Change | PromptRow::Buttons { preview: true, .. })
    )
}

/// Tlacitko vyzvy v okne: plocha a index odpovedi.
pub struct Button {
    pub rect: Rect,
    pub index: usize,
}

/// Rozlozeni tlacitek vyzvy (stejne pro kresleni i klik): popisky pismem
/// UI, vyska radku mrizky bez okraje.
pub fn buttons(look: &mut Look, lines: &[Line], g: &Grid) -> Vec<(Button, String, bool, bool)> {
    let t = look.t.clone();
    let mut out = Vec::new();
    for (row, l) in lines.iter().enumerate() {
        let Role::Prompt(PromptRow::Buttons { labels, sel, preview }) = &l.role else { continue };
        let s = t.text.small;
        let f = look.ui.face(t.text.body_strong.weight);
        let mut x = g.x + if *preview { t.space[3] as i32 } else { 0 };
        let y = g.row_y(row) + 1;
        for (i, label) in labels.iter().enumerate() {
            let w = f.width(label, s.size) + 2 * t.space[4];
            if x + w as i32 > g.right() {
                break;
            }
            let primary = *preview && i == 0;
            out.push((Button { rect: Rect::new(x, y, w, g.ch.saturating_sub(2)), index: i }, label.clone(), primary, i == *sel));
            x += (w + t.space[3]) as i32;
        }
    }
    out
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
            r if preview_row(r) => {
                // Nahled zmen: ramecek se zlutym okrajem, zahlavi (otazka)
                // `warn_soft`, pod tlacitky linka jako v navrhu.
                let first = row;
                while row < lines.len() && preview_row(&lines[row].role) {
                    row += 1;
                }
                let x = g.x - t.space[3] as i32;
                let w = PREVIEW_W.min((g.right() + t.space[3] as i32 - x).max(0) as u32);
                let r = Rect::new(x, g.row_y(first), w, (row - first) as u32 * g.ch);
                let rad = t.radius.control;
                c.round_fill_bordered(r, rad, col.term_bg, crate::alpha(col.warn, 128), 1);
                let old = c.clip();
                c.set_clip(old.intersect(&Rect::new(r.x, r.y, r.w, g.ch)));
                c.round_fill(Rect::new(r.x + 1, r.y + 1, r.w.saturating_sub(2), g.ch + rad), rad, col.warn_soft);
                c.set_clip(old);
                if row - first > 1 {
                    c.fill(Rect::new(r.x + 1, g.row_y(row - 1) - 1, r.w.saturating_sub(2), 1), col.border);
                }
                continue;
            }
            Role::Input { prefix, cwd } => {
                let y = g.row_y(row);
                c.fill(Rect::new(g.x, y - 3, (g.right() - g.x).max(0) as u32, 1), col.border);
                if let Some(&(_, end)) = lines[row].part_cols().get(prefix.saturating_sub(1)) {
                    let w = (end as u32 * g.cw).saturating_sub(g.cw);
                    let r = Rect::new(g.x, y + 1, w, g.ch - 2);
                    c.round_fill(r, (g.ch - 2) / 2, col.accent_soft);
                    // Ikona slozky (`icon_s`, nejvys vyska pilulky bez
                    // okraje), za ni jmeno slozky (okraj s3, mezera s1).
                    let pad = t.space[3] as i32;
                    let is = t.size.icon_s.min(r.h.saturating_sub(4));
                    look.icons.draw(c, "folder", r.x + pad, r.y + (r.h - is) as i32 / 2, is, t.icon_stroke, col.accent_ink);
                    let tx = r.x + pad + is as i32 + t.space[1] as i32;
                    let s = t.text.small;
                    let f = look.ui.face(t.text.body_strong.weight);
                    let base = r.y + f.baseline_in(s.size, r.h);
                    let room = (r.x + r.w as i32 - pad - tx).max(0) as u32;
                    f.draw_ellipsized(c, tx, base, cwd, s.size, col.accent_ink, room);
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
    // Volby vyzvy jako tlacitka (hlavni = akcent), vybrana klavesnici
    // s prstencem fokusu.
    for (b, label, primary, sel) in buttons(look, lines, g) {
        let r = b.rect;
        let rad = t.radius.control;
        let ink = if primary {
            c.round_fill(r, rad, col.accent);
            col.on_accent
        } else {
            c.round_fill_bordered(r, rad, col.surface, col.border_strong, 1);
            col.ink
        };
        if sel {
            let fr = t.size.focus_ring;
            let o = (t.size.focus_offset + fr) as i32;
            c.round_stroke(Rect::new(r.x - o, r.y - o, r.w + 2 * o as u32, r.h + 2 * o as u32), rad + o as u32, fr, col.focus);
        }
        let s = t.text.small;
        let f = look.ui.face(t.text.body_strong.weight);
        let base = r.y + f.baseline_in(s.size, r.h);
        f.draw(c, r.x + t.space[4] as i32, base, &label, s.size, ink);
    }
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
