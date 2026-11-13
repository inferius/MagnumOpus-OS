//! Textove pole (`text_input`, `password_input`) a viceradkove pole
//! (`text_area`): geometrie, mys, klavesy, schranka a kresleni.
//!
//! Text a vyber drzi [`crate::text::TextEdit`] podle klice prvku; aplikace
//! dostava novou hodnotu zpravou `on_change` a vraci ji ve `view` (zdroj
//! pravdy je aplikace: kdyz hodnotu upravi, pole ji prevezme).
//!
//! Klavesy: sipky (Ctrl = po slovech, Shift = vyber), Home/End (Ctrl =
//! zacatek/konec textu), nahoru/dolu a PgUp/PgDn ve viceradkovem poli,
//! Backspace/Delete (Ctrl = slovo), Ctrl+A, Ctrl+Z, Ctrl+Shift+Z / Ctrl+Y,
//! Ctrl+C/X/V a Ctrl+Insert, Shift+Delete, Shift+Insert. Enter odesle
//! jednoradkove pole, ve viceradkovem vlozi radek (Ctrl+Enter odesle).
//! Mys: klik = kurzor, Shift+klik rozsiri vyber, tah vybira, dvojklik
//! slovo, trojklik radek.

use alloc::string::String;
use alloc::vec::Vec;

use gfx::{Canvas, Rect};
use inputproto::code::*;
use inputproto::mods;

use crate::event::{Clip, Response};
use crate::layout::ElemKind;
use crate::text::{self, Motion, TextEdit};
use crate::{Key, State, Ui};

/// Nejdelsi prodleva mezi kliky dvojkliku a trojkliku (ns).
pub const MULTI_CLICK: u64 = 400_000_000;

/// Viditelny radek pole: bajtovy rozsah v textu.
pub(crate) type Line = (usize, usize);

/// Znak zkratky: podle rozlozeni (`ch`), jinak podle polohy klavesy.
fn shortcut(code: u16, ch: Option<char>) -> Option<char> {
    if let Some(c) = ch.filter(|c| c.is_ascii_alphabetic()) {
        return Some(c.to_ascii_lowercase());
    }
    Some(match code {
        KEY_A => 'a',
        KEY_C => 'c',
        KEY_V => 'v',
        KEY_X => 'x',
        KEY_Y => 'y',
        KEY_Z => 'z',
        _ => return None,
    })
}

impl<M: Clone> Ui<M> {
    /// Je prvek `i` textove pole? Vraci (viceradkove, heslo).
    pub(crate) fn field_kind(&self, i: usize) -> Option<(bool, bool)> {
        match &self.elems.get(i)?.kind {
            ElemKind::TextInput { multiline, secure, .. } => Some((*multiline, *secure)),
            _ => None,
        }
    }

    /// Ramecek pole (bez radku s chybou pod nim).
    pub(crate) fn field_box(&self, i: usize) -> Rect {
        let e = &self.elems[i];
        let mut r = e.rect;
        if e.error.is_some() {
            let t = &self.look.theme;
            r.h = r.h.saturating_sub(t.text.caption.line + t.space[1]);
        }
        r
    }

    /// Plocha textu uvnitr ramecku.
    pub(crate) fn field_inner(&self, i: usize) -> Rect {
        let b = self.field_box(i);
        let sp = &self.look.theme.space;
        let multi = self.field_kind(i).is_some_and(|k| k.0);
        let (px, py) = (sp[4], if multi { sp[2] } else { 0 });
        Rect::new(b.x + px as i32, b.y + py as i32, b.w.saturating_sub(2 * px), b.h.saturating_sub(2 * py))
    }

    /// Text pole (z editoru, jinak hodnota z rozvrzeni).
    pub(crate) fn field_text(&self, i: usize) -> &str {
        let e = &self.elems[i];
        if let Some(ed) = self.edits.get(&e.key) {
            return ed.text();
        }
        match &e.kind {
            ElemKind::TextInput { value, .. } => value,
            _ => "",
        }
    }

