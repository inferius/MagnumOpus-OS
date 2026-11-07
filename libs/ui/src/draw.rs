//! Kresleni prvku do bufferu (jeden prostor: okno nebo posouvany obsah).

use gfx::{Canvas, Rect};

use crate::layout::ElemKind;
use crate::{Ui, theme};

impl<M: Clone> Ui<M> {
    /// Nakresli prvky prostoru `space` (0 = okno, jinak index oblasti + 1)
    /// do platna; pocatek platna = pocatek prostoru.
    pub fn draw(&mut self, c: &mut Canvas<'_>, space: usize) {
        c.fill(c.bounds(), theme::BG);
        let (lh, base) = self.text.line(theme::PX);
        for i in 0..self.elems.len() {
            let e = &self.elems[i];
            if e.space != space {
                continue;
            }
            let r = e.rect;
            let focused = self.focus == Some(i);
            let hover = self.hover == Some(i);
            let pressed = self.pressed == Some(i);
            let ty = |h: u32| r.y + (h as i32 - lh as i32) / 2 + base;
            let dim = e.disabled;
            match &e.kind {
                ElemKind::Label { text, heading } => {
                    let px = if *heading { theme::HEADING_PX } else { theme::PX };
                    let (hl, hb) = self.text.line(px);
                    let y = r.y + (r.h as i32 - hl as i32) / 2 + hb;
                    let old = c.clip();
                    c.set_clip(old.intersect(&r));
                    self.text.draw(c, r.x, y, text, px, theme::TEXT);
                    c.set_clip(old);
                }
                ElemKind::Button { text, .. } => {
                    let col = if dim || pressed {
                        theme::PRESSED
                    } else if hover {
                        theme::HOVER
                    } else {
                        theme::BUTTON
                    };
                    c.rounded(r, 6, col);
                    if focused {
                        c.outline(r, 2, theme::ACCENT);
                    }
                    let tw = self.text.width(text, theme::PX) as i32;
                    let fg = if dim { theme::DIM } else { theme::TEXT };
                    self.text.draw(c, r.x + (r.w as i32 - tw) / 2, ty(r.h), text, theme::PX, fg);
                }
                ElemKind::TextInput { value, placeholder, .. } => {
                    c.rounded(r, 6, theme::INPUT);
                    c.outline(r, 1, if focused { theme::ACCENT } else { theme::BORDER });
                    let old = c.clip();
                    c.set_clip(old.intersect(&Rect::new(r.x + 6, r.y, r.w.saturating_sub(12), r.h)));
                    let (s, fg) = if value.is_empty() { (placeholder.as_str(), theme::DIM) } else { (value.as_str(), theme::TEXT) };
                    self.text.draw(c, r.x + 10, ty(r.h), s, theme::PX, fg);
                    if focused {
                        let pos = self.text.positions(value, theme::PX);
                        let ci = self.cursor.get(&i).copied().unwrap_or(0).min(pos.len() - 1);
                        let cx = r.x + 10 + pos[ci];
                        c.fill(Rect::new(cx, r.y + 6, 2, r.h.saturating_sub(12)), theme::ACCENT);
                    }
                    c.set_clip(old);
                }
                ElemKind::Checkbox { text, checked, .. } => {
                    let b = Rect::new(r.x, r.y + (r.h as i32 - 18) / 2, 18, 18);
                    c.rounded(b, 4, if *checked { theme::ACCENT } else { theme::INPUT });
                    c.outline(b, 1, if focused { theme::ACCENT } else { theme::BORDER });
                    if *checked {
                        // Fajfka z kratkych bloku.
                        for k in 0..4 {
                            c.fill(Rect::new(b.x + 4 + k, b.y + 8 + k, 2, 2), theme::TEXT);
                        }
                        for k in 0..7 {
                            c.fill(Rect::new(b.x + 7 + k, b.y + 11 - k, 2, 2), theme::TEXT);
                        }
                    }
                    self.text.draw(c, r.x + 26, ty(r.h), text, theme::PX, theme::TEXT);
                }
                ElemKind::List { .. } => {
                    c.rounded(r, 6, theme::INPUT);
                    if focused {
                        c.outline(r, 1, theme::ACCENT);
                    }
                }
                ElemKind::ListItem { text, selected, .. } => {
                    if *selected {
                        c.fill(Rect::new(r.x + 2, r.y + 1, r.w.saturating_sub(4), r.h - 2), theme::SELECTED);
                    } else if hover {
                        c.fill(Rect::new(r.x + 2, r.y + 1, r.w.saturating_sub(4), r.h - 2), theme::BUTTON);
                    }
                    let old = c.clip();
                    c.set_clip(old.intersect(&r));
                    self.text.draw(c, r.x + 10, ty(r.h), text, theme::PX, theme::TEXT);
                    c.set_clip(old);
                }
                ElemKind::Progress { value } => {
                    c.rounded(r, 5, theme::INPUT);
                    let w = r.w * *value / 100;
                    if w > 0 {
                        c.rounded(Rect::new(r.x, r.y, w, r.h), 5, theme::ACCENT);
                    }
                }
                ElemKind::Group => {}
                ElemKind::Scroll { .. } => {
                    // Obsah je vlastni vrstva; tady jen ram.
                    c.outline(r, 1, theme::BORDER);
                }
            }
        }
    }
}
