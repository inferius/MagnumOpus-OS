//! Vyskakovaci prvky okna: rozbalovaci vyber (`select`), nabidka (kontextova
//! `Node::context_menu`) a bublina (`Node::tooltip`).
//!
//! Vzhled podle `.popover` a `.menu` v `design/assets/mo.css`: karta
//! `surface` s ramem `border` (vysoky kontrast 2 px), zaobleni `card`,
//! okraj `space[2]`, polozky vysky `control_h` se zaoblenim `xs`, najeti
//! a vyber `accent_soft`. Bublina jako v kompozitoru (`surface`, ram
//! `border_strong`, pismo `small`).
//!
//! Ovladani nabidky: sipky (preskoci oddelovace a zakazane), Home/End,
//! Enter / mezernik vybere, Esc zavre, psani skoci na polozku, mys: najeti,
//! klik, stisk-tah-pusteni; klik mimo nabidku ji zavre (a nic dalsiho
//! neudela). Vyber se otevre klikem, Enter, mezernikem, Alt+dolu nebo F4;
//! zavreny vyber meni sipky nahoru / dolu primo. Kontextova nabidka: prave
//! tlacitko, Shift+F10, klavesa Menu.
//!
//! Nabidka se kresli do okna (orizne se jeho okrajem); prekryti pres okraj
//! okna bude systemovy popover kompozitoru (dalsi krok).

use alloc::string::String;
use alloc::vec::Vec;

use gfx::{Canvas, Rect};
use guiproto::a11y::{self, Node as A11yNode, Role};
use guiproto::window::InputEvent;
use inputproto::code::*;
use inputproto::mods;

use crate::event::Response;
use crate::layout::ElemKind;
use crate::node::{MenuAct, MenuEntry};
use crate::{Key, State, Ui};

/// Prodleva bubliny (ns, jako v kompozitoru).
pub const TIP_DELAY: u64 = 500_000_000;

/// Nejsirsi bublina (px).
const TIP_MAX_W: u32 = 280;

/// Klavesa Menu (kontextova nabidka).
const KEY_MENU: u16 = 127;

/// Co udela polozka otevrene nabidky.
#[derive(Clone)]
pub(crate) enum PAct<M> {
    Msg(M),
    Action(String, Vec<guiproto::action::Arg>),
    /// Volba `k` rozbalovaciho vyberu.
    Pick(usize),
    /// Prikaz textoveho pole (`Ui::field_command`, nabidka pole).
    Field(char),
    /// Podnabidka (`submenu.rs`).
    Sub(Vec<PItem<M>>),
}

#[derive(Clone)]
pub(crate) struct PItem<M> {
    pub(crate) label: String,
    pub(crate) shortcut: String,
    pub(crate) disabled: String,
    pub(crate) checked: Option<bool>,
    pub(crate) separator: bool,
    pub(crate) act: Option<PAct<M>>,
    /// Mnemonika: pismeno (male) a bajt v `label`, kde se podtrhne.
    pub(crate) mnemonic: Option<(char, usize)>,
}

impl<M> PItem<M> {
    pub(crate) fn usable(&self) -> bool {
        !self.separator && self.disabled.is_empty() && self.act.is_some()
    }
}

/// Otevrena nabidka.
#[derive(Clone)]
pub(crate) struct Popup<M> {
    pub(crate) owner: Key,
    pub(crate) select: bool,
    pub(crate) items: Vec<PItem<M>>,
    /// Obdelnik v souradnicich okna.
    pub(crate) rect: Rect,
    pub(crate) hi: Option<usize>,
    pub(crate) scroll: i32,
    /// Stisk zacal uvnitr nabidky (pusteni nad polozkou ji vybere).
    pub(crate) press_inside: bool,
    /// Ukazatel se od otevreni pohnul (stisk-tah-pusteni u vyberu).
    pub(crate) moved: bool,
    pub(crate) opened_at: (i32, i32),
}

impl<M> Popup<M> {
    pub(crate) fn owner(&self) -> Key {
        self.owner
    }
}

pub(crate) fn virt_id(el: u32, kind: u64, n: u64) -> u32 {
    0x4000_0000 | (Key::ROOT.mix(el as u64).mix(kind).mix(n).0 as u32 & 0x3FFF_FFFF)
}

impl<M: Clone> Ui<M> {
    pub(crate) fn row_h(&self, it: &PItem<M>) -> u32 {
        if it.separator { self.look.theme.space[2] + 1 } else { self.look.control_h() }
    }