    /// Polohy x hranic grafemu useku `s[a..b]` od jeho zacatku: (bajt, x).
    /// Heslo: tecky se stalym krokem (sirka pismen by prozradila znaky).
    pub(crate) fn line_xs(&self, s: &str, a: usize, b: usize, secure: bool) -> Vec<(usize, i32)> {
        let seg = &s[a..b];
        let bounds = text::boundaries(seg);
        if secure {
            let step = self.dot_step() as i32;
            return bounds.iter().enumerate().map(|(k, &o)| (a + o, k as i32 * step)).collect();
        }
        let st = self.look.body();
        let pos = self.look.face_ref(st).positions(seg, st.size);
        let mut out = Vec::with_capacity(bounds.len());
        let mut bi = 0;
        for (ci, (o, _)) in seg.char_indices().chain(core::iter::once((seg.len(), ' '))).enumerate() {
            if bi < bounds.len() && bounds[bi] == o {
                out.push((a + o, pos[ci.min(pos.len() - 1)]));
                bi += 1;
            }
        }
        out
    }

    /// Prumer tecky hesla a krok mezi teckami.
    pub(crate) fn dot_size(&self) -> u32 {
        (self.look.body().line * 3 / 10).max(4)
    }

    pub(crate) fn dot_step(&self) -> u32 {
        self.dot_size() * 7 / 4
    }

    /// Sirka useku textu.
    fn seg_w(&self, s: &str, secure: bool) -> i32 {
        if secure {
            return text::grapheme_count(s) as i32 * self.dot_step() as i32;
        }
        let st = self.look.body();
        self.look.width(s, st) as i32
    }

    /// Viditelne radky: jednoradkove pole jeden, viceradkove po `\n`
    /// a zalomene podle slov na sirku pole (slovo delsi nez radek se
    /// zalomi po grafemech).
    pub(crate) fn field_lines(&self, i: usize) -> Vec<Line> {
        let s = self.field_text(i);
        let Some((true, secure)) = self.field_kind(i) else { return alloc::vec![(0, s.len())] };
        let w = self.field_inner(i).w as i32;
        let mut out = Vec::new();
        let mut start = 0;
        loop {
            let end = s[start..].find('\n').map_or(s.len(), |p| start + p);
            self.wrap(s, start, end, w, secure, &mut out);
            if end >= s.len() {
                break;
            }
            start = end + 1;
        }
        out
    }

    fn wrap(&self, s: &str, a: usize, b: usize, w: i32, secure: bool, out: &mut Vec<Line>) {
        let mut ls = a;
        if a == b {
            out.push((a, b));
            return;
        }
        while ls < b {
            if self.seg_w(&s[ls..b], secure) <= w {
                out.push((ls, b));
                return;
            }
            // Nejdelsi prefix po hranici slova, ktery se vejde.
            let mut cut = ls;
            let mut k = ls;
            while k < b {
                let n = text::next_word(s, k).min(b);
                if n <= k || self.seg_w(&s[ls..n], secure) > w {
                    break;
                }
                cut = n;
                k = n;
            }
            if cut == ls {
                // Slovo se nevejde: po grafemech (aspon jeden).
                cut = text::next_boundary(s, ls);
                let mut k = cut;
                while k < b {
                    let n = text::next_boundary(s, k);
                    if self.seg_w(&s[ls..n], secure) > w {
                        break;
                    }
                    cut = n;
                    k = n;
                }
            }
            out.push((ls, cut));
            ls = cut;
        }
    }

    /// Radek a x kurzoru (od zacatku plochy textu, bez posunu).
    pub(crate) fn caret_pos(&self, i: usize, lines: &[Line], at: usize) -> (usize, i32) {
        let secure = self.field_kind(i).is_some_and(|k| k.1);
        let s = self.field_text(i);
        // Kurzor na konci zalomeneho radku patri na zacatek dalsiho.
        let li = lines.iter().rposition(|&(a, _)| a <= at).unwrap_or(0);
        let (a, b) = lines[li];
        let x = self.line_xs(s, a, b, secure).iter().find(|&&(o, _)| o >= at).map_or(0, |&(_, x)| x);
        (li, x)
    }

