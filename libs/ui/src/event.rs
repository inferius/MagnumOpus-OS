//! Vstup a akce pristupnosti -> zpravy aplikace.
//!
//! Tab posouva fokus, Enter a mezernik aktivuji tlacitko, mezernik prepne
//! zaskrtavaci pole, sipky meni vyber v seznamu, textove pole edituje
//! vlozeny text (`Text`) a klavesy Backspace, Delete, sipky, Home, End.

use alloc::string::String;
use alloc::vec::Vec;

use guiproto::a11y::actions;
use guiproto::window::InputEvent;
use inputproto::code::*;
use inputproto::mods;

use crate::layout::ElemKind;
use crate::{Ui, node_id};

/// Vysledek udalosti.
pub struct Response<M> {
    pub msgs: Vec<M>,
    /// Je treba prekreslit (fokus, najeti, kurzor).
    pub redraw: bool,
    /// Zmeneny posun posouvanych oblasti (index prvku, posun): jen
    /// vlastnost vrstvy v kompozitoru, bez prekresleni.
    pub scrolls: Vec<(usize, i32)>,
    /// Kroky trasovani vstupu (klient je vypise s id udalosti).
    pub trace: Vec<String>,
}

impl<M> Default for Response<M> {
    fn default() -> Self {
        Self {
            msgs: Vec::new(),
            redraw: false,
            scrolls: Vec::new(),
            trace: Vec::new(),
        }
    }
}

/// Krok kolecka v pixelech.
const WHEEL_STEP: i32 = 40;

impl<M: Clone> Ui<M> {
    /// Prvek v bode okna (nejhlubsi, mimo skupiny).
    pub fn hit(&self, x: i32, y: i32) -> Option<usize> {
        // Posouvany obsah: prevod do souradnic obsahu.
        let mut space = 0usize;
        let (mut sx, mut sy) = (x, y);
        for a in self.scroll_areas() {
            if a.rect.contains(x, y) {
                space = a.index + 1;
                sx = x - a.rect.x;
                sy = y - a.rect.y + a.offset;
            }
        }
        self.elems.iter().enumerate().rev().find_map(|(i, e)| {
            let leaf = !matches!(e.kind, ElemKind::Group | ElemKind::Scroll { .. } | ElemKind::Label { .. });
            (e.space == space && leaf && e.rect.contains(sx, sy)).then_some(i)
        })
    }

    fn scroll_at(&self, x: i32, y: i32) -> Option<crate::ScrollArea> {
        self.scroll_areas().into_iter().rev().find(|a| a.rect.contains(x, y))
    }

    pub fn event(&mut self, ev: InputEvent, mods_now: u16) -> Response<M> {
        self.mods = mods_now;
        let mut r = Response::default();
        match ev {
            InputEvent::PointerMove { x, y } => {
                self.pointer = (x, y);
                let h = self.hit(x, y);
                if h != self.hover {
                    self.hover = h;
                    r.redraw = true;
                }
            }
            InputEvent::PointerLeave => {
                if self.hover.take().is_some() {
                    r.redraw = true;
                }
            }
            InputEvent::Button { code, pressed, x, y } if code == BTN_LEFT => {
                self.pointer = (x, y);
                let h = self.hit(x, y);
                let act = if pressed { "stisk" } else { "pusteni" };
                r.trace.push(alloc::format!("{act} na {x},{y}: prvek {}", self.describe(h)));
                if pressed {
                    self.pressed = h;
                    self.pressed_rect = h.map(|i| (self.elems[i].space, self.elems[i].rect));
                    if let Some(i) = h
                        && !self.elems[i].disabled
                    {
                        if self.elems[i].focusable() {
                            self.focus = Some(i);
                        }
                        if let ElemKind::ListItem { list, .. } = self.elems[i].kind {
                            self.focus = Some(list - 1);
                        }
                        self.click_text(i, x);
                    }
                    r.redraw = true;
                } else {
                    // Stisk a pusteni se paruji podle mista prvku: mezi nimi se
                    // rozhrani muze znovu rozvrhnout (data ze sluzby) a indexy
                    // prvku se posunou; klik se nesmi ztratit.
                    let p = self.pressed.take();
                    let at = self.pressed_rect.take();
                    let same = h.is_some_and(|i| at == Some((self.elems[i].space, self.elems[i].rect)));
                    if p.is_some() && (p == h || same)
                        && let Some(i) = h
                        && !self.elems[i].disabled
                    {
                        self.activate(i, &mut r);
                    } else {
                        let why = if p.is_none() { "stisk nedosel nebo byl zrusen" } else if h.is_some_and(|i| self.elems[i].disabled) { "prvek je nedostupny" } else { "stisk byl na jinem prvku" };
                        r.trace.push(alloc::format!("bez aktivace: {why} (stisk {})", self.describe(p)));
                    }
                    r.redraw = true;
                }
            }
            InputEvent::Scroll { dy, .. } => {
                if let Some(a) = self.scroll_at(self.pointer.0, self.pointer.1) {
                    let max = (a.content_h as i32 - a.rect.h as i32).max(0);
                    let new = (a.offset - dy * WHEEL_STEP).clamp(0, max);
                    if new != a.offset {
                        self.scroll.insert(a.index, new);
                        r.scrolls.push((a.index, new));
                    }
                }
            }
            InputEvent::Key { code, action, .. } if action != 0 => self.key(code, &mut r),
            InputEvent::Text(ch) => self.insert(ch, &mut r),
            InputEvent::Focus(false) => {
                self.pressed = None;
                self.hover = None;
                self.active = false;
                r.redraw = true;
            }
            InputEvent::Focus(true) => {
                self.active = true;
                r.redraw = true;
            }
            _ => {}
        }
        r
    }

