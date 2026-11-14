//! Kresleni textoveho pole a jeho tlacitek (oko hesla, krizek hledani,
//! krokovani cisla); varovani Caps Lock. Geometrie a klavesy: `field.rs`.

use alloc::string::String;
use alloc::vec::Vec;

use gfx::{Canvas, Rect};
use inputproto::mods;

use crate::event::Response;
use crate::field::FieldBtn;
use crate::fieldopts::FieldKind;
use crate::layout::ElemKind;
use crate::{State, Ui};

impl<M: Clone> Ui<M> {
    /// Tlacitka uvnitr pole (souradnice prvku): oko hesla, krizek hledani,
    /// krokovani cisla (horni a dolni pulka). Mimo poradi Tab.
    pub(crate) fn field_buttons(&self, i: usize) -> Vec<(FieldBtn, Rect)> {
        let mut out = Vec::new();
        let Some(o) = self.field_opts(i) else { return out };
        if self.elems[i].disabled {
            return out;
        }
        let b = self.field_box(i);
        let bw = self.field_button_w();
        let mut right = b.right() as i32 - self.look.theme.space[1] as i32;
        let mut take = |btn: FieldBtn, out: &mut Vec<(FieldBtn, Rect)>| {
            right -= bw as i32;
            out.push((btn, Rect::new(right, b.y, bw, b.h)));
        };
        if self.field_kind(i).is_some_and(|k| k.1) {
            take(FieldBtn::Eye, &mut out);
        }
        let editable = self.field_editable(i);
        match &o.kind {
            FieldKind::Search if editable && !self.field_text(i).is_empty() => take(FieldBtn::Clear, &mut out),
            FieldKind::Number(_) if editable => {
                take(FieldBtn::Up, &mut out);
                // Krokovani: horni pulka nahoru, dolni dolu.
                let r = out.last().unwrap().1;
                let h = r.h / 2;
                out.last_mut().unwrap().1 = Rect::new(r.x, r.y, r.w, h);
                out.push((FieldBtn::Down, Rect::new(r.x, r.y + h as i32, r.w, r.h - h)));
            }
            _ => {}
        }
        out
    }

    /// Tlacitko pole v bode `(x, y)` (souradnice prvku).
    pub(crate) fn field_button_at(&self, i: usize, x: i32, y: i32) -> Option<FieldBtn> {
        self.field_buttons(i).into_iter().find(|(_, r)| r.contains(x, y)).map(|(b, _)| b)
    }

    /// Stisk tlacitka v poli.
    pub(crate) fn field_button(&mut self, i: usize, btn: FieldBtn, r: &mut Response<M>) {
        let key = self.elems[i].key;
        match btn {
            FieldBtn::Eye => {
                if !self.revealed.remove(&key) {
                    self.revealed.insert(key);
                }
                self.field_reveal(i);
            }
            FieldBtn::Clear => self.field_replace(i, "", r),
            FieldBtn::Up => {
                self.number_step(i, 1, r);
            }
            FieldBtn::Down => {
                self.number_step(i, -1, r);
            }
        }
        r.redraw = true;
    }

    /// Varovani Caps Lock pod polem s heslem (s fokusem, bez chyby).
    pub(crate) fn caps_warning(&self, i: usize) -> bool {
        self.focus == Some(i) && self.mods & mods::CAPS_LOCK != 0 && self.elems[i].error.is_none() && self.field_kind(i).is_some_and(|k| k.1)
    }