    /// Bajtova pozice nejblizsi bodu `(x, y)` v souradnicich prostoru.
    pub(crate) fn field_offset_at(&self, i: usize, x: i32, y: i32) -> usize {
        let inner = self.field_inner(i);
        let (sx, sy) = self.tscroll.get(&self.elems[i].key).copied().unwrap_or((0, 0));
        let lines = self.field_lines(i);
        let lh = self.look.body().line as i32;
        let multi = self.field_kind(i).is_some_and(|k| k.0);
        let li = if multi { ((y - inner.y + sy).max(0) / lh) as usize } else { 0 }.min(lines.len() - 1);
        self.offset_in_line(i, lines[li], x - inner.x + sx)
    }

    fn offset_in_line(&self, i: usize, (a, b): Line, lx: i32) -> usize {
        let secure = self.field_kind(i).is_some_and(|k| k.1);
        let xs = self.line_xs(self.field_text(i), a, b, secure);
        xs.iter().min_by_key(|&&(_, x)| (x - lx).abs()).map_or(a, |&(o, _)| o)
    }

    /// Posun obsahu pole tak, aby byl kurzor videt.
    pub(crate) fn field_reveal(&mut self, i: usize) {
        let key = self.elems[i].key;
        let Some(at) = self.edits.get(&key).map(|e| e.cursor()) else { return };
        let inner = self.field_inner(i);
        let lines = self.field_lines(i);
        let (li, cx) = self.caret_pos(i, &lines, at);
        let lh = self.look.body().line as i32;
        let multi = self.field_kind(i).is_some_and(|k| k.0);
        let (mut sx, mut sy) = self.tscroll.get(&key).copied().unwrap_or((0, 0));
        if multi {
            sx = 0;
            let top = li as i32 * lh;
            if top < sy {
                sy = top;
            } else if top + lh > sy + inner.h as i32 {
                sy = top + lh - inner.h as i32;
            }
            let max = (lines.len() as i32 * lh - inner.h as i32).max(0);
            sy = sy.clamp(0, max);
        } else {
            sy = 0;
            let w = inner.w as i32 - 2;
            if cx < sx {
                sx = cx;
            } else if cx > sx + w {
                sx = cx - w;
            }
            let secure = self.field_kind(i).is_some_and(|k| k.1);
            let full = self.seg_w(self.field_text(i), secure);
            sx = sx.clamp(0, (full - w).max(0));
        }
        self.tscroll.insert(key, (sx, sy));
    }

    /// Kolecko nad viceradkovym polem: posun obsahu. Vraci, zda pole
    /// posun prijalo (jinak se posune oblast pod nim).
    pub(crate) fn field_wheel(&mut self, i: usize, dy: i32, step: i32) -> bool {
        if self.field_kind(i).is_none_or(|k| !k.0) {
            return false;
        }
        let key = self.elems[i].key;
        let lh = self.look.body().line as i32;
        let max = (self.field_lines(i).len() as i32 * lh - self.field_inner(i).h as i32).max(0);
        let (sx, sy) = self.tscroll.get(&key).copied().unwrap_or((0, 0));
        let ny = (sy - dy * step).clamp(0, max);
        self.tscroll.insert(key, (sx, ny));
        ny != sy
    }

