//! Vstup a akce pristupnosti -> zpravy aplikace.
//!
//! Tab posouva fokus, Enter a mezernik aktivuji tlacitko, mezernik prepne
//! zaskrtavaci pole, sipky meni vyber v seznamu; textova pole viz
//! [`crate::field`].

use alloc::string::String;
use alloc::vec::Vec;

use guiproto::a11y::actions;
use guiproto::window::InputEvent;
use inputproto::code::*;
use inputproto::mods;

use crate::layout::ElemKind;
use crate::Ui;

/// Vysledek udalosti.
pub struct Response<M> {
    pub msgs: Vec<M>,
    /// Je treba prekreslit (fokus, najeti, kurzor).
    pub redraw: bool,
    /// Zmeneny posun posouvanych oblasti (id oblasti, posun): jen
    /// vlastnost vrstvy v kompozitoru, bez prekresleni.
    pub scrolls: Vec<(u32, i32)>,
    /// Kroky trasovani vstupu (klient je vypise s id udalosti).
    pub trace: Vec<String>,
    /// Pozadavek na schranku (klient ho preda kompozitoru).
    pub clipboard: Option<Clip>,
    /// Akce z manifestu vybrane v nabidce (`MenuAct::Action`): klient
    /// zavola `App::action`.
    pub actions: Vec<(String, Vec<guiproto::action::Arg>)>,
    /// Stisk klavesy, ktery zadny prvek nespotreboval (kod, znak,
    /// modifikatory): klient ho preda aplikaci (`App::key`, zkratky jako
    /// F2, Delete, Ctrl+N, Alt+sipky).
    pub unhandled_key: Option<(u16, Option<char>, u16)>,
}

/// Pozadavek pole na schranku.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Clip {
    /// Zkopirovat text (Ctrl+C, Ctrl+X).
    Copy(String),
    /// Vlozit (Ctrl+V): data prijdou pres [`Ui::paste`].
    Paste,
}