    /// Klik do textoveho pole: kurzor na nejblizsi znak.
    fn click_text(&mut self, i: usize, x: i32) {
        if let ElemKind::TextInput { value, .. } = &self.elems[i].kind {
            let b = self.look.body();
            let pos = self.look.face_ref(b).positions(value, b.size);
            let lx = x - self.elems[i].rect.x - self.look.theme.space[4] as i32;
            let best = (0..pos.len()).min_by_key(|&k| (pos[k] - lx).abs()).unwrap_or(0);
            self.cursor.insert(i, best);
        }
    }

    /// Aktivace prvku (klik, Enter, akce pristupnosti).
    fn activate(&mut self, i: usize, r: &mut Response<M>) {
        if self.elems[i].disabled {
            return;
        }
        let d = self.describe(Some(i));
        match &self.elems[i].kind {
            ElemKind::Button { on_click: Some(m), .. } => {
                r.trace.push(alloc::format!("klik tlacitka {d}"));
                r.msgs.push(m.clone());
            }
            ElemKind::Checkbox { checked, on_toggle: Some(f), .. } => {
                r.trace.push(alloc::format!("prepnuti {d} na {}", if *checked { "nezaskrtnuto" } else { "zaskrtnuto" }));
                r.msgs.push(f(!*checked));
            }
            ElemKind::ListItem { index, list, .. } => {
                if let ElemKind::List { on_select: Some(f), .. } = &self.elems[*list - 1].kind {
                    r.trace.push(alloc::format!("vyber {d}"));
                    r.msgs.push(f(*index));
                }
            }
            _ => r.trace.push(alloc::format!("{d}: bez akce")),
        }
    }