    /// Stisk mysi v poli (`x, y` v prostoru prvku), `time` pro dvojklik.
    pub(crate) fn field_press(&mut self, i: usize, x: i32, y: i32, time: Option<u64>) {
        let key = self.elems[i].key;
        let at = self.field_offset_at(i, x, y);
        let count = match (self.last_click, time) {
            (Some((k, t, lx, ly, n)), Some(now)) if k == key && now.saturating_sub(t) <= MULTI_CLICK && (lx - x).abs() <= 4 && (ly - y).abs() <= 4 => n % 3 + 1,
            _ => 1,
        };
        self.last_click = time.map(|t| (key, t, x, y, count));
        let shift = self.mods & mods::SHIFT != 0;
        let secure = self.field_kind(i).is_some_and(|k| k.1);
        let Some(ed) = self.edits.get_mut(&key) else { return };
        match count {
            // Heslo: dvojklik vybere vse (slova by prozradila mezery).
            2 if secure => ed.select_all(),
            2 => {
                let (a, b) = text::word_at(ed.text(), at);
                ed.select(a, b);
            }
            3 => {
                let (a, b) = text::line_at(ed.text(), at);
                ed.select(a, b);
            }
            _ => ed.set_cursor(at, shift),
        }
        self.drag_field = Some(key);
        self.field_reveal(i);
    }

    /// Tah s drzenym tlacitkem: rozsiri vyber.
    pub(crate) fn field_drag(&mut self, x: i32, y: i32) -> bool {
        let Some(key) = self.drag_field else { return false };
        let Some(i) = self.index_of(key) else { return false };
        // Souradnice prostoru pole (posouvana oblast).
        let (sx, sy) = self.to_space(i, x, y);
        let at = self.field_offset_at(i, sx, sy);
        let Some(ed) = self.edits.get_mut(&key) else { return false };
        if ed.cursor() == at {
            return false;
        }
        ed.set_cursor(at, true);
        self.field_reveal(i);
        true
    }

    /// Bod okna do souradnic prostoru prvku `i`.
    pub(crate) fn to_space(&self, i: usize, x: i32, y: i32) -> (i32, i32) {
        let sp = self.elems[i].space;
        if sp == 0 {
            return (x, y);
        }
        match self.scroll_areas().into_iter().find(|a| a.index + 1 == sp) {
            Some(a) => (x - a.rect.x, y - a.rect.y + a.offset),
            None => (x, y),
        }
    }

    /// Zmena textu editorem: zprava `on_change` a odhaleni kurzoru.
    fn field_changed(&mut self, i: usize, r: &mut Response<M>) {
        let key = self.elems[i].key;
        let t = self.edits.get(&key).map(|e| String::from(e.text())).unwrap_or_default();
        if let ElemKind::TextInput { on_change: Some(f), .. } = &self.elems[i].kind {
            r.msgs.push(f(t));
        }
        self.field_reveal(i);
        r.redraw = true;
    }

    /// Ma pole vlastni preklad klaves (`Node::key_map`)?
    fn has_key_map(&self, i: usize) -> bool {
        matches!(&self.elems[i].kind, ElemKind::TextInput { key_map: Some(_), .. })
    }

    /// Vlozeny znak (psani).
    pub(crate) fn field_insert(&mut self, i: usize, ch: char, r: &mut Response<M>) {
        // S vlastnim prekladem klaves vklada pole az pri stisku klavesy;
        // heslo jen z mistniho fyzickeho vstupu.
        if ch.is_control() || self.elems[i].disabled || self.has_key_map(i) || (self.field_kind(i).is_some_and(|k| k.1) && !self.local_input()) {
            return;
        }
        let key = self.elems[i].key;
        let Some(ed) = self.edits.get_mut(&key) else { return };
        let mut b = [0u8; 4];
        ed.insert(ch.encode_utf8(&mut b));
        self.field_changed(i, r);
    }

    /// Vlozeni ze schranky do pole, ktere o nej pozadalo.
    pub fn paste(&mut self, data: &str) -> Response<M> {
        let mut r = Response::default();
        let Some(key) = self.paste_target.take() else { return r };
        let Some(i) = self.index_of(key) else { return r };
        if self.focus != Some(i) || self.elems[i].disabled {
            return r;
        }
        // Heslo: vlozit jen na gesto z mistniho fyzickeho vstupu.
        if self.field_kind(i).is_some_and(|k| k.1) && self.paste_class != guiproto::window::input_class::LOCAL {
            return r;
        }
        if let Some(ed) = self.edits.get_mut(&key) {
            ed.insert(data);
            self.field_changed(i, &mut r);
        }
        r
    }