    /// Obdelnik prvku `i` v souradnicich okna (prvek v posouvane oblasti).
    pub(crate) fn window_rect(&self, i: usize) -> Rect {
        let r = self.elems[i].rect;
        let sp = self.elems[i].space;
        if sp == 0 {
            return r;
        }
        match self.scroll_areas().into_iter().find(|a| a.index + 1 == sp) {
            Some(a) => Rect::new(r.x + a.rect.x, r.y + a.rect.y - a.offset, r.w, r.h),
            None => r,
        }
    }

    /// Otevre nabidku s polozkami `items` u bodu / pod prvkem `anchor`
    /// (souradnice okna), sirka nejmene `min_w`.
    pub(crate) fn open_popup(&mut self, owner: Key, select: bool, items: Vec<PItem<M>>, anchor: Rect, min_w: u32, hi: Option<usize>) {
        let th = &self.look.theme;
        let pad = th.space[2];
        let body = self.look.body();
        let gap = th.space[3];
        let check = th.size.icon_s + gap;
        let label_w = items.iter().map(|i| self.look.width(&i.label, body)).max().unwrap_or(0);
        let short_w = items.iter().map(|i| self.look.width(&i.shortcut, body)).max().unwrap_or(0);
        let area = self.popup_area();
        let w = (label_w + if short_w > 0 { short_w + 2 * gap } else { 0 } + check + 2 * gap + 2 * pad).max(min_w).max(th.size.hit_min * 5).min(area.w);
        let content: u32 = items.iter().map(|i| self.row_h(i)).sum::<u32>() + 2 * pad;
        let h = content.min(area.h.saturating_sub(2 * th.space[2]));
        // Pod kotvu, kdyz se nevejde, nad ni; vzdy v dovolene plose.
        let below = anchor.bottom() as i32 + 2;
        let y = if below + h as i32 <= area.bottom() as i32 { below } else { (anchor.y - 2 - h as i32).max(area.y) };
        let x = anchor.x.clamp(area.x, (area.right() as i32 - w as i32).max(area.x));
        let p = self.pointer;
        self.popup = Some(Popup { owner, select, items, rect: Rect::new(x, y, w, h), hi, scroll: 0, press_inside: false, moved: false, opened_at: p });
        self.tip = None;
        self.tip_shown = None;
        if let Some(k) = hi {
            self.popup_reveal(k);
        }
    }

    /// Kde smi nabidka a bublina lezet (souradnice okna): s vrstvou
    /// popoveru a znamou polohou okna cela pracovni plocha (presah pres
    /// okraj okna), jinak okno.
    pub(crate) fn popup_area(&self) -> Rect {
        let win = Rect::new(0, 0, self.w, self.h);
        let c = &self.cx;
        if !self.overlay || c.window.w == 0 || c.work_area.w == 0 {
            return win;
        }
        Rect::new(c.work_area.x - c.window.x, c.work_area.y - c.window.y, c.work_area.w, c.work_area.h)
    }

    /// Obdelnik vrstvy popoveru (souradnice okna): otevrena nabidka
    /// a zobrazena bublina; `None` = nic.
    pub fn overlay_rect(&self) -> Option<Rect> {
        let p = self.popup.iter().chain(self.popup_stack.iter()).map(|p| p.rect).reduce(|a, b| a.union(&b));
        let t = self.tip_text().map(|t| self.tip_rect(&t));
        let s = self.suggest_rect();
        [p, t, s].into_iter().flatten().reduce(|a, b| a.union(&b))
    }

    /// Nakresli nabidku, bublinu a seznam navrhu do platna vrstvy popoveru, jejiz pocatek
    /// je `origin` (souradnice okna, viz [`Ui::overlay_rect`]).
    pub fn draw_overlay(&mut self, c: &mut Canvas<'_>, origin: Rect) {
        let saved = (self.popup.clone(), self.pointer);
        if let Some(p) = &mut self.popup {
            p.rect = Rect::new(p.rect.x - origin.x, p.rect.y - origin.y, p.rect.w, p.rect.h);
        }
        self.pointer = (self.pointer.0 - origin.x, self.pointer.1 - origin.y);
        // Bublina se umistuje od ukazatele: posunuta stejne jako nabidka.
        let tip_origin = origin;
        self.tip_shift = (tip_origin.x, tip_origin.y);
        c.fill(c.bounds(), 0);
        self.draw_overlays(c);
        self.tip_shift = (0, 0);
        (self.popup, self.pointer) = saved;
    }

    pub(crate) fn popup_reveal(&mut self, k: usize) {
        let Some(p) = &self.popup else { return };
        let pad = self.look.theme.space[2] as i32;
        let top: i32 = p.items[..k].iter().map(|i| self.row_h(i) as i32).sum();
        let h = self.row_h(&p.items[k]) as i32;
        let view = p.rect.h as i32 - 2 * pad;
        let total: i32 = p.items.iter().map(|i| self.row_h(i) as i32).sum();
        let mut s = p.scroll;
        if top < s {
            s = top;
        } else if top + h > s + view {
            s = top + h - view;
        }
        if let Some(p) = &mut self.popup {
            p.scroll = s.clamp(0, (total - view).max(0));
        }
    }

