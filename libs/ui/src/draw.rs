//! Kresleni prvku do bufferu (jeden prostor: okno nebo posouvany obsah).
//!
//! Vzhled podle `design/design-system.html`: tlacitka jako pilulky
//! s ramem `border_strong`, pole a seznamy se zaoblenim `radius.control`,
//! fokus jako ramecek `size.focus_ring` s odsazenim `size.focus_offset`
//! barvou `focus`. Vse z tokenu ([`crate::look::Look`]).

use gfx::{Canvas, Rect};
use theme::TextStyle;

use crate::Ui;
use crate::layout::ElemKind;

/// Ramecek fokusu kolem `r` (vne prvku, zaobleni o odsazeni vetsi).
fn focus_ring(c: &mut Canvas<'_>, r: Rect, radius: u32, ring: u32, offset: u32, color: u32) {
    let e = ring + offset;
    let o = Rect::new(r.x - e as i32, r.y - e as i32, r.w + 2 * e, r.h + 2 * e);
    c.round_stroke(o, radius + e, ring, color);
}

impl<M: Clone> Ui<M> {
    /// Nakresli prvky prostoru `space` (0 = okno, jinak index oblasti + 1)
    /// do platna; pocatek platna = pocatek prostoru.
    pub fn draw(&mut self, c: &mut Canvas<'_>, space: usize) {
        let th = self.look.theme.clone();
        let col = &th.color;
        let sp = &th.space;
        let (ring, ring_off) = (th.size.focus_ring, th.size.focus_offset);
        c.fill(c.bounds(), col.surface);
        for i in 0..self.elems.len() {
            let e = &self.elems[i];
            if e.space != space {
                continue;
            }
            let r = e.rect;
            let focused = self.focus == Some(i);
            let hover = self.hover == Some(i);
            let pressed = self.pressed == Some(i);
            let dim = e.disabled;
            match &e.kind {
                ElemKind::Label { text, heading, warn } => {
                    let s = if *heading { self.look.heading() } else { self.look.body() };
                    let old = c.clip();
                    c.set_clip(old.intersect(&r));
                    if *warn {
                        let tw = self.look.width(text, s) + 2 * sp[4];
                        let chip = Rect::new(r.x, r.y, tw.min(r.w), r.h);
                        c.round_fill(chip, chip.h / 2, col.warn_soft);
                        let f = self.look.face(s);
                        let b = chip.y + f.baseline_in(s.size, chip.h);
                        f.draw(c, chip.x + sp[4] as i32, b, text, s.size, col.warn);
                    } else {
                        let f = self.look.face(s);
                        let b = r.y + f.baseline_in(s.size, r.h);
                        f.draw(c, r.x, b, text, s.size, col.ink);
                    }
                    c.set_clip(old);
                }
                ElemKind::Button { text, .. } => {
                    let bg = if dim {
                        col.surface_2
                    } else if pressed || hover {
                        col.surface_3
                    } else {
                        col.surface
                    };
                    let border = if dim { col.border } else { col.border_strong };
                    c.round_fill_bordered(r, r.h / 2, bg, border, 1);
                    if focused && !dim {
                        focus_ring(c, r, r.h / 2, ring, ring_off, col.focus);
                    }
                    let s = self.look.button();
                    let f = self.look.face(s);
                    let tw = f.width(text, s.size) as i32;
                    let fg = if dim { col.ink_faint } else { col.ink };
                    let b = r.y + f.baseline_in(s.size, r.h);
                    f.draw(c, r.x + (r.w as i32 - tw) / 2, b, text, s.size, fg);
                }
                ElemKind::TextInput { value, placeholder, .. } => {
                    let rad = th.radius.control;
                    c.round_fill_bordered(r, rad, col.surface, if focused { col.accent } else { col.border_strong }, 1);
                    if focused {
                        focus_ring(c, r, rad, ring, ring_off, col.focus);
                    }
                    let s = self.look.body();
                    let inset = sp[4] as i32;
                    let old = c.clip();
                    c.set_clip(old.intersect(&Rect::new(r.x + inset / 2, r.y, r.w.saturating_sub(sp[4]), r.h)));
                    let (txt, fg) = if value.is_empty() { (placeholder.as_str(), col.ink_muted) } else { (value.as_str(), col.ink) };
                    let f = self.look.face(s);
                    let b = r.y + f.baseline_in(s.size, r.h);
                    f.draw(c, r.x + inset, b, txt, s.size, fg);
                    if focused {
                        let pos = self.look.face_ref(s).positions(value, s.size);
                        let ci = self.cursor.get(&i).copied().unwrap_or(0).min(pos.len() - 1);
                        let cx = r.x + inset + pos[ci];
                        let ch = s.line;
                        c.fill(Rect::new(cx, r.y + (r.h as i32 - ch as i32) / 2, 2, ch), col.accent);
                    }
                    c.set_clip(old);
                }
                ElemKind::Checkbox { text, checked, .. } => {
                    let bs = th.size.icon_m;
                    let b = Rect::new(r.x, r.y + (r.h as i32 - bs as i32) / 2, bs, bs);
                    if *checked {
                        c.round_fill(b, th.radius.xs, col.accent);
                        let is = th.size.icon_s;
                        self.icons.draw(c, "check", b.x + (bs - is) as i32 / 2, b.y + (bs - is) as i32 / 2, is, th.icon_stroke, col.on_accent);
                    } else {
                        c.round_fill_bordered(b, th.radius.xs, col.surface, col.border_strong, 1);
                    }
                    if focused {
                        focus_ring(c, b, th.radius.xs, ring, ring_off, col.focus);
                    }
                    let s = self.look.body();
                    let f = self.look.face(s);
                    let base = r.y + f.baseline_in(s.size, r.h);
                    f.draw(c, r.x + (bs + sp[3]) as i32, base, text, s.size, col.ink);
                }
                ElemKind::List { .. } => {
                    c.round_fill_bordered(r, th.radius.card, col.surface, col.border, 1);
                    if focused {
                        focus_ring(c, r, th.radius.card, ring, ring_off, col.focus);
                    }
                }
                ElemKind::ListItem { text, selected, .. } => {
                    let rad = th.radius.control;
                    let fg = if *selected {
                        c.round_fill(r, rad, col.accent_soft);
                        col.accent_ink
                    } else {
                        if hover {
                            c.round_fill(r, rad, col.surface_3);
                        }
                        col.ink
                    };
                    let s: TextStyle = if *selected { th.text.body_strong } else { self.look.body() };
                    let old = c.clip();
                    c.set_clip(old.intersect(&r));
                    let f = self.look.face(s);
                    let b = r.y + f.baseline_in(s.size, r.h);
                    f.draw_ellipsized(c, r.x + sp[4] as i32, b, text, s.size, fg, r.w.saturating_sub(2 * sp[4]));
                    c.set_clip(old);
                }
                ElemKind::Progress { value } => {
                    c.round_fill(r, r.h / 2, col.surface_3);
                    let w = r.w * (*value).min(100) / 100;
                    if w > 0 {
                        c.round_fill(Rect::new(r.x, r.y, w.max(r.h), r.h), r.h / 2, col.accent);
                    }
                }
                ElemKind::Group => {}
                ElemKind::Scroll { .. } => {
                    // Obsah je vlastni vrstva; tady jen ram.
                    c.round_stroke(r, th.radius.control, 1, col.border);
                }
            }
        }
    }

}