    /// Klavesa v poli. Vraci `false`, kdyz ji pole nezpracovalo (Tab,
    /// Esc, sipka nahoru v jednoradkovem poli...).
    pub(crate) fn field_key(&mut self, i: usize, code: u16, ch: Option<char>, r: &mut Response<M>) -> bool {
        let key = self.elems[i].key;
        let Some((multi, secure)) = self.field_kind(i) else { return false };
        let (ctrl, shift) = (self.mods & mods::CTRL != 0, self.mods & mods::SHIFT != 0);
        if self.elems[i].disabled {
            return false;
        }
        // Heslo: zadna editace ze vzdaleneho ani umeleho vstupu (spotrebovano,
        // aby se nedostalo k aplikaci).
        if secure && !self.local_input() {
            return true;
        }
        // Zkratky s Ctrl.
        if ctrl && !matches!(code, KEY_LEFT | KEY_RIGHT | KEY_HOME | KEY_END | KEY_BACKSPACE | KEY_DELETE | KEY_ENTER | KEY_KPENTER | KEY_INSERT) {
            let Some(c) = shortcut(code, ch) else { return false };
            let ed = self.edits.get_mut(&key).unwrap();
            match c {
                'a' => {
                    ed.select_all();
                    r.redraw = true;
                }
                'z' if shift => {
                    if ed.redo() {
                        self.field_changed(i, r);
                    }
                }
                'z' => {
                    if ed.undo() {
                        self.field_changed(i, r);
                    }
                }
                'y' => {
                    if ed.redo() {
                        self.field_changed(i, r);
                    }
                }
                'c' | 'x' => {
                    // Heslo se do schranky nikdy nedostane.
                    if secure || ed.selection().is_none() {
                        return true;
                    }
                    r.clipboard = Some(Clip::Copy(String::from(ed.selected_text())));
                    if c == 'x' {
                        ed.delete(true, false);
                        self.field_changed(i, r);
                    }
                }
                'v' => {
                    self.paste_class = self.input_class;
                    self.paste_target = Some(key);
                    r.clipboard = Some(Clip::Paste);
                }
                _ => return false,
            }
            return true;
        }
        // Insert: Ctrl = kopirovat, Shift = vlozit; Shift+Delete = vyjmout.
        if code == KEY_INSERT || (code == KEY_DELETE && shift) {
            let ed = self.edits.get_mut(&key).unwrap();
            if code == KEY_INSERT && shift {
                self.paste_class = self.input_class;
                self.paste_target = Some(key);
                r.clipboard = Some(Clip::Paste);
            } else if !secure && ed.selection().is_some() {
                r.clipboard = Some(Clip::Copy(String::from(ed.selected_text())));
                if code == KEY_DELETE {
                    ed.delete(true, false);
                    self.field_changed(i, r);
                }
            }
            return true;
        }
        let lines = self.field_lines(i);
        let lh = self.look.body().line as i32;
        let page = (self.field_inner(i).h as i32 / lh).max(1) as usize;
        let word = ctrl && !secure;
        let ed = self.edits.get_mut(&key).unwrap();
        let motion = match code {
            // Heslo: Ctrl+sipky na zacatek a konec (slova by prozradila
            // mezery).
            KEY_LEFT if ctrl && secure => Some(Motion::Start),
            KEY_RIGHT if ctrl && secure => Some(Motion::End),
            KEY_LEFT if word => Some(Motion::WordLeft),
            KEY_RIGHT if word => Some(Motion::WordRight),
            KEY_LEFT => Some(Motion::Left),
            KEY_RIGHT => Some(Motion::Right),
            KEY_HOME if ctrl || !multi => Some(Motion::Start),
            KEY_END if ctrl || !multi => Some(Motion::End),
            _ => None,
        };
        if let Some(m) = motion {
            ed.move_to(m, shift);
            self.field_reveal(i);
            r.redraw = true;
            return true;
        }
        match code {
            KEY_BACKSPACE | KEY_DELETE => {
                if ed.delete(code == KEY_BACKSPACE, word) {
                    self.field_changed(i, r);
                }
            }
            KEY_ENTER | KEY_KPENTER if multi && !ctrl => {
                ed.insert("\n");
                self.field_changed(i, r);
            }
            KEY_ENTER | KEY_KPENTER => {
                if let ElemKind::TextInput { on_submit: Some(m), .. } = &self.elems[i].kind {
                    r.msgs.push(m.clone());
                }
            }
            // Viceradkove pole: Home/End na viditelnem radku, nahoru/dolu
            // a o stranku s pamatovanou polohou x.
            KEY_HOME | KEY_END | KEY_UP | KEY_DOWN | KEY_PAGEUP | KEY_PAGEDOWN if multi => {
                let at = ed.cursor();
                let goal = ed.goal_x;
                let (li, cx) = self.caret_pos(i, &lines, at);
                let target = match code {
                    KEY_HOME => lines[li].0,
                    KEY_END => lines[li].1,
                    _ => {
                        let n = lines.len() - 1;
                        let to = match code {
                            KEY_UP => li.checked_sub(1),
                            KEY_DOWN => (li < n).then_some(li + 1),
                            KEY_PAGEUP => Some(li.saturating_sub(page)),
                            _ => Some((li + page).min(n)),
                        };
                        match to {
                            Some(t) => self.offset_in_line(i, lines[t], goal.unwrap_or(cx)),
                            // Prvni radek nahoru = zacatek, posledni dolu = konec.
                            None if code == KEY_UP => 0,
                            None => self.field_text(i).len(),
                        }
                    }
                };
                let ed = self.edits.get_mut(&key).unwrap();
                ed.set_cursor(target, shift);
                if matches!(code, KEY_UP | KEY_DOWN | KEY_PAGEUP | KEY_PAGEDOWN) {
                    ed.goal_x = Some(goal.unwrap_or(cx));
                }
                self.field_reveal(i);
                r.redraw = true;
            }
            // Vlastni preklad klaves: znak z `key_map` (bez Ctrl a Alt).
            _ if !ctrl && self.mods & mods::ALT == 0 => {
                let ch = match &self.elems[i].kind {
                    ElemKind::TextInput { key_map: Some(f), .. } => f(code, self.mods),
                    _ => return false,
                };
                if let Some(c) = ch.filter(|c| !c.is_control()) {
                    let mut b = [0u8; 4];
                    self.edits.get_mut(&key).unwrap().insert(c.encode_utf8(&mut b));
                    self.field_changed(i, r);
                }
            }
            _ => return false,
        }
        true
    }