    /// Polozka nabidky pod bodem okna.
    pub(crate) fn popup_item_at(&self, x: i32, y: i32) -> Option<usize> {
        let p = self.popup.as_ref()?;
        if !p.rect.contains(x, y) {
            return None;
        }
        let pad = self.look.theme.space[2] as i32;
        let mut top = p.rect.y + pad - p.scroll;
        for (k, it) in p.items.iter().enumerate() {
            let h = self.row_h(it) as i32;
            if y >= top && y < top + h {
                return it.usable().then_some(k);
            }
            top += h;
        }
        None
    }

    /// Polozky z `MenuEntry` (kontextova nabidka).
    pub(crate) fn items_from(entries: &[MenuEntry<M>]) -> Vec<PItem<M>> {
        entries
            .iter()
            .map(|e| {
                let (label, mnemonic) = crate::submenu::mnemonic(&e.label);
                PItem {
                    label,
                    shortcut: e.shortcut.clone(),
                    disabled: e.disabled.clone(),
                    checked: e.checked,
                    separator: e.separator,
                    act: e.act.as_ref().map(|a| match a {
                        MenuAct::Msg(m) => PAct::Msg(m.clone()),
                        MenuAct::Action(id, args) => PAct::Action(id.clone(), args.clone()),
                        MenuAct::Sub(items) => PAct::Sub(Self::items_from(items)),
                    }),
                    mnemonic,
                }
            })
            .collect()
    }

    /// Otevre rozbalovaci vyber `i`.
    fn open_select(&mut self, i: usize) {
        let ElemKind::Select { options, selected, .. } = &self.elems[i].kind else { return };
        let items = options
            .iter()
            .enumerate()
            .map(|(k, o)| PItem { label: o.clone(), shortcut: String::new(), disabled: String::new(), checked: Some(k == *selected), separator: false, act: Some(PAct::Pick(k)), mnemonic: None })
            .collect();
        let sel = (*selected < options.len()).then_some(*selected);
        let r = self.window_rect(i);
        self.open_popup(self.elems[i].key, true, items, r, r.w, sel);
    }

    /// Kontextova nabidka prvku `i` nebo jeho nejblizsiho predka, ktery ji
    /// ma; `at` = bod okna (mys) nebo `None` (klavesnice: pod prvkem).
    /// Kontextova nabidka prvku `i` nebo nejblizsiho predka, ktery ji ma:
    /// (index vlastnika, polozky).
    fn context_of(&self, i: usize) -> Option<(usize, Vec<PItem<M>>)> {
        let mut k = Some(i);
        while let Some(j) = k {
            if let Some(m) = &self.elems.get(j)?.menu {
                return Some((j, Self::items_from(m)));
            }
            k = self.elems[j].parent.checked_sub(1);
        }
        None
    }

    pub(crate) fn open_context(&mut self, i: usize, at: Option<(i32, i32)>) -> bool {
        let Some((j, items)) = self.context_of(i) else { return false };
        if items.is_empty() {
            return false;
        }
        let anchor = match at {
            Some((x, y)) => Rect::new(x, y - 2, 1, 1),
            None => self.window_rect(i),
        };
        self.open_popup(self.elems[j].key, false, items, anchor, 0, None);
        true
    }

    /// Po prestavbe pohledu: otevrena kontextova nabidka (ne vyber, ne
    /// podnabidka) prevezme polozky z noveho pohledu. Pravy klik na radek
    /// tabulky nabidku otevre hned, ale vyber radku zpracuje aplikace az
    /// potom (`on_context`); bez obnovy by nabidka ukazovala stav pred
    /// vyberem (zakazane akce). Jiny pocet polozek = jina nabidka: zustane.
    pub(crate) fn refresh_context(&mut self) {
        if !self.popup_stack.is_empty() {
            return;
        }
        let Some(owner) = self.popup.as_ref().filter(|p| !p.select).map(|p| p.owner) else { return };
        let Some(i) = self.index_of(owner) else { return };
        let Some((j, items)) = self.context_of(i) else { return };
        if self.elems[j].key != owner {
            return;
        }
        if let Some(p) = self.popup.as_mut()
            && p.items.len() == items.len()
        {
            p.items = items;
        }
    }

    pub(crate) fn close_popup(&mut self, r: &mut Response<M>) {
        self.popup_stack.clear();
        if self.popup.take().is_some() {
            r.redraw = true;
        }
    }

