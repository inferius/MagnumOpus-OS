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
//! Mys: klik = kurzor, Shift+klik rozsiri vyber, tah vybira (za okrajem
//! s automatickym posunem, `caret.rs`), dvojklik slovo, trojklik radek.
//! Alt+sipky a Alt+Backspace/Delete jako Ctrl (po slovech, zvyk z macOS).
//! Insert prepina prepis. Jen pro cteni: vyber a kopirovani ano, zmena ne.
//! Cislo: filtr znaku, sipky krok, PgUp/PgDn 10 kroku, Home/End meze.
//! Hledani: Esc smaze text (druhe Esc pole opusti), sipka dolu na dalsi
//! prvek. Heslo: oko prepina zobrazeni, varovani Caps Lock pod polem.

use alloc::string::String;
use alloc::vec::Vec;

use gfx::Rect;
use inputproto::code::*;
use inputproto::mods;

use crate::event::{Clip, Response};
use crate::fieldopts::{FieldKind, FieldOpts};
use crate::layout::ElemKind;
use crate::text::{self, Motion, TextEdit};
use crate::{Key, Ui};

/// Nejdelsi prodleva mezi kliky dvojkliku a trojkliku (ns).
pub const MULTI_CLICK: u64 = 400_000_000;

/// Tlacitko uvnitr pole (mimo poradi Tab).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FieldBtn {
    /// Oko: zobrazit / skryt heslo.
    Eye,
    /// Krizek: smazat text hledani.
    Clear,
    /// Krokovani cisla.
    Up,
    Down,
}

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

    /// Volby pole (jen pro cteni, delka, druh).
    pub(crate) fn field_opts(&self, i: usize) -> Option<&FieldOpts> {
        match &self.elems.get(i)?.kind {
            ElemKind::TextInput { opts, .. } => Some(opts),
            _ => None,
        }
    }

    /// Kresli se pole teckami (heslo bez zapnuteho zobrazeni)?
    pub(crate) fn field_masked(&self, i: usize) -> bool {
        self.field_kind(i).is_some_and(|k| k.1) && !self.revealed.contains(&self.elems[i].key)
    }

    /// Lze text menit (ne jen pro cteni, ne nedostupne)?
    pub(crate) fn field_editable(&self, i: usize) -> bool {
        !self.elems[i].disabled && self.field_opts(i).is_some_and(|o| !o.read_only)
    }

    /// Sirka tlacitka uvnitr pole (oko, krizek, krokovani).
    pub(crate) fn field_button_w(&self) -> u32 {
        self.look.theme.size.icon_m + self.look.theme.space[2]
    }

    /// Mista v poli vlevo a vpravo od textu: lupa hledani; krizek hledani
    /// (jen s textem), oko hesla, jednotka a krokovani cisla.
    /// Viditelny koncovy slot pole `i` (napoveda klavesy jen u prazdneho
    /// pole; viceradkove pole slot nema).
    pub(crate) fn trailing_of(&self, i: usize) -> Option<crate::fieldopts::Trailing> {
        use crate::fieldopts::Trailing;
        let t = self.field_opts(i)?.trailing.clone()?;
        if self.field_kind(i).is_some_and(|k| k.0) {
            return None;
        }
        match t {
            Trailing::Key(_) if !self.field_text(i).is_empty() => None,
            t => Some(t),
        }
    }

    /// Sirka koncoveho slotu (0 = zadny).
    pub(crate) fn trailing_w(&self, i: usize) -> u32 {
        use crate::fieldopts::Trailing;
        let th = &self.look.theme;
        match self.trailing_of(i) {
            None => 0,
            Some(Trailing::Key(k)) => self.look.width(&k, th.text.caption) + 2 * th.space[2],
            Some(Trailing::Icon(_)) => th.size.icon_s,
            Some(Trailing::Text(s)) => self.look.width(&s, th.text.caption),
        }
    }

    pub(crate) fn field_slots(&self, i: usize) -> (u32, u32) {
        let Some(o) = self.field_opts(i) else { return (0, 0) };
        let th = &self.look.theme;
        let bw = self.field_button_w();
        let editable = self.field_editable(i);
        let secure = self.field_kind(i).is_some_and(|k| k.1);
        let mut right = if secure && !self.elems[i].disabled { bw } else { 0 };
        let left = match &o.kind {
            FieldKind::Search => {
                if editable && !self.field_text(i).is_empty() {
                    right += bw;
                }
                th.size.icon_s + th.space[2]
            }
            FieldKind::Number(n) => {
                if !n.unit.is_empty() {
                    right += self.look.width(&n.unit, self.look.body()) + th.space[1];
                }
                if editable {
                    right += bw;
                }
                0
            }
            FieldKind::Text => 0,
        };
        let tw = self.trailing_w(i);
        if tw > 0 {
            right += tw + th.space[2];
        }
        (left, right)
    }

    /// Posun textu zarovnaneho vpravo (cislo, kdyz se vejde).
    pub(crate) fn field_align(&self, i: usize) -> i32 {
        if !matches!(self.field_opts(i).map(|o| &o.kind), Some(FieldKind::Number(_))) {
            return 0;
        }
        let w = self.field_inner(i).w as i32 - 2;
        let tw = self.seg_w(self.field_text(i), false);
        (w - tw).max(0)
    }

    /// Ramecek pole (bez radku s chybou pod nim).
    pub(crate) fn field_box(&self, i: usize) -> Rect {
        let e = &self.elems[i];
        let mut r = e.rect;
        if e.error.is_some() || self.caps_warning(i) {
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
        let (l, r) = self.field_slots(i);
        Rect::new(b.x + (px + l) as i32, b.y + py as i32, b.w.saturating_sub(2 * px + l + r), b.h.saturating_sub(2 * py))
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
        let Some((true, _)) = self.field_kind(i) else { return alloc::vec![(0, s.len())] };
        let secure = self.field_masked(i);
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
        let secure = self.field_masked(i);
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
        self.offset_in_line(i, lines[li], x - inner.x + sx - self.field_align(i))
    }

    fn offset_in_line(&self, i: usize, (a, b): Line, lx: i32) -> usize {
        let secure = self.field_masked(i);
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
            let full = self.seg_w(self.field_text(i), self.field_masked(i));
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
    pub(crate) fn field_changed(&mut self, i: usize, r: &mut Response<M>) {
        let key = self.elems[i].key;
        let t = self.edits.get(&key).map(|e| String::from(e.text())).unwrap_or_default();
        let empty = t.is_empty();
        if let ElemKind::TextInput { on_change: Some(f), .. } = &self.elems[i].kind {
            r.msgs.push(f(t));
        }
        // Hledani: prazdny text hned, jinak az po prodleve bez psani.
        if matches!(self.field_opts(i).map(|o| &o.kind), Some(FieldKind::Search)) {
            if empty {
                self.search_due = None;
                self.fire_search(key);
                r.msgs.append(&mut self.timer_msgs);
            } else {
                self.search_due = Some((key, self.now + self.look.theme.motion.search_delay as u64 * 1_000_000));
            }
        }
        self.caret_reset();
        self.field_reveal(i);
        r.redraw = true;
    }

    /// Nahradi cely text pole (krok cisla, smazani hledani) jako jeden
    /// krok historie.
    pub(crate) fn field_replace(&mut self, i: usize, t: &str, r: &mut Response<M>) {
        let key = self.elems[i].key;
        let Some(ed) = self.edits.get_mut(&key) else { return };
        if ed.text() == t {
            return;
        }
        ed.select_all();
        if t.is_empty() {
            ed.delete(true, false);
        } else {
            ed.insert(t);
        }
        self.field_changed(i, r);
    }

    /// Krok ciselneho pole o `n` kroku (zaporne dolu).
    pub(crate) fn number_step(&mut self, i: usize, n: i32, r: &mut Response<M>) -> bool {
        let Some(FieldKind::Number(spec)) = self.field_opts(i).map(|o| o.kind.clone()) else { return false };
        if !self.field_editable(i) {
            return true;
        }
        let t = spec.stepped(self.field_text(i), n);
        self.field_replace(i, &t, r);
        if let Some(ed) = self.edits.get_mut(&self.elems[i].key) {
            ed.select_all();
        }
        true
    }

    /// Ma pole vlastni preklad klaves (`Node::key_map`)?
    fn has_key_map(&self, i: usize) -> bool {
        matches!(&self.elems[i].kind, ElemKind::TextInput { key_map: Some(_), .. })
    }

    /// Vlozeny znak (psani).
    pub(crate) fn field_insert(&mut self, i: usize, ch: char, r: &mut Response<M>) {
        // S vlastnim prekladem klaves vklada pole az pri stisku klavesy;
        // heslo jen z mistniho fyzickeho vstupu.
        if ch.is_control() || !self.field_editable(i) || self.has_key_map(i) || (self.field_kind(i).is_some_and(|k| k.1) && !self.local_input()) {
            return;
        }
        if let Some(FieldKind::Number(n)) = self.field_opts(i).map(|o| &o.kind)
            && !n.accepts(ch)
        {
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
        if self.focus != Some(i) || !self.field_editable(i) {
            return r;
        }
        // Heslo: vlozit jen na gesto z mistniho fyzickeho vstupu.
        if self.field_kind(i).is_some_and(|k| k.1) && self.paste_class != guiproto::window::input_class::LOCAL {
            return r;
        }
        // Cislo: jen povolene znaky.
        let data: String = match self.field_opts(i).map(|o| &o.kind) {
            Some(FieldKind::Number(n)) => data.chars().filter(|&c| n.accepts(c)).collect(),
            _ => String::from(data),
        };
        if let Some(ed) = self.edits.get_mut(&key) {
            ed.insert(&data);
            self.field_changed(i, &mut r);
        }
        r
    }

    /// Klavesa v poli. Vraci `false`, kdyz ji pole nezpracovalo (Tab,
    /// Esc, sipka nahoru v jednoradkovem poli...).
    pub(crate) fn field_key(&mut self, i: usize, code: u16, ch: Option<char>, r: &mut Response<M>) -> bool {
        let key = self.elems[i].key;
        let Some((multi, secure)) = self.field_kind(i) else { return false };
        let (ctrl, shift, alt) = (self.mods & mods::CTRL != 0, self.mods & mods::SHIFT != 0, self.mods & mods::ALT != 0);
        if self.elems[i].disabled {
            return false;
        }
        // Heslo: zadna editace ze vzdaleneho ani umeleho vstupu (spotrebovano,
        // aby se nedostalo k aplikaci).
        if secure && !self.local_input() {
            return true;
        }
        let editable = self.field_editable(i);
        let kind = self.field_opts(i).map(|o| o.kind.clone()).unwrap_or_default();
        self.caret_reset();
        // Cislo: krok sipkami a strankou, meze Home / End.
        if let FieldKind::Number(n) = &kind
            && !ctrl
            && !alt
        {
            let step = match code {
                KEY_UP => Some(1),
                KEY_DOWN => Some(-1),
                KEY_PAGEUP => Some(10),
                KEY_PAGEDOWN => Some(-10),
                _ => None,
            };
            if let Some(k) = step {
                return self.number_step(i, k, r);
            }
            let bound = match code {
                KEY_HOME if !shift && n.min > f64::MIN => Some(n.min),
                KEY_END if !shift && n.max < f64::MAX => Some(n.max),
                _ => None,
            };
            if let Some(v) = bound {
                if editable {
                    let t = n.format(v);
                    self.field_replace(i, &t, r);
                }
                return true;
            }
        }
        // Hledani: Esc smaze text (prazdne pole Esc nechava aplikaci),
        // sipka dolu prejde k vysledkum (dalsi prvek v poradi Tab).
        if kind == FieldKind::Search && !ctrl && !alt {
            if code == KEY_ESC {
                if self.field_text(i).is_empty() || !editable {
                    return false;
                }
                self.field_replace(i, "", r);
                return true;
            }
            if code == KEY_DOWN {
                return self.focus_step(false, r);
            }
        }
        // Zkratky s Ctrl.
        if ctrl && !matches!(code, KEY_LEFT | KEY_RIGHT | KEY_HOME | KEY_END | KEY_BACKSPACE | KEY_DELETE | KEY_ENTER | KEY_KPENTER | KEY_INSERT) {
            let Some(c) = shortcut(code, ch) else { return false };
            return self.field_command(i, c, shift, r);
        }
        // Insert: Ctrl = kopirovat, Shift = vlozit, samotny prepne prepis;
        // Shift+Delete = vyjmout.
        if code == KEY_INSERT || (code == KEY_DELETE && shift) {
            let cmd = match (code, ctrl, shift) {
                (KEY_INSERT, true, _) => 'c',
                (KEY_INSERT, _, true) => 'v',
                (KEY_INSERT, _, _) => {
                    if editable {
                        let ed = self.edits.get_mut(&key).unwrap();
                        ed.overwrite = !ed.overwrite;
                        r.redraw = true;
                    }
                    return true;
                }
                _ => 'x',
            };
            return self.field_command(i, cmd, false, r);
        }
        let lines = self.field_lines(i);
        let lh = self.look.body().line as i32;
        let page = (self.field_inner(i).h as i32 / lh).max(1) as usize;
        // Po slovech s Ctrl i Alt (Windows / macOS); heslo nikdy.
        let word = (ctrl || alt) && !secure;
        let ed = self.edits.get_mut(&key).unwrap();
        let motion = match code {
            // Heslo: Ctrl+sipky na zacatek a konec (slova by prozradila
            // mezery).
            KEY_LEFT if (ctrl || alt) && secure => Some(Motion::Start),
            KEY_RIGHT if (ctrl || alt) && secure => Some(Motion::End),
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
                if editable && ed.delete(code == KEY_BACKSPACE, word) {
                    self.field_changed(i, r);
                }
            }
            KEY_ENTER | KEY_KPENTER if multi && !ctrl => {
                if editable {
                    ed.insert("\n");
                    self.field_changed(i, r);
                }
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
            _ if !ctrl && !alt => {
                let ch = match &self.elems[i].kind {
                    ElemKind::TextInput { key_map: Some(f), .. } => f(code, self.mods),
                    _ => return false,
                };
                if let Some(c) = ch.filter(|c| !c.is_control())
                    && editable
                {
                    let mut b = [0u8; 4];
                    self.edits.get_mut(&key).unwrap().insert(c.encode_utf8(&mut b));
                    self.field_changed(i, r);
                }
            }
            _ => return false,
        }
        true
    }

    /// Prikaz pole (zkratka s Ctrl nebo polozka kontextove nabidky):
    /// `a` vse, `z` zpet (`redo` = znovu), `y` znovu, `c` kopirovat,
    /// `x` vyjmout, `v` vlozit, `d` smazat vyber. Vraci `false` pro
    /// neznamy prikaz.
    pub(crate) fn field_command(&mut self, i: usize, c: char, redo: bool, r: &mut Response<M>) -> bool {
        let key = self.elems[i].key;
        let secure = self.field_kind(i).is_some_and(|k| k.1);
        let editable = self.field_editable(i);
        let Some(ed) = self.edits.get_mut(&key) else { return false };
        match c {
            'a' => {
                ed.select_all();
                r.redraw = true;
            }
            'z' | 'y' if !editable => {}
            'z' if redo => {
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
                if c == 'x' && editable {
                    ed.delete(true, false);
                    self.field_changed(i, r);
                }
            }
            'd' => {
                if editable && ed.delete(true, false) {
                    self.field_changed(i, r);
                }
            }
            'v' => {
                if editable {
                    self.paste_class = self.input_class;
                    self.paste_target = Some(key);
                    r.clipboard = Some(Clip::Paste);
                }
            }
            _ => return false,
        }
        true
    }

    /// Prevezme hodnoty poli z rozvrzeni (po `rebuild`).
    pub(crate) fn sync_fields(&mut self) {
        let mut keep = Vec::new();
        for i in 0..self.elems.len() {
            if let ElemKind::TextInput { value, multiline, opts, .. } = &self.elems[i].kind {
                let key = self.elems[i].key;
                keep.push(key);
                match self.edits.get_mut(&key) {
                    Some(e) if e.multiline() == *multiline => e.set_text(value),
                    _ => {
                        self.edits.insert(key, TextEdit::new(value, *multiline));
                    }
                }
                let e = self.edits.get_mut(&key).unwrap();
                e.max = opts.max_len;
                if opts.read_only {
                    e.overwrite = false;
                }
            }
        }
        self.edits.retain(|k, _| keep.contains(k));
        self.revealed.retain(|k| keep.contains(k));
        if self.search_due.is_some_and(|(k, _)| !keep.contains(&k)) {
            self.search_due = None;
        }
        self.tscroll.retain(|k, _| keep.contains(k));
        if self.drag_field.is_some_and(|k| !keep.contains(&k)) {
            self.drag_field = None;
        }
        // Jednoradkove pole s fokusem: kurzor videt i po zmene sirky; bez
        // fokusu zustava posun, jaky byl (nove pole od zacatku textu, ne od
        // kurzoru na konci, O23). Viceradkove jen posun v mezich (kolecko
        // nesmi kazde prekresleni vratit ke kurzoru).
        for i in 0..self.elems.len() {
            match self.field_kind(i) {
                Some((false, _)) if self.focus == Some(i) || self.tscroll.contains_key(&self.elems[i].key) => self.field_reveal(i),
                Some((false, _)) => {}
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
