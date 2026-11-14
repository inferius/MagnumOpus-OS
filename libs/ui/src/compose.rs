//! Rozpracovany text u kurzoru pole (mrtva klavesa, pozdeji IME): zprava
//! kompozitoru `Compose` (jako `preedit_string` ve Waylandu
//! text-input-v3). Text se kresli za kurzorem (podtrzeny podle priznaku
//! segmentu), neni soucasti hodnoty pole a nejde do `on_change`; slozeny
//! vysledek prijde jako `Text`. Heslo rozpracovany text nikdy neukaze.
//! Konci prazdnou zpravou, klikem, ztratou fokusu nebo zmenou pole.

use alloc::string::String;

use gfx::{Canvas, Rect};

use crate::Ui;

impl<M: Clone> Ui<M> {
    /// Rozpracovany text od kompozitoru; vraci, zda prekreslit.
    pub fn compose(&mut self, text: &str, cursor_begin: u32, cursor_end: u32, flags: u32) -> bool {
        let target = self.focused_field().filter(|&(i, _)| self.field_editable(i) && !self.field_kind(i).is_some_and(|k| k.1)).map(|(_, k)| k);
        let new = match target {
            Some(k) if !text.is_empty() => {
                let ok = |c: u32| (c as usize) <= text.len() && text.is_char_boundary(c as usize);
                let (b, e) = if ok(cursor_begin) && ok(cursor_end) && cursor_begin <= cursor_end { (cursor_begin, cursor_end) } else { (text.len() as u32, text.len() as u32) };
                Some((k, String::from(text), b, e, flags))
            }
            _ => None,
        };
        if new == self.composing {
            return false;
        }
        self.composing = new;
        self.caret_reset();
        true
    }

    /// Rozpracovany text pole `i`: (text, kurzor v nem, priznaky).
    pub(crate) fn composing_in(&self, i: usize) -> Option<(&str, u32, u32)> {
        let (k, t, _, e, f) = self.composing.as_ref()?;
        (*k == self.elems[i].key && self.focus == Some(i)).then_some((t.as_str(), *e, *f))
    }

    /// Posun kurzoru o sirku rozpracovaneho textu pred nim (px).
    pub(crate) fn compose_shift(&self, i: usize) -> i32 {
        match self.composing_in(i) {
            Some((t, e, _)) => self.look.width(&t[..e as usize], self.look.body()) as i32,
            None => 0,
        }
    }

    /// Nakresli rozpracovany text pole `i` na kurzoru (`x`, `y` = levy
    /// horni roh kurzoru): plocha pole pod nim (zakryje nasledujici
    /// text), text a podtrzeni segmentu.
    pub(crate) fn draw_compose(&mut self, c: &mut Canvas<'_>, i: usize, x: i32, y: i32) {
        let Some((t, _, flags)) = self.composing_in(i).map(|(t, e, f)| (String::from(t), e, f)) else { return };
        let th = self.look.theme.clone();
        let col = &th.color;
        let s = self.look.body();
        let w = self.look.width(&t, s);
        c.fill(Rect::new(x, y, w + 2, s.line), col.surface);
        let f = self.look.face(s);
        let base = y + f.baseline_in(s.size, s.line);
        f.draw(c, x, base, &t, s.size, col.ink);
        if flags & guiproto::window::compose::UNDERLINE != 0 {
            c.fill(Rect::new(x, base + 2, w, 1), col.ink);
        }
    }
}