    /// Vybere polozku `k` otevrene nabidky a zavre ji.
    pub(crate) fn choose(&mut self, k: usize, r: &mut Response<M>) {
        // Podnabidka se otevre, nabidka zustane.
        if self.popup.as_ref().is_some_and(|p| matches!(p.items.get(k).filter(|i| i.usable()).and_then(|i| i.act.as_ref()), Some(PAct::Sub(_)))) {
            self.open_submenu(k);
            r.redraw = true;
            return;
        }
        let Some(p) = self.popup.take() else { return };
        // Vlastnik vybrane polozky je koren (podnabidky maji stejneho).
        self.popup_stack.clear();
        r.redraw = true;
        let Some(it) = p.items.get(k).filter(|i| i.usable()) else { return };
        match it.act.clone() {
            Some(PAct::Msg(m)) => r.msgs.push(m),
            Some(PAct::Action(id, args)) => r.actions.push((id, args)),
            Some(PAct::Field(c)) => {
                if let Some(i) = self.index_of(p.owner) {
                    self.field_command(i, c, false, r);
                }
            }
            Some(PAct::Sub(_)) => {}
            Some(PAct::Pick(n)) => {
                if let Some(i) = self.index_of(p.owner)
                    && let ElemKind::Select { selected, on_select: Some(f), .. } = &self.elems[i].kind
                    && *selected != n
                {
                    r.msgs.push(f(n));
                }
            }
            None => {}
        }
    }

    /// Dalsi pouzitelna polozka od `from` smerem `dir`.
    pub(crate) fn step_item(&self, from: Option<usize>, dir: i32) -> Option<usize> {
        let p = self.popup.as_ref()?;
        let n = p.items.len() as i32;
        let mut k = match from {
            Some(k) => k as i32,
            None if dir > 0 => -1,
            None => n,
        };
        for _ in 0..n {
            k += dir;
            if k < 0 || k >= n {
                return from;
            }
            if p.items[k as usize].usable() {
                return Some(k as usize);
            }
        }
        from
    }