impl<M> Default for Response<M> {
    fn default() -> Self {
        Self {
            msgs: Vec::new(),
            redraw: false,
            scrolls: Vec::new(),
            trace: Vec::new(),
            clipboard: None,
            actions: Vec::new(),
            unhandled_key: None,
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

    /// Udalost bez casu (dvojklik se nerozpozna).
    pub fn event(&mut self, ev: InputEvent, mods_now: u16) -> Response<M> {
        self.event_at(ev, mods_now, None)
    }

    /// Udalost s casem zaznamu (ns): dvojklik a trojklik v textovem poli.
    pub fn event_at(&mut self, ev: InputEvent, mods_now: u16, time: Option<u64>) -> Response<M> {
        self.mods = mods_now;
        // Seznam navrhu u pole: sipky, Enter, Tab, Esc a mys v nem; ostatni
        // klavesy jdou do pole.
        let mut pre = Response::default();
        let open = self.suggest_open();
        let mut r = if self.suggest_event(&ev, &mut pre) { Response::default() } else { self.event_core(ev, time) };
        // Fokus nebo pole se mohly zmenit.
        self.sync_suggest();
        if !pre.msgs.is_empty() {
            pre.msgs.append(&mut r.msgs);
            r.msgs = pre.msgs;
        }
        r.redraw |= pre.redraw || open != self.suggest_open();
        r
    }

    fn event_core(&mut self, ev: InputEvent, time: Option<u64>) -> Response<M> {
        if let Some(t) = time {
            self.now = t;
        }
        // Pamet skupin fokusu pred zmenou (navrat do skupiny).
        if matches!(ev, InputEvent::Key { .. } | InputEvent::Button { pressed: true, .. }) {
            self.remember_focus();
        }
        // Prstenec fokusu jen po ovladani klavesnici (ne po kliknuti).
        match ev {
            InputEvent::Key { action, .. } if action != 0 => self.kbd_focus = true,
            InputEvent::Button { pressed: true, .. } => self.kbd_focus = false,
            _ => {}
        }
        // Kurzor pole: po kazde cinnosti hned videt.
        if matches!(ev, InputEvent::Key { .. } | InputEvent::Text(_) | InputEvent::Button { pressed: true, .. } | InputEvent::Focus(true)) {
            self.caret_reset();
        }
        // Bublina zmizi pri kliku a klavese.
        let hid = matches!(ev, InputEvent::Button { pressed: true, .. } | InputEvent::Key { .. } | InputEvent::PointerLeave | InputEvent::Focus(false)) && self.tip_hide();
        if let Some(mut r) = self.popup_event(&ev, time) {
            r.redraw |= hid;
            return r;
        }
        if let Some(r) = self.control_event(&ev) {
            return r;
        }
        if let Some(r) = self.table_event(&ev, time) {
            return r;
        }
        let mut r = Response::default();
        match ev {
            InputEvent::PointerMove { x, y } => {
                self.pointer = (x, y);
                if self.field_drag(x, y) {
                    r.redraw = true;
                }
                if self.update_hover() {
                    r.redraw = true;
                }
                let h = self.hover;
                if self.tip_shown.is_some() && self.tip.map(|t| t.0) != h.map(|i| self.elems[i].key) {
                    r.redraw = true;
                }
                self.tip_hover(time);
            }
            InputEvent::PointerLeave => {
                self.pointer = (-1, -1);
                if self.update_hover() {
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
                    self.hover = h;
                    if let Some(i) = h
                        && !self.elems[i].disabled
                    {
                        if self.elems[i].focusable() {
                            self.focus = Some(i);
                        }
                        if let ElemKind::ListItem { list, .. } = self.elems[i].kind {
                            self.focus = Some(list - 1);
                        }
                        if self.field_kind(i).is_some() {
                            let (sx, sy) = self.to_space(i, x, y);
                            match self.field_button_at(i, sx, sy) {
                                // Tlacitko v poli (oko, krizek, krok): ne tah.
                                Some(b) => {
                                    self.field_button(i, b, &mut r);
                                    self.pressed = None;
                                }
                                None => self.field_press(i, sx, sy, time),
                            }
                        }
                    }
                    r.redraw = true;
                } else {
                    // Stisk a pusteni se paruji podle klice prvku: mezi nimi
                    // se rozhrani muze znovu rozvrhnout (data ze sluzby),
                    // `rebuild` stisk prevede na novy index; klik se neztrati.
                    let p = self.pressed.take();
                    self.drag_field = None;
                    if p.is_some() && p == h
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
                // Viceradkove pole pod ukazatelem ma prednost pred oblasti.
                if let Some(i) = self.hit(self.pointer.0, self.pointer.1)
                    && self.field_wheel(i, dy, WHEEL_STEP)
                {
                    r.redraw = true;
                } else if let Some(a) = self.scroll_at(self.pointer.0, self.pointer.1) {
                    let max = (a.content_h as i32 - a.rect.h as i32).max(0);
                    let new = (a.offset - dy * WHEEL_STEP).clamp(0, max);
                    if new != a.offset {
                        self.scroll.insert(self.elems[a.index].key, new);
                        r.scrolls.push((a.id, new));
                    }
                }
            }
            // Enter / mezernik na tlacitku a zaskrtavacim poli: stisk = jen
            // stav "stisknuto", akce az pri pusteni (jako mys).
            InputEvent::Key { code, action, .. } if action != 0 && (code == KEY_ENTER || code == KEY_KPENTER || code == KEY_SPACE) && self.activates_on_release() => {
                if action == 1 {
                    self.key_pressed = self.focus;
                    r.redraw = true;
                }
            }
            InputEvent::Key { code, action: 0, .. } if code == KEY_ENTER || code == KEY_KPENTER || code == KEY_SPACE => {
                if let Some(i) = self.key_pressed.take()
                    && self.focus == Some(i)
                {
                    self.activate(i, &mut r);
                }
                r.redraw = true;
            }
            InputEvent::Key { code, action, ch } if action != 0 => {
                if !self.key(code, ch, &mut r) {
                    // Nespotrebovana sipka: prostorova navigace fokusu.
                    let plain = self.mods & (mods::CTRL | mods::ALT | mods::META | mods::SHIFT) == 0;
                    if plain && matches!(code, KEY_LEFT | KEY_RIGHT | KEY_UP | KEY_DOWN) && self.spatial_move(code) {
                        r.redraw = true;
                    } else {
                        r.unhandled_key = Some((code, ch, self.mods));
                    }
                }
            }
            InputEvent::Text(ch) => {
                if let Some((f, _)) = self.focused_field() {
                    self.field_insert(f, ch, &mut r);
                }
            }
            InputEvent::Focus(false) => {
                self.pressed = None;
                self.drag_field = None;
                self.table_press = None;
                self.dragging = None;
                self.key_pressed = None;
                self.hover = None;
                self.hover_part = None;
                self.hover_part = None;
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

    /// Ma prvek s fokusem akci az pri pusteni klavesy (tlacitko, pole)?
    fn activates_on_release(&self) -> bool {
        self.focus.is_some_and(|f| matches!(self.elems[f].kind, ElemKind::Button { .. } | ElemKind::Checkbox { .. } | ElemKind::Switch { .. } | ElemKind::Radio { .. }))
    }

    /// Aktivace prvku (klik, Enter, akce pristupnosti).
    fn activate(&mut self, i: usize, r: &mut Response<M>) {
        if self.elems[i].disabled {
            return;
        }
        if self.activate_control(i, r) {
            return;
        }
        // Tlacitko jen s nabidkou (bez `on_click`): klik ji otevre.
        if matches!(self.elems[i].kind, ElemKind::Button { on_click: None, .. }) && self.elems[i].menu.is_some() {
            self.open_context(i, None);
            r.redraw = true;
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

    /// Klavesa pro prvek s fokusem; `false` = nespotrebovana (dostane ji
    /// aplikace).
    fn key(&mut self, code: u16, ch: Option<char>, r: &mut Response<M>) -> bool {
        if let Some((f, _)) = self.focused_field()
            && code != KEY_TAB
        {
            return self.field_key(f, code, ch, r);
        }
        if code == KEY_TAB {
            return self.focus_step(self.mods & mods::SHIFT != 0, r);
        }
        let Some(f) = self.focus else { return false };
        if self.control_key(f, code, r) {
            return true;
        }
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
                return next.is_some();
            }
            _ => return false,
        }
        true
    }

    /// Dalsi (`back` = predchozi) zastavka poradi Tab.
    pub(crate) fn focus_step(&mut self, back: bool, r: &mut Response<M>) -> bool {
        let n = self.elems.len();
        let order: Vec<usize> = (0..n).filter(|&i| self.elems[i].focusable() && !self.elems[i].skip_tab).collect();
        if order.is_empty() {
            return false;
        }
        let pos = self.focus.and_then(|f| order.iter().position(|&i| i == f));
        let next = match (pos, back) {
            (None, _) => 0,
            (Some(p), false) => (p + 1) % order.len(),
            (Some(p), true) => (p + order.len() - 1) % order.len(),
        };
        self.focus = Some(order[next]);
        self.kbd_focus = true;
        r.redraw = true;
        true
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
            ElemKind::Table(_) => ("tabulka", ""),
            _ => ("jiny", ""),
        };
        alloc::format!("{i} {k} '{t}'")
    }

    /// Akce pristupnosti na uzlu (ctecka obrazovky, test, agent).
    pub fn a11y_action(&mut self, node: u32, action: u32, value: &str) -> Response<M> {
        let mut r = Response::default();
        let Some(i) = self.elem_ids.iter().position(|&id| id == node) else {
            if !self.suggest_a11y_action(node, action, &mut r) && !self.popup_a11y_action(node, action, &mut r) && !self.table_a11y_action(node, action, &mut r) {
                self.control_a11y_action(node, action, &mut r);
            }
            return r;
        };
        if action & actions::FOCUS != 0 && self.elems[i].focusable() {
            self.focus = Some(i);
            self.kbd_focus = true;
            r.redraw = true;
        }
        if action & (actions::ACTIVATE | actions::TOGGLE) != 0 {
            self.activate(i, &mut r);
        }
        if action & actions::SET_VALUE != 0 && self.slider_set(i, value, &mut r) {
            return r;
        }
        // Heslo nenastavi zadna akce pristupnosti (agent, automat): jen
        // mistni fyzicka klavesnice.
        if action & actions::SET_VALUE != 0 && self.field_kind(i).is_some_and(|k| k.1) {
            r.trace.push(String::from("nastaveni hesla akci pristupnosti odmitnuto"));
            return r;
        }
        if action & actions::SET_VALUE != 0 && self.field_kind(i).is_some() && !self.field_editable(i) {
            r.trace.push(String::from("pole je jen pro cteni nebo nedostupne"));
            return r;
        }
        if action & actions::SET_VALUE != 0
            && let ElemKind::TextInput { on_change: Some(f), .. } = &self.elems[i].kind
        {
            r.msgs.push(f(value.into()));
            if let Some(ed) = self.edits.get_mut(&self.elems[i].key) {
                ed.set_text(value);
                ed.set_cursor(value.len(), false);
            }
        }
        r
    }
}
