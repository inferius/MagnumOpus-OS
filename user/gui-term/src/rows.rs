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

/// Mrizka v okne: radky maji vlastni vysku podle role (okraje nad a pod
/// textem jako v navrhu: hlavicka bloku 28 px, ramecek nahledu, paticka),
/// text radku je vzdy pasmo vysky `ch`. Radky se skladaji od spodniho
/// okraje nahoru (radek vstupu sedi dole); stavovy radek nahore je
/// pripnuty a co se nevejde, schova se pod nim (`clip_top`).
#[derive(Default)]
pub struct Grid {
    pub x: i32,
    pub cw: u32,
    pub ch: u32,
    pub cols: usize,
    /// Horni hrana textu radku.
    tops: Vec<i32>,
    /// Okraj nad a pod textem radku (px).
    pads: Vec<(u32, u32)>,
    /// Odsazeni textu radku vpravo (px; text v ramecku nahledu, chyby).
    indents: Vec<u32>,
    /// Nad touto hranou se radky obsahu nekresli (pod stavovym radkem).
    pub clip_top: i32,
    /// Spodni hrana posledniho radku.
    pub bottom: i32,
}

/// Rozestupy navrhu pro role radku (tokeny `space`).
pub struct Spacing {
    pub sp: [u32; 12],
    /// Vyska tlacitek vyzvy a pilulky slozky (`control_h_compact`).
    pub control: u32,
    /// Vyska radku vstupu (`#line`: `control_h`).
    pub line: u32,
}

fn is_err(l: Option<&Line>) -> bool {
    matches!(l.map(|l| &l.role), Some(Role::Body { error: true, .. }))
}

fn is_preview(l: Option<&Line>) -> bool {
    l.is_some_and(|l| preview_row(&l.role))
}

/// Okraj nad a pod textem radku `k` a odsazeni textu (px).
fn pads(lines: &[Line], k: usize, (cw, ch): (u32, u32), s: &Spacing) -> (u32, u32, u32) {
    let sp = &s.sp;
    let prev = k.checked_sub(1).and_then(|p| lines.get(p));
    let next = lines.get(k + 1);
    // Ramecek: odsazeni od pruhu bloku (3 px + s4) a vnitrni okraj s4.
    let boxed = 3 + sp[4] + sp[4];
    let around = |center: u32| center.saturating_sub(ch) / 2;
    match lines.get(k).map(|l| &l.role) {
        // `.blk-head`: min-height 28 px.
        Some(Role::Head { .. }) => (around(s.control), around(s.control), 0),
        // `.errbox`: okraj s3 nad prvnim a pod poslednim radkem.
        Some(Role::Body { error: true, .. }) => {
            let pre = if is_err(prev) { 0 } else { sp[3] };
            let post = if is_err(next) { 0 } else { sp[3] };
            (pre, post, sp[4])
        }
        Some(Role::Prompt(PromptRow::Question { preview: true })) => (sp[3], sp[3], boxed),
        Some(Role::Prompt(PromptRow::Change)) => {
            let pre = if matches!(prev.map(|l| &l.role), Some(Role::Prompt(PromptRow::Change))) { 0 } else { sp[2] };
            let post = if matches!(next.map(|l| &l.role), Some(Role::Prompt(PromptRow::Change))) { 0 } else { sp[2] };
            // Radek zmeny zacina dvema mezerami (konzole): text zarovnat
            // se zahlavim ramecku.
            (pre, post, boxed.saturating_sub(2 * cw))
        }
        // `.diffbox footer`: okraj s3, tlacitka `.btn.small`.
        Some(Role::Prompt(PromptRow::Buttons { preview: true, .. })) => {
            let m = sp[3] + around(s.control);
            (m, m, boxed)
        }
        Some(Role::Prompt(PromptRow::Buttons { .. })) => (sp[2] + around(s.control), sp[2] + around(s.control), 0),
        Some(Role::Prompt(PromptRow::Question { .. })) if !is_preview(prev) => (sp[2], 0, 0),
        // `.input-wrap`: linka, okraj s3, radek vstupu vysky `control_h`.
        Some(Role::Input { .. }) => (sp[3] + around(s.line), around(s.line), 0),
        // `.hintbar`: margin-top s2.
        Some(Role::Hints(_)) => (sp[2], 0, 0),
        _ => (0, 0, 0),
    }
}