    /// Udalosti pri otevrene nabidce (vsechny jsou jeji) a otevirani
    /// vyberu a kontextove nabidky. `None` = obecne zpracovani.
    pub(crate) fn popup_event(&mut self, ev: &InputEvent, time: Option<u64>) -> Option<Response<M>> {
        let mut r = Response::default();
        if let Some(t) = time {
            self.now = t;
        }
        if self.popup.is_none() {
            return self.popup_open_event(ev);
        }
        match *ev {
            InputEvent::PointerMove { x, y } => {
                self.pointer = (x, y);
                // Ukazatel nad rodicovskou nabidkou: podnabidky pod ni zavrit.
                if self.submenu_pointer(x, y) {
                    r.redraw = true;
                }
                let hi = self.popup_item_at(x, y);
                let p = self.popup.as_mut().unwrap();
                if (x - p.opened_at.0).abs().max((y - p.opened_at.1).abs()) > 4 {
                    p.moved = true;
                }
                if hi.is_some() && hi != p.hi {
                    p.hi = hi;
                    r.redraw = true;
                    // Najeti na podnabidku ji otevre.
                    if let Some(k) = hi
                        && matches!(self.popup.as_ref().unwrap().items[k].act, Some(PAct::Sub(_)))
                    {
                        self.open_submenu(k);
                    }
                }
            }
            InputEvent::Button { pressed: true, x, y, .. } => {
                self.submenu_pointer(x, y);
                if self.popup.as_ref().unwrap().rect.contains(x, y) {
                    self.popup.as_mut().unwrap().press_inside = true;
                } else {
                    // Klik mimo nabidku ji jen zavre.
                    self.close_popup(&mut r);
                    self.pressed = None;
                }
            }
            InputEvent::Button { pressed: false, x, y, .. } => {
                let p = self.popup.as_ref().unwrap();
                let release_picks = p.press_inside || (p.select && p.moved);
                if let Some(k) = self.popup_item_at(x, y)
                    && release_picks
                {
                    self.choose(k, &mut r);
                }
                if let Some(p) = self.popup.as_mut() {
                    p.press_inside = false;
                }
                self.pressed = None;
            }
            InputEvent::Scroll { dy, .. } => {
                let pad = self.look.theme.space[2] as i32;
                let p = self.popup.as_ref().unwrap();
                let total: i32 = p.items.iter().map(|i| self.row_h(i) as i32).sum();
                let max = (total - (p.rect.h as i32 - 2 * pad)).max(0);
                let step = self.look.control_h() as i32;
                let p = self.popup.as_mut().unwrap();
                p.scroll = (p.scroll - dy * step).clamp(0, max);
                r.redraw = true;
            }
            InputEvent::Key { code, action, ch } if action != 0 => {
                let hi = self.popup.as_ref().unwrap().hi;
                let sub = hi.is_some_and(|k| matches!(self.popup.as_ref().unwrap().items[k].act, Some(PAct::Sub(_))));
                let to = match code {
                    // Podnabidka: vpravo otevre, vlevo (a Esc) zavre.
                    KEY_RIGHT if sub => {
                        self.open_submenu(hi.unwrap());
                        r.redraw = true;
                        return Some(r);
                    }
                    KEY_LEFT | KEY_ESC if !self.popup_stack.is_empty() => {
                        self.close_submenu();
                        r.redraw = true;
                        return Some(r);
                    }
                    KEY_DOWN => self.step_item(hi, 1),
                    KEY_UP => self.step_item(hi, -1),
                    KEY_HOME => self.step_item(None, 1),
                    KEY_END => self.step_item(None, -1),
                    KEY_ENTER | KEY_KPENTER | KEY_SPACE => {
                        if let Some(k) = hi {
                            self.choose(k, &mut r);
                        }
                        return Some(r);
                    }
                    // Tab ve vyberu zvoli aktivni polozku a jde dal (jako
                    // ostatni systemy); v nabidce jen zavre.
                    KEY_TAB if self.popup.as_ref().unwrap().select => {
                        if let Some(k) = hi {
                            self.choose(k, &mut r);
                        }
                        self.close_popup(&mut r);
                        self.focus_step(self.mods & mods::SHIFT != 0, &mut r);
                        return Some(r);
                    }
                    KEY_ESC | KEY_TAB => {
                        self.close_popup(&mut r);
                        return Some(r);
                    }
                    _ => {
                        // Mnemonika: jedina polozka s pismenem se rovnou vybere.
                        if let Some(c) = ch.filter(|c| !c.is_control())
                            && let Some(k) = self.mnemonic_pick(c, hi)
                        {
                            match k {
                                Ok(k) => self.choose(k, &mut r),
                                Err(k) => {
                                    self.popup.as_mut().unwrap().hi = Some(k);
                                    self.popup_reveal(k);
                                }
                            }
                            r.redraw = true;
                            return Some(r);
                        }
                        // Psani: prvni pouzitelna polozka zacinajici pismenem.
                        let c = ch.filter(|c| !c.is_control()).map(|c| c.to_lowercase().collect::<String>());
                        let p = self.popup.as_ref().unwrap();
                        c.and_then(|c| {
                            let n = p.items.len();
                            let start = hi.map_or(0, |h| h + 1);
                            (0..n).map(|d| (start + d) % n).find(|&k| p.items[k].usable() && p.items[k].label.to_lowercase().starts_with(&c))
                        })
                    }
                };
                if let Some(k) = to {
                    self.popup.as_mut().unwrap().hi = Some(k);
                    self.popup_reveal(k);
                    r.redraw = true;
                }
            }
            InputEvent::Focus(false) | InputEvent::PointerLeave => {
                if matches!(ev, InputEvent::Focus(false)) {
                    self.close_popup(&mut r);
                }
                return None;
            }
            _ => {}
        }
        Some(r)
    }