    /// Nakresli pole `i` (ramecek, vyber, text, kurzor, chyba).
    pub(crate) fn draw_field(&mut self, c: &mut Canvas<'_>, i: usize, st: State) {
        let th = self.look.theme.clone();
        let col = &th.color;
        let Some((multi, secure)) = self.field_kind(i) else { return };
        let b = self.field_box(i);
        let rad = th.radius.control;
        let edge = if st.error {
            col.danger
        } else if st.focus_visible {
            col.accent
        } else {
            col.border_strong
        };
        let k = if st.disabled { th.opacity.disabled } else { 255 };
        let fade = |c: u32| crate::draw::fade(c, col.surface, k);
        c.round_fill_bordered(b, rad, col.surface, fade(edge), if st.error { 2 } else { 1 });
        if st.focus_visible {
            crate::draw::focus_ring(c, b, rad, th.size.focus_ring, th.size.focus_offset, col.focus);
        }
        let inner = self.field_inner(i);
        let key = self.elems[i].key;
        let (sx, sy) = self.tscroll.get(&key).copied().unwrap_or((0, 0));
        let s = String::from(self.field_text(i));
        let (cur, sel) = self.edits.get(&key).map_or((s.len(), None), |e| (e.cursor(), e.selection()));
        let style = self.look.body();
        let lh = style.line as i32;
        let lines = self.field_lines(i);
        let old = c.clip();
        c.set_clip(old.intersect(&Rect::new(inner.x - 1, b.y + 1, inner.w + 2, b.h.saturating_sub(2))));
        let top = if multi { inner.y } else { b.y + (b.h as i32 - lh) / 2 };
        if s.is_empty() {
            let ph = match &self.elems[i].kind {
                ElemKind::TextInput { placeholder, .. } => placeholder.clone(),
                _ => String::new(),
            };
            let f = self.look.face(style);
            let base = top + f.baseline_in(style.size, style.line);
            f.draw(c, inner.x, base, &ph, style.size, fade(col.ink_muted));
        }
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
                let x0 = x_of(sa.max(a));
                let mut x1 = x_of(sb.min(e));
                // Vybrany konec radku: kousek navic, at je vyber videt.
                if multi && sb > e {
                    x1 += self.dot_size() as i32;
                }
                if x1 > x0 {
                    let bg = if st.focused && self.active { col.accent_soft } else { col.surface_3 };
                    c.fill(Rect::new(inner.x - sx + x0, y, (x1 - x0) as u32, lh as u32), bg);
                }
            }
            if secure {
                let d = self.dot_size();
                let step = self.dot_step();
                for &(_, px) in &xs[..xs.len() - 1] {
                    let dx = inner.x - sx + px + (step - d) as i32 / 2;
                    c.round_fill(Rect::new(dx, y + (lh - d as i32) / 2, d, d), d / 2, fade(col.ink));
                }
            } else {
                let f = self.look.face(style);
                let base = y + f.baseline_in(style.size, style.line);
                f.draw(c, inner.x - sx, base, &s[a..e], style.size, fade(col.ink));
            }
        }
        if st.focus_visible {
            let (li, cx) = self.caret_pos(i, &lines, cur);
            let y = top + li as i32 * lh - sy;
            c.fill(Rect::new(inner.x - sx + cx, y, 2, lh as u32), col.accent);
        }
        c.set_clip(old);
        // Text chyby pod polem.
        if let Some(err) = self.elems[i].error.clone() {
            let cs = th.text.caption;
            let f = self.look.face(cs);
            let y = b.bottom() as i32 + th.space[1] as i32;
            let base = y + f.baseline_in(cs.size, cs.line);
            f.draw(c, b.x + th.space[2] as i32, base, &err, cs.size, col.danger);
        }
    }

    /// Prevezme hodnoty poli z rozvrzeni (po `rebuild`).
    pub(crate) fn sync_fields(&mut self) {
        let mut keep = Vec::new();
        for i in 0..self.elems.len() {
            if let ElemKind::TextInput { value, multiline, .. } = &self.elems[i].kind {
                let key = self.elems[i].key;
                keep.push(key);
                match self.edits.get_mut(&key) {
                    Some(e) if e.multiline() == *multiline => e.set_text(value),
                    _ => {
                        self.edits.insert(key, TextEdit::new(value, *multiline));
                    }
                }
            }
        }
        self.edits.retain(|k, _| keep.contains(k));
        self.tscroll.retain(|k, _| keep.contains(k));
        if self.drag_field.is_some_and(|k| !keep.contains(&k)) {
            self.drag_field = None;
        }
        // Jednoradkove pole: kurzor videt i po zmene sirky; viceradkove jen
        // posun v mezich (kolecko nesmi kazde prekresleni vratit ke kurzoru).
        for i in 0..self.elems.len() {
            match self.field_kind(i) {
                Some((false, _)) => self.field_reveal(i),
                Some((true, _)) => {
                    let key = self.elems[i].key;
                    let lh = self.look.body().line as i32;
                    let max = (self.field_lines(i).len() as i32 * lh - self.field_inner(i).h as i32).max(0);
                    if let Some(s) = self.tscroll.get_mut(&key) {
                        s.1 = s.1.clamp(0, max);
                    }
                }
                None => {}
            }
        }
    }

    /// Klic pole s fokusem.
    pub(crate) fn focused_field(&self) -> Option<(usize, Key)> {
        let f = self.focus?;
        self.field_kind(f).map(|_| (f, self.elems[f].key))
    }
}