    /// Nakresli pole `i` (ramecek, vyber, text, kurzor, chyba).
    pub(crate) fn draw_field(&mut self, c: &mut Canvas<'_>, i: usize, st: State) {
        let th = self.look.theme.clone();
        let col = &th.color;
        let Some((multi, _)) = self.field_kind(i) else { return };
        let secure = self.field_masked(i);
        let opts = self.field_opts(i).cloned().unwrap_or_default();
        let b = self.field_box(i);
        let rad = th.radius.control;
        let typing = st.focused && self.active && !st.disabled;
        let edge = if st.error {
            col.danger
        } else if typing {
            col.accent
        } else if st.hover {
            col.ink_muted
        } else {
            col.border_strong
        };
        let k = if st.disabled { th.opacity.disabled } else { 255 };
        let fade = |c: u32| crate::draw::fade(c, col.surface, k);
        // Jen pro cteni: plocha `surface_2` (vypada jako text, ne vstup).
        let bg = if opts.read_only { col.surface_2 } else { col.surface };
        c.round_fill_bordered(b, rad, bg, fade(edge), if st.error { 2 } else { 1 });
        if st.focus_visible {
            crate::draw::focus_ring(c, b, rad, th.size.focus_ring, th.size.focus_offset, col.focus);
        }
        let inner = self.field_inner(i);
        let key = self.elems[i].key;
        let (sx, sy) = self.tscroll.get(&key).copied().unwrap_or((0, 0));
        let ax = self.field_align(i);
        let s = String::from(self.field_text(i));
        let sel = self.edits.get(&key).and_then(|e| e.selection());
        let style = self.look.body();
        let lh = style.line as i32;
        let lines = self.field_lines(i);
        let top = if multi { inner.y } else { b.y + (b.h as i32 - lh) / 2 };
        // Lupa hledani vlevo.
        if opts.kind == FieldKind::Search {
            let is = th.size.icon_s;
            let x = inner.x - (is + th.space[2]) as i32;
            self.icons.draw(c, "search", x, b.y + (b.h as i32 - is as i32) / 2, is, th.icon_stroke, fade(col.ink_muted));
        }
        // Tlacitka v poli a jednotka cisla.
        let ic = th.size.icon_s;
        for (btn, r) in self.field_buttons(i) {
            let name = match btn {
                FieldBtn::Eye if self.revealed.contains(&key) => "eye-off",
                FieldBtn::Eye => "eye",
                FieldBtn::Clear => "close",
                FieldBtn::Up => "up",
                FieldBtn::Down => "down",
            };
            let size = if matches!(btn, FieldBtn::Up | FieldBtn::Down) { ic.min(r.h) } else { ic };
            let hot = self.hover == Some(i) && self.pointer_in(i, r);
            self.icons.draw(c, name, r.x + (r.w as i32 - size as i32) / 2, r.y + (r.h as i32 - size as i32) / 2, size, th.icon_stroke, if hot { col.ink } else { col.ink_muted });
        }
        if let FieldKind::Number(n) = &opts.kind
            && !n.unit.is_empty()
        {
            let f = self.look.face(style);
            let base = top + f.baseline_in(style.size, style.line);
            f.draw(c, inner.right() as i32 + th.space[1] as i32, base, &n.unit, style.size, fade(col.ink_muted));
        }
        let old = c.clip();
        c.set_clip(old.intersect(&Rect::new(inner.x - 1, b.y + 1, inner.w + 2, b.h.saturating_sub(2))));
        if s.is_empty() {
            let ph = match &self.elems[i].kind {
                ElemKind::TextInput { placeholder, .. } => placeholder.clone(),
                _ => String::new(),
            };
            let f = self.look.face(style);
            let base = top + f.baseline_in(style.size, style.line);
            f.draw(c, inner.x, base, &ph, style.size, fade(col.ink_muted));
        }
        let x0 = inner.x - sx + ax;
        for (li, &(a, e)) in lines.iter().enumerate() {
            let y = top + li as i32 * lh - sy;
            if y + lh < b.y || y > b.bottom() as i32 {
                continue;
            }
            let xs = self.line_xs(&s, a, e, secure);
            let x_of = |o: usize| xs.iter().find(|&&(p, _)| p >= o).map_or(0, |&(_, x)| x);
            // Vyber za textem: v aktivnim poli akcent, jinak neutralni.
            if let Some((sa, sb)) = sel
                && sa <= e
                && sb >= a
            {
                let s0 = x_of(sa.max(a));
                let mut s1 = x_of(sb.min(e));
                // Vybrany konec radku: kousek navic, at je vyber videt.
                if multi && sb > e {
                    s1 += self.dot_size() as i32;
                }
                if s1 > s0 {
                    let bg = if st.focused && self.active { col.accent_soft } else { col.surface_3 };
                    c.fill(Rect::new(x0 + s0, y, (s1 - s0) as u32, lh as u32), bg);
                }
            }
            if secure {
                let d = self.dot_size();
                let step = self.dot_step();
                for &(_, px) in &xs[..xs.len() - 1] {
                    let dx = x0 + px + (step - d) as i32 / 2;
                    c.round_fill(Rect::new(dx, y + (lh - d as i32) / 2, d, d), d / 2, fade(col.ink));
                }
            } else {
                let f = self.look.face(style);
                let base = y + f.baseline_in(style.size, style.line);
                f.draw(c, x0, base, &s[a..e], style.size, fade(col.ink));
            }
        }
        // Kurzor: v aktivnim okne podle blikani. S vlastni vrstvou kurzoru
        // (klient v OS) ho kresli vrstva, blikani pak nic neprekresluje.
        if typing && self.caret_on && !self.caret_layer
            && let Some(r) = self.caret_geom(i)
        {
            c.fill(r, col.accent);
        }
        c.set_clip(old);
        // Text chyby pod polem, jinak varovani Caps Lock.
        let note = match self.elems[i].error.clone() {
            Some(err) => Some((err, col.danger)),
            None if self.caps_warning(i) => Some((String::from("Caps Lock je zapnutý"), col.ink_muted)),
            None => None,
        };
        if let Some((text, ink)) = note {
            let cs = th.text.caption;
            let f = self.look.face(cs);
            let y = b.bottom() as i32 + th.space[1] as i32;
            let base = y + f.baseline_in(cs.size, cs.line);
            f.draw(c, b.x + th.space[2] as i32, base, &text, cs.size, ink);
        }
    }