    /// Otevreni vyberu (stisk, klavesy) a kontextove nabidky; zavreny
    /// vyber sipkami meni volbu. Ostatni udalosti `None`.
    fn popup_open_event(&mut self, ev: &InputEvent) -> Option<Response<M>> {
        let mut r = Response::default();
        match *ev {
            InputEvent::Button { code, pressed: true, x, y } if code == BTN_LEFT => {
                let i = self.hit(x, y)?;
                if !matches!(self.elems[i].kind, ElemKind::Select { .. }) || self.elems[i].disabled {
                    return None;
                }
                self.pointer = (x, y);
                self.focus = Some(i);
                self.hover = Some(i);
                self.open_select(i);
                r.redraw = true;
                Some(r)
            }
            InputEvent::Button { code, pressed: true, x, y } if code == BTN_RIGHT => {
                let i = self.hit(x, y)?;
                // Tabulka nejdriv vybere radek (`on_context`), nabidku otevre sama.
                if matches!(self.elems[i].kind, ElemKind::Table(_)) {
                    return None;
                }
                if self.field_kind(i).is_some() && !self.elems[i].disabled {
                    self.pointer = (x, y);
                    self.kbd_focus = false;
                    self.open_field_menu(i, Some((x, y)));
                    r.redraw = true;
                    return Some(r);
                }
                self.pointer = (x, y);
                if self.elems[i].focusable() {
                    self.focus = Some(i);
                }
                self.open_context(i, Some((x, y))).then(|| {
                    r.redraw = true;
                    r
                })
            }
            InputEvent::Key { code, action, ch } if action != 0 => {
                let i = self.focus?;
                let shift = self.mods & mods::SHIFT != 0;
                if code == KEY_MENU || (code == KEY_F10 && shift) {
                    if self.field_kind(i).is_some() && !self.elems[i].disabled {
                        self.open_field_menu(i, None);
                        r.redraw = true;
                        return Some(r);
                    }
                    return self.open_context(i, None).then(|| {
                        r.redraw = true;
                        r
                    });
                }
                let ElemKind::Select { options, selected, on_select } = &self.elems[i].kind else { return None };
                let alt = self.mods & mods::ALT != 0;
                match code {
                    KEY_ENTER | KEY_KPENTER | KEY_SPACE | KEY_F4 => {}
                    KEY_DOWN if alt => {}
                    _ if self.mods & (mods::CTRL | mods::ALT) == 0 && (matches!(code, KEY_HOME | KEY_END) || ch.is_some_and(|c| !c.is_control() && c != ' ')) => {
                        return self.select_key(i, code, ch, &mut r).then_some(r);
                    }
                    KEY_UP | KEY_DOWN => {
                        let n = options.len();
                        let k = if code == KEY_UP { selected.saturating_sub(1) } else { (selected + 1).min(n.saturating_sub(1)) };
                        if k != *selected
                            && let Some(f) = on_select
                        {
                            r.msgs.push(f(k));
                        }
                        r.redraw = true;
                        return Some(r);
                    }
                    _ => return None,
                }
                if self.elems[i].disabled {
                    return Some(r);
                }
                self.open_select(i);
                r.redraw = true;
                Some(r)
            }
            _ => None,
        }
    }

    /// Nakresli rozbalovaci vyber (pole se sipkou).
    pub(crate) fn draw_select(&mut self, c: &mut Canvas<'_>, i: usize, st: State) {
        let th = self.look.theme.clone();
        let col = &th.color;
        let r = self.elems[i].rect;
        let ElemKind::Select { options, selected, .. } = &self.elems[i].kind else { return };
        let text = options.get(*selected).cloned().unwrap_or_default();
        let k = if st.disabled { th.opacity.disabled } else { 255 };
        let fade = |x: u32| crate::draw::fade(x, col.surface, k);
        let bg = if st.hover || st.pressed { col.surface_3 } else { col.surface };
        c.round_fill_bordered(r, th.radius.control, fade(bg), fade(col.border_strong), 1);
        if st.focus_visible {
            crate::draw::focus_ring(c, r, th.radius.control, th.size.focus_ring, th.size.focus_offset, col.focus);
        }
        let is = th.size.icon_s;
        let body = self.look.body();
        let room = r.w.saturating_sub(2 * th.space[4] + is + th.space[2]);
        let f = self.look.face(body);
        let b = r.y + f.baseline_in(body.size, r.h);
        f.draw_ellipsized(c, r.x + th.space[4] as i32, b, &text, body.size, fade(col.ink), room);
        self.icons.draw(c, "down", r.right() as i32 - (th.space[4] + is) as i32, r.y + (r.h - is) as i32 / 2, is, th.icon_stroke, fade(col.ink_muted));
    }

    /// Nakresli otevrenou nabidku a bublinu (vrstva okna, nad prvky).
    pub(crate) fn draw_overlays(&mut self, c: &mut Canvas<'_>) {
        self.draw_suggest(c);
        self.draw_popup(c);
        self.draw_tip(c);
    }

    fn draw_popup(&mut self, c: &mut Canvas<'_>) {
        // Rodicovske nabidky pod podnabidkou.
        let levels: Vec<Popup<M>> = self.popup_stack.iter().cloned().chain(self.popup.clone()).collect();
        for p in levels {
            self.draw_one_popup(c, p);
        }
    }