impl Grid {
    /// Rozlozeni `rows` radku mrizky mezi `top` a `bottom` (od spodu).
    pub fn layout(x: i32, (cw, ch): (u32, u32), cols: usize, rows: usize, (top, bottom): (i32, i32), lines: &[Line], s: &Spacing) -> Self {
        let mut g = Grid { x, cw, ch, cols, tops: alloc::vec![0; rows], pads: alloc::vec![(0, 0); rows], indents: alloc::vec![0; rows], clip_top: top, bottom };
        for k in 0..rows {
            let (pre, post, ind) = pads(lines, k, (cw, ch), s);
            g.pads[k] = (pre, post);
            g.indents[k] = ind;
        }
        let pinned = matches!(lines.first().map(|l| &l.role), Some(Role::Status));
        let mut y = bottom;
        let first = if pinned { 1 } else { 0 };
        for k in (first..rows).rev() {
            let (pre, post) = g.pads[k];
            y -= (post + ch) as i32;
            g.tops[k] = y;
            y -= pre as i32;
        }
        if pinned && rows > 0 {
            let (pre, post) = g.pads[0];
            g.tops[0] = top + pre as i32;
            g.clip_top = g.tops[0] + (ch + post) as i32;
        }
        // Radek, ktery se nevejde cely, se schova cely (ne pulka textu).
        if let Some(k) = (first..rows).find(|&k| g.span(k).0 >= g.clip_top) {
            g.clip_top = g.span(k).0;
        }
        g
    }

    /// Horni hrana textu radku (za poslednim radkem dalsi pod nim).
    pub fn row_y(&self, row: usize) -> i32 {
        match self.tops.get(row) {
            Some(&y) => y,
            None => self.bottom + (row.saturating_sub(self.tops.len()) as u32 * self.ch) as i32,
        }
    }

    /// Cely radek i s okraji: horni a spodni hrana.
    pub fn span(&self, row: usize) -> (i32, i32) {
        let (pre, post) = self.pads.get(row).copied().unwrap_or((0, 0));
        let y = self.row_y(row);
        (y - pre as i32, y + (self.ch + post) as i32)
    }

    /// Je radek videt (neni schovany pod stavovym radkem)?
    pub fn visible(&self, row: usize) -> bool {
        row == 0 && self.clip_top > self.row_y(0) || self.row_y(row) >= self.clip_top
    }

    /// Radek pod bodem `y` (i v okrajich radku).
    pub fn row_at(&self, y: i32) -> Option<usize> {
        (0..self.tops.len()).find(|&k| {
            let (a, b) = self.span(k);
            self.visible(k) && y >= a && y < b
        })
    }

    /// Levy okraj textu radku.
    pub fn text_x(&self, row: usize) -> i32 {
        self.x + self.indents.get(row).copied().unwrap_or(0) as i32
    }

    /// Bunka `col` radku `row`.
    pub fn cell_x(&self, row: usize, col: usize) -> i32 {
        self.text_x(row) + (col as u32 * self.cw) as i32
    }

    fn col_x(&self, col: usize) -> i32 {
        self.x + (col as u32 * self.cw) as i32
    }