    fn key(&mut self, code: u16, r: &mut Response<M>) {
        if code == KEY_TAB {
            let n = self.elems.len();
            let order: Vec<usize> = (0..n).filter(|&i| self.elems[i].focusable()).collect();
            if order.is_empty() {
                return;
            }
            let pos = self.focus.and_then(|f| order.iter().position(|&i| i == f));
            let back = self.mods & mods::SHIFT != 0;
            let next = match (pos, back) {
                (None, _) => 0,
                (Some(p), false) => (p + 1) % order.len(),
                (Some(p), true) => (p + order.len() - 1) % order.len(),
            };
            self.focus = Some(order[next]);
            r.redraw = true;
            return;
        }
        let Some(f) = self.focus else { return };
        match &self.elems[f].kind {
            ElemKind::Button { .. } if code == KEY_ENTER || code == KEY_KPENTER || code == KEY_SPACE => self.activate(f, r),
            ElemKind::Checkbox { .. } if code == KEY_SPACE || code == KEY_ENTER => self.activate(f, r),
            ElemKind::List { count, on_select: Some(sel), .. } => {
                let cur = self.elems.iter().find_map(|e| match e.kind {
                    ElemKind::ListItem { index, selected: true, list, .. } if list == f + 1 => Some(index),
                    _ => None,
                });
                let n = *count;
                let next = match (code, cur) {
                    (KEY_DOWN, None) => Some(0),
                    (KEY_DOWN, Some(c)) => Some((c + 1).min(n.saturating_sub(1))),
                    (KEY_UP, Some(c)) => Some(c.saturating_sub(1)),
                    _ => None,
                };
                if let Some(k) = next
                    && n > 0
                {
                    r.msgs.push(sel(k));
                }
            }
            ElemKind::TextInput { value, on_change, on_submit, .. } => {
                let len = value.chars().count();
                let cur = self.cursor.get(&f).copied().unwrap_or(len).min(len);
                let mut chars: Vec<char> = value.chars().collect();
                let mut changed = false;
                let newcur = match code {
                    KEY_LEFT => cur.saturating_sub(1),
                    KEY_RIGHT => (cur + 1).min(len),
                    KEY_HOME => 0,
                    KEY_END => len,
                    KEY_BACKSPACE if cur > 0 => {
                        chars.remove(cur - 1);
                        changed = true;
                        cur - 1
                    }
                    KEY_DELETE if cur < len => {
                        chars.remove(cur);
                        changed = true;
                        cur
                    }
                    KEY_ENTER | KEY_KPENTER => {
                        if let Some(m) = on_submit {
                            r.msgs.push(m.clone());
                        }
                        cur
                    }
                    _ => return,
                };
                if changed && let Some(f2) = on_change {
                    r.msgs.push(f2(chars.into_iter().collect::<String>()));
                }
                self.cursor.insert(f, newcur);
                r.redraw = true;
            }
            _ => {}
        }
    }

    fn insert(&mut self, ch: char, r: &mut Response<M>) {
        let Some(f) = self.focus else { return };
        if let ElemKind::TextInput { value, on_change: Some(cb), .. } = &self.elems[f].kind {
            let mut chars: Vec<char> = value.chars().collect();
            let cur = self.cursor.get(&f).copied().unwrap_or(chars.len()).min(chars.len());
            chars.insert(cur, ch);
            r.msgs.push(cb(chars.into_iter().collect()));
            self.cursor.insert(f, cur + 1);
            r.redraw = true;
        }
    }

    /// Popis prvku pro trasovani (druh a text, ne hodnota pole: muze byt
    /// heslo).
    fn describe(&self, i: Option<usize>) -> String {
        let Some(i) = i.filter(|&i| i < self.elems.len()) else { return String::from("zadny") };
        let (k, t) = match &self.elems[i].kind {
            ElemKind::Button { text, .. } => ("tlacitko", text.as_str()),
            ElemKind::Checkbox { text, .. } => ("zaskrtavaci pole", text.as_str()),
            ElemKind::ListItem { text, .. } => ("radek", text.as_str()),
            ElemKind::TextInput { .. } => ("textove pole", ""),
            ElemKind::List { .. } => ("seznam", ""),
            _ => ("jiny", ""),
        };
        alloc::format!("{i} {k} '{t}'")
    }

    /// Akce pristupnosti na uzlu (ctecka obrazovky, test, agent).
    pub fn a11y_action(&mut self, node: u32, action: u32, value: &str) -> Response<M> {
        let mut r = Response::default();
        let Some(i) = (0..self.elems.len()).find(|&i| node_id(i) == node) else {
            return r;
        };
        if action & actions::FOCUS != 0 && self.elems[i].focusable() {
            self.focus = Some(i);
            r.redraw = true;
        }
        if action & (actions::ACTIVATE | actions::TOGGLE) != 0 {
            self.activate(i, &mut r);
        }
        if action & actions::SET_VALUE != 0
            && let ElemKind::TextInput { on_change: Some(f), .. } = &self.elems[i].kind
        {
            r.msgs.push(f(value.into()));
            self.cursor.insert(i, value.chars().count());
        }
        r
    }
}