    fn draw_one_popup(&mut self, c: &mut Canvas<'_>, p: Popup<M>) {
        let th = self.look.theme.clone();
        let col = &th.color;
        let edge = if th.mode == theme::Mode::Contrast { 2 } else { 1 };
        c.round_fill_bordered(p.rect, th.radius.card, col.surface, col.border, edge);
        let pad = th.space[2] as i32;
        let old = c.clip();
        let inner = Rect::new(p.rect.x + pad, p.rect.y + pad, p.rect.w.saturating_sub(2 * pad as u32), p.rect.h.saturating_sub(2 * pad as u32));
        c.set_clip(old.intersect(&inner));
        let body = self.look.body();
        let gap = th.space[3] as i32;
        let is = th.size.icon_s;
        let mut y = inner.y - p.scroll;
        for (k, it) in p.items.iter().enumerate() {
            let h = self.row_h(it) as i32;
            if it.separator {
                c.fill(Rect::new(inner.x + gap, y + h / 2, inner.w.saturating_sub(2 * gap as u32), 1), col.border);
                y += h;
                continue;
            }
            let row = Rect::new(inner.x, y, inner.w, h as u32);
            if p.hi == Some(k) {
                c.round_fill(row, th.radius.xs, col.accent_soft);
            }
            let ink = if it.disabled.is_empty() { col.ink } else { col.ink_faint };
            let mut x = row.x + gap;
            if it.checked == Some(true) {
                self.icons.draw(c, "check", x, y + (h - is as i32) / 2, is, th.icon_stroke, ink);
            }
            x += (is + th.space[3]) as i32;
            let f = self.look.face(body);
            let b = y + f.baseline_in(body.size, h as u32);
            let sw = if it.shortcut.is_empty() { 0 } else { f.width(&it.shortcut, body.size) as i32 + 2 * gap };
            f.draw_ellipsized(c, x, b, &it.label, body.size, ink, (row.right() as i32 - x - sw).max(0) as u32);
            if !it.shortcut.is_empty() {
                let w = f.width(&it.shortcut, body.size) as i32;
                f.draw(c, row.right() as i32 - gap - w, b, &it.shortcut, body.size, col.ink_muted);
            }
            // Mnemonika podtrzena, podnabidka sipkou vpravo.
            if let Some((_, at)) = it.mnemonic
                && at < it.label.len()
            {
                let x0 = x + f.width(&it.label[..at], body.size) as i32;
                let end = it.label[at..].chars().next().map_or(at, |ch| at + ch.len_utf8());
                let w = f.width(&it.label[at..end], body.size);
                c.fill(Rect::new(x0, b + 2, w, 1), ink);
            }
            if matches!(it.act, Some(PAct::Sub(_))) {
                self.icons.draw(c, "right", row.right() as i32 - gap - is as i32, y + (h - is as i32) / 2, is, th.icon_stroke, col.ink_muted);
            }
            y += h;
        }
        c.set_clip(old);
    }

    /// Obdelnik bubliny (pod ukazatelem, uvnitr okna).
    fn tip_rect(&self, text: &str) -> Rect {
        let th = &self.look.theme;
        let s = th.text.small;
        // Plocha v souradnicich okna; pri kresleni do vrstvy popoveru je
        // ukazatel posunuty o jeji pocatek (`tip_shift`).
        let (sx, sy) = self.tip_shift;
        let a = self.popup_area();
        let area = Rect::new(a.x - sx, a.y - sy, a.w, a.h);
        let w = (self.look.width(text, s) + 2 * th.space[4]).min(TIP_MAX_W).min(area.w);
        let h = s.line + 2 * th.space[2];
        let (px, py) = self.pointer;
        let below = py + th.size.icon_m as i32;
        let y = if below + h as i32 <= area.bottom() as i32 { below } else { (py - h as i32 - th.space[2] as i32).max(area.y) };
        Rect::new(px.clamp(area.x, (area.right() as i32 - w as i32).max(area.x)), y, w, h)
    }

    fn tip_text(&self) -> Option<String> {
        let k = self.tip_shown?;
        self.elems[self.index_of(k)?].tooltip.clone()
    }

    fn draw_tip(&mut self, c: &mut Canvas<'_>) {
        let Some(text) = self.tip_text() else { return };
        let th = self.look.theme.clone();
        let r = self.tip_rect(&text);
        let s = th.text.small;
        c.round_fill_bordered(r, th.radius.control, th.color.surface, th.color.border_strong, 1);
        let f = self.look.face(s);
        let b = r.y + f.baseline_in(s.size, r.h);
        f.draw_ellipsized(c, r.x + th.space[4] as i32, b, &text, s.size, th.color.ink, r.w.saturating_sub(2 * th.space[4]));
    }

    /// Najeti na prvek s bublinou (cas `time`): odpocet, jinak bublinu
    /// skryt. Volat po zmene najeti.
    pub(crate) fn tip_hover(&mut self, time: Option<u64>) {
        let k = self.hover.filter(|&i| self.elems[i].tooltip.is_some()).map(|i| self.elems[i].key);
        if self.tip.map(|t| t.0) != k {
            self.tip = k.map(|k| (k, time.unwrap_or(self.now)));
            self.tip_shown = None;
        }
    }

    /// Skryje bublinu (klik, klavesa, odjeti).
    pub(crate) fn tip_hide(&mut self) -> bool {
        let shown = self.tip_shown.take().is_some();
        self.tip = None;
        shown
    }