    pub fn right(&self) -> i32 {
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

/// Rozlozeni tlacitek vyzvy (stejne pro kresleni i klik): `.btn.small`
/// z navrhu (vyska `control_h_compact`, okraj s4, pismo `small`), hlavni
/// tlacitko nahledu s ikonou potvrzeni.
pub fn buttons(look: &mut Look, lines: &[Line], g: &Grid) -> Vec<(Button, String, bool, bool)> {
    let t = look.t.clone();
    let mut out = Vec::new();
    let h = t.size.control_h_compact;
    for (row, l) in lines.iter().enumerate() {
        let Role::Prompt(PromptRow::Buttons { labels, sel, preview }) = &l.role else { continue };
        if !g.visible(row) {
            continue;
        }
        let s = t.text.small;
        let f = look.ui.face(t.text.body_strong.weight);
        let mut x = g.text_x(row);
        let y = g.row_y(row) + (g.ch as i32 - h as i32) / 2;
        for (i, label) in labels.iter().enumerate() {
            let primary = *preview && i == 0;
            let icon = if primary { t.size.icon_s + t.space[2] } else { 0 };
            let w = f.width(label, s.size) + 2 * t.space[4] + icon;
            if x + w as i32 > g.right() {
                break;
            }
            out.push((Button { rect: Rect::new(x, y, w, h), index: i }, label.clone(), primary, i == *sel));
            x += (w + t.space[3]) as i32;
        }
    }
    out
}

/// Pod textem: ramecky chyb a nahledu, pilulka slozky, linka paticky.
pub fn under(c: &mut Canvas<'_>, look: &mut Look, lines: &[Line], g: &Grid) {
    let t = look.t.clone();
    let col = &t.color;
    let sp = &t.space;
    // Obsah bloku zacina za pruhem (3 px) a okrajem s4 (`.blk`).
    let content = g.x + 3 + sp[4] as i32;
    let mut row = 0;
    while row < lines.len() {
        match &lines[row].role {
            Role::Body { error: true, .. } => {
                // Souvisle radky hlaseni chyby = jeden ramecek (`.errbox`:
                // okraj s3 s4, text od sloupce 2 radku).
                let first = row;
                let mut width = 0;
                while row < lines.len() && matches!(lines[row].role, Role::Body { error: true, .. }) {
                    width = width.max(lines[row].width());
                    row += 1;
                }
                let x = content.max(g.cell_x(first, 2) - sp[4] as i32);
                let text = g.cell_x(first, width.max(2)) - x + sp[4] as i32;
                let w = (text.max(0) as u32).min(PREVIEW_W).min((g.right() + sp[4] as i32 - x).max(0) as u32);
                let (top, bot) = (g.span(first).0, g.span(row - 1).1);
                let r = Rect::new(x, top, w, (bot - top).max(0) as u32);
                c.round_fill_bordered(r, t.radius.control, col.danger_soft, crate::alpha(col.danger, 115), 1);
                continue;
            }
            r if preview_row(r) => {
                // Nahled zmen (`.blk.preview` + `.diffbox`): pruh `warn`
                // vlevo, ramecek se zlutym okrajem, zahlavi (otazka)
                // `warn_soft`, nad tlacitky linka.
                let first = row;
                while row < lines.len() && preview_row(&lines[row].role) {
                    row += 1;
                }
                let last = row - 1;
                let (top, bot) = (g.span(first).0, g.span(last).1);
                c.fill(Rect::new(g.x, top, 3, (bot - top).max(0) as u32), col.warn);
                let x = content;
                let w = PREVIEW_W.min((g.right() + sp[4] as i32 - x).max(0) as u32);
                let r = Rect::new(x, top, w, (bot - top).max(0) as u32);
                let rad = t.radius.control;
                c.round_fill_bordered(r, rad, col.term_bg, crate::alpha(col.warn, 128), 1);
                if matches!(lines[first].role, Role::Prompt(PromptRow::Question { .. })) {
                    let hb = g.span(first).1;
                    let old = c.clip();
                    c.set_clip(old.intersect(&Rect::new(r.x, r.y, r.w, (hb - r.y).max(0) as u32)));
                    c.round_fill(Rect::new(r.x + 1, r.y + 1, r.w.saturating_sub(2), (hb - r.y) as u32 + rad), rad, col.warn_soft);
                    c.set_clip(old);
                }
                if last > first && matches!(lines[last].role, Role::Prompt(PromptRow::Buttons { .. })) {
                    c.fill(Rect::new(r.x + 1, g.span(last).0, r.w.saturating_sub(2), 1), col.border);
                }
                continue;
            }
            Role::Input { prefix, cwd } => {
                let y = g.row_y(row);
                // Paticka (`.input-wrap`): linka pres celou sirku mrizky
                // i s okraji.
                c.fill(Rect::new(0, g.span(row).0, (g.right() + g.x).max(0) as u32, 1), col.border);
                if let Some(&(_, end)) = lines[row].part_cols().get(prefix.saturating_sub(1)) {
                    // Pilulka `.cwd`: vyska `control_h_compact`, okraj s3,
                    // ikona, mezera s1, jmeno, okraj s3; sirka podle textu,
                    // nejvys k mezere s3 pred textem radku (bunka `end`).
                    let pad = sp[3];
                    let s = t.text.small;
                    let f = look.ui.face(t.text.body_strong.weight);
                    let h = t.size.control_h_compact;
                    let is = t.size.icon_s;
                    let max = (end as u32 * g.cw).saturating_sub(sp[3]);
                    let w = (2 * pad + is + sp[1] + f.width(cwd, s.size)).min(max);
                    let r = Rect::new(g.x, y + (g.ch as i32 - h as i32) / 2, w, h);
                    c.round_fill(r, h / 2, col.accent_soft);
                    look.icons.draw(c, "folder", r.x + pad as i32, r.y + (r.h - is) as i32 / 2, is, t.icon_stroke, col.accent_ink);
                    let tx = r.x + (pad + is + sp[1]) as i32;
                    let base = r.y + f.baseline_in(s.size, r.h);
                    let room = (r.x + r.w as i32 - pad as i32 - tx).max(0) as u32;
                    f.draw_ellipsized(c, tx, base, cwd, s.size, col.accent_ink, room);
                }
            }
            _ => {}
        }
        row += 1;
    }
}

/// Klavesy jedne napovedy: kombinace s modifikatorem ("Ctrl-C",
/// "Shift+PgUp") jako samostatne klavesy, jinak cela.
fn keys_of(key: &str) -> Vec<&str> {
    let parts: Vec<&str> = key.split(['+', '-']).collect();
    let modifier = |p: &str| matches!(p, "Ctrl" | "Shift" | "Alt" | "Super");
    if parts.len() > 1 && parts[..parts.len() - 1].iter().all(|p| modifier(p)) && !parts[parts.len() - 1].is_empty() {
        parts
    } else {
        alloc::vec![key]
    }
}

/// Napoveda klaves v paticce (`.hintbar` a `.kbd` z navrhu): kazda
/// klavesa stitek (plocha `surface`, okraj `border_strong`, spodni hrana
/// silnejsi, pismo `caption`), za klavesami popis `term_dim`, mezi
/// polozkami mezera `s4`. Co se nevejde, nahradi "...".
fn paint_hints(c: &mut Canvas<'_>, look: &mut Look, chips: &[(String, String)], g: &Grid, y: i32) {
    let t = look.t.clone();
    let col = &t.color;
    let s = t.text.caption;
    let sp = &t.space;
    // Vyska stitku: radek `caption` s okrajem s2, nejvys radek mrizky.
    let h = (s.line + sp[2]).min(g.ch);
    let ky = y + (g.ch - h) as i32 / 2;
    let kpad = sp[2] + sp[1];
    let kmin = t.size.icon_s + 2 * sp[1];
    let mut x = g.x;
    for (k, (key, label)) in chips.iter().enumerate() {
        let keys = keys_of(key);
        let fk = look.ui.face(s.weight);
        let widths: Vec<u32> = keys.iter().map(|q| (fk.width(q, s.size) + 2 * kpad).max(kmin)).collect();
        let kw: u32 = widths.iter().sum::<u32>() + sp[1] * (keys.len() as u32 - 1);
        let fl = look.ui.face(s.weight);
        let lw = if label.is_empty() { 0 } else { sp[2] + fl.width(label, s.size) };
        if x + (kw + lw) as i32 > g.right() {
            if k > 0 {
                let base = y + fl.baseline_in(s.size, g.ch);
                fl.draw(c, x, base, "\u{2026}", s.size, col.term_dim);
            }
            break;
        }
        for (q, w) in keys.iter().zip(&widths) {
            let r = Rect::new(x, ky, *w, h);
            // Spodni hrana 2 px: plocha okraje, nad ni klavesa o 1 px nizsi.
            c.round_fill(r, t.radius.xs, col.border_strong);
            c.round_fill_bordered(Rect::new(r.x, r.y, r.w, r.h - 1), t.radius.xs, col.surface, col.border_strong, 1);
            let fk = look.ui.face(s.weight);
            let base = r.y + fk.baseline_in(s.size, r.h - 1);
            let tw = fk.width(q, s.size);
            fk.draw(c, r.x + (r.w - tw) as i32 / 2, base, q, s.size, col.ink);
            x += (*w + sp[1]) as i32;
        }
        x -= sp[1] as i32;
        if !label.is_empty() {
            let fl = look.ui.face(s.weight);
            let base = y + fl.baseline_in(s.size, g.ch);
            fl.draw(c, x + sp[2] as i32, base, label, s.size, col.term_dim);
        }
        x += (lw + sp[4]) as i32;
    }
}

/// Nad textem: pruh bloku, cas vpravo, linka zahlavi tabulky, stitky klaves.
pub fn over(c: &mut Canvas<'_>, look: &mut Look, lines: &[Line], g: &Grid, hover: Option<u64>) {
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
        let mut x = r.x + t.space[4] as i32;
        if primary {
            let is = t.size.icon_s;
            look.icons.draw(c, "check", x, r.y + (r.h - is) as i32 / 2, is, t.icon_stroke, ink);
            x += (is + t.space[2]) as i32;
        }
        let f = look.ui.face(t.text.body_strong.weight);
        let base = r.y + f.baseline_in(s.size, r.h);
        f.draw(c, x, base, &label, s.size, ink);
    }
    for (row, l) in lines.iter().enumerate() {
        let y = g.row_y(row);
        match &l.role {
            Role::Head { mark, meta, block } => {
                let (a, b) = g.span(row);
                c.fill(Rect::new(g.x, a, 3, (b - a) as u32), mark_color(look, *mark));
                // Blok pod ukazatelem ma vpravo akce: cas se posune vlevo.
                let right = g.right() - if hover == Some(*block) { (crate::blockact::width(look, g) + t.space[2]) as i32 } else { 0 };
                // Cas a vysledek vpravo pismem UI (neuspech barvou chyby).
                let text: String = (*meta..l.parts_len()).map(|i| l.part_text(i)).collect::<String>();
                let text = text.trim();
                if !text.is_empty() && term::activity::drawn(text.chars().next().unwrap_or(' ')).is_none() {
                    let s = t.text.caption;
                    let f = look.ui.face(s.weight);
                    let tw = f.width(text, s.size);
                    let color = if *mark == Mark::Err { col.danger } else { col.term_dim };
                    let base = y + f.baseline_in(s.size, g.ch);
                    f.draw(c, right - tw as i32, base, text, s.size, color);
                } else if !text.is_empty() {
                    // Ukazatel cinnosti: kresli ho mrizka (posledni bunka).
                    let ch = text.chars().next().unwrap();
                    if let Some(d) = term::activity::drawn(ch) {
                        crate::draw_activity(c, Rect::new(g.right() - g.cw as i32, y, g.cw, g.ch), d, col.accent);
                    }
                }
            }
            Role::Body { mark, table_head, .. } => {
                let (a, b) = g.span(row);
                c.fill(Rect::new(g.x, a, 3, (b - a) as u32), mark_color(look, *mark));
                if *table_head {
                    let w = (l.width().saturating_sub(2) as u32 * g.cw).min((g.right() - g.col_x(2)).max(0) as u32);
                    c.fill(Rect::new(g.col_x(2), y + g.ch as i32 - 2, w, 1), col.border);
                }
            }
            Role::Hints(chips) => paint_hints(c, look, chips, g, y),
            _ => {}
        }
    }
}