    /// Obdelnik kurzoru pole `i` (souradnice prvku, oriznuty plochou
    /// textu): svisla cara, v prepisu podtrzeni sirky dalsiho znaku.
    pub(crate) fn caret_geom(&self, i: usize) -> Option<Rect> {
        let (multi, _) = self.field_kind(i)?;
        let key = self.elems[i].key;
        let ed = self.edits.get(&key)?;
        let (cur, over) = (ed.cursor(), ed.overwrite);
        let read_only = self.field_opts(i).is_some_and(|o| o.read_only);
        let secure = self.field_masked(i);
        let b = self.field_box(i);
        let inner = self.field_inner(i);
        let (sx, sy) = self.tscroll.get(&key).copied().unwrap_or((0, 0));
        let lh = self.look.body().line as i32;
        let top = if multi { inner.y } else { b.y + (b.h as i32 - lh) / 2 };
        let x0 = inner.x - sx + self.field_align(i);
        let lines = self.field_lines(i);
        let s = self.field_text(i);
        let (li, cx) = self.caret_pos(i, &lines, cur);
        let y = top + li as i32 * lh - sy;
        let (la, le) = lines[li];
        let next = if cur < le { crate::text::next_boundary(s, cur) } else { cur };
        let r = if over && next > cur && la <= cur && !read_only {
            let xs = self.line_xs(s, la, le, secure);
            let x_of = |o: usize| xs.iter().find(|&&(p, _)| p >= o).map_or(0, |&(_, x)| x);
            let w = (x_of(next) - cx).max(2);
            Rect::new(x0 + cx, y + lh - 2, w as u32, 2)
        } else {
            Rect::new(x0 + cx, y, 2, lh as u32)
        };
        let clip = Rect::new(inner.x - 1, b.y + 1, inner.w + 2, b.h.saturating_sub(2));
        let r = r.intersect(&clip);
        (r.w > 0 && r.h > 0).then_some(r)
    }

    /// Kurzor pole s fokusem pro vrstvu kurzoru klienta: obdelnik
    /// v souradnicich okna (oriznuty posouvanou oblasti), barva a zda je
    /// prave videt (blikani). `None` = zadny kurzor.
    pub fn caret(&self) -> Option<(Rect, u32, bool)> {
        let (i, _) = self.focused_field()?;
        if !self.active || self.elems[i].disabled {
            return None;
        }
        let r = self.caret_geom(i)?;
        let w = self.window_rect(i);
        let e = self.elems[i].rect;
        let mut r = Rect::new(r.x - e.x + w.x, r.y - e.y + w.y, r.w, r.h);
        let sp = self.elems[i].space;
        if sp != 0
            && let Some(a) = self.scroll_areas().into_iter().find(|a| a.index + 1 == sp)
        {
            r = r.intersect(&a.rect);
        }
        if r.w == 0 || r.h == 0 {
            return None;
        }
        Some((r, self.look.theme.color.accent, self.caret_on))
    }

    /// Klient kresli kurzor do vlastni vrstvy ([`Ui::caret`]); prvky ho pak
    /// nekresli a blikani nepotrebuje prekresleni okna.
    pub fn use_caret_layer(&mut self, on: bool) {
        self.caret_layer = on;
    }

    /// Je ukazatel v obdelniku `r` prvku `i` (souradnice prvku)?
    fn pointer_in(&self, i: usize, r: Rect) -> bool {
        let (x, y) = self.to_space(i, self.pointer.0, self.pointer.1);
        r.contains(x, y)
    }
}