    /// Je otevrena nabidka (vyber, kontextova)?
    /// Otevre nabidku `items` u obdelniku `anchor` (souradnice okna) bez
    /// prvku, ke kteremu by patrila: nabidka zastupce v liste kompozitoru.
    /// Vlastnikem je koren pohledu; vyber polozky vrati jeji zpravu.
    pub fn open_menu(&mut self, items: Vec<crate::MenuEntry<M>>, anchor: Rect) {
        let items = Self::items_from(&items);
        let owner = self.elems.first().map_or(crate::Key::ROOT, |e| e.key);
        self.open_popup(owner, false, items, anchor, 0, None);
    }

    /// Obdelnik otevrene nabidky (souradnice okna).
    pub fn popup_rect(&self) -> Option<Rect> {
        self.popup.as_ref().map(|p| p.rect)
    }

    pub fn popup_open(&self) -> bool {
        self.popup.is_some()
    }

    /// Kdy klient ma probudit `Ui::timer` (bublina, kurzor a hledani
    /// poli, `caret.rs`), `None` = nic neceka.
    pub fn next_deadline(&self) -> Option<u64> {
        let tip = match (self.tip, self.tip_shown) {
            (Some((_, t)), None) => Some(t + TIP_DELAY),
            _ => None,
        };
        [tip, self.caret_deadline()].into_iter().flatten().min()
    }

    /// Cas `now` (ns, stejne hodiny jako cas vstupu): ukaze bublinu, kdyz
    /// uplynula prodleva. Vraci, zda prekreslit.
    pub fn timer(&mut self, now: u64) -> bool {
        self.now = now;
        let mut redraw = self.caret_timer(now);
        if let (Some((k, t)), None) = (self.tip, self.tip_shown)
            && now >= t + TIP_DELAY
            && self.popup.is_none()
        {
            self.tip_shown = Some(k);
            redraw = true;
        }
        redraw
    }

    /// Uzly pristupnosti otevrene nabidky a bubliny.
    pub(crate) fn popup_a11y(&self, out: &mut Vec<A11yNode>) {
        use a11y::state::*;
        if let Some(p) = &self.popup {
            let owner = self.index_of(p.owner).map_or(crate::WINDOW_NODE, |i| self.elem_ids[i]);
            let mid = virt_id(owner, 6, 0);
            out.push(A11yNode { id: mid, parent: owner, role: Role::Menu, name: String::new(), value: String::new(), state: 0, layer: crate::ROOT_LAYER, bounds: p.rect, actions: 0 });
            let pad = self.look.theme.space[2] as i32;
            let mut y = p.rect.y + pad - p.scroll;
            for (k, it) in p.items.iter().enumerate() {
                let h = self.row_h(it) as i32;
                if !it.separator {
                    let mut state = 0;
                    if !it.disabled.is_empty() {
                        state |= DISABLED;
                    }
                    if it.checked == Some(true) {
                        state |= CHECKED;
                    }
                    if p.hi == Some(k) {
                        state |= FOCUSED;
                    }
                    let name = if it.disabled.is_empty() { it.label.clone() } else { alloc::format!("{} ({})", it.label, it.disabled) };
                    let bounds = Rect::new(p.rect.x + pad, y, p.rect.w.saturating_sub(2 * pad as u32), h as u32).intersect(&p.rect);
                    out.push(A11yNode { id: virt_id(owner, 7, k as u64), parent: mid, role: Role::MenuItem, name, value: it.shortcut.clone(), state, layer: crate::ROOT_LAYER, bounds, actions: if it.usable() { a11y::actions::ACTIVATE } else { 0 } });
                }
                y += h;
            }
        }
        if let Some(text) = self.tip_text()
            && let Some(i) = self.tip_shown.and_then(|k| self.index_of(k))
        {
            let id = self.elem_ids[i];
            out.push(A11yNode { id: virt_id(id, 8, 0), parent: id, role: Role::Tooltip, name: text.clone(), value: String::new(), state: 0, layer: crate::ROOT_LAYER, bounds: self.tip_rect(&text), actions: 0 });
        }
    }

    /// Akce pristupnosti na polozce otevrene nabidky.
    pub(crate) fn popup_a11y_action(&mut self, node: u32, action: u32, r: &mut Response<M>) -> bool {
        let Some(p) = &self.popup else { return false };
        let owner = self.index_of(p.owner).map_or(crate::WINDOW_NODE, |i| self.elem_ids[i]);
        let Some(k) = (0..p.items.len()).find(|&k| virt_id(owner, 7, k as u64) == node) else { return false };
        if action & a11y::actions::ACTIVATE != 0 {
            self.choose(k, r);
        }
        true
    }
}
