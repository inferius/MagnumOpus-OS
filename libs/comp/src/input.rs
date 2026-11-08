//! Rozdeleni vstupu: klavesnice do okna s fokusem (nebo do dialogu),
//! mys do okna pod ukazatelem; presun a zmena velikosti okna podle
//! titulku a deklarovanych oblasti - bez dotazu na aplikaci, takze to
//! funguje i se zaseknutou aplikaci.

use alloc::vec;

use guiproto::Rect;
use guiproto::window::{InputEvent, InputRecord, Mode, RegionKind, ServerMsg};
use inputproto::{Action, Button, Event, Record};

use crate::render::{CURSOR_H, CURSOR_W, Hover};
use crate::{ClientId, Comp, SYNC_TIMEOUT};

/// Nove okno jine aplikace dostane fokus jen tehdy, kdyz uzivatel aspon
/// tak dlouho nepsal.
pub const FOCUS_GRACE: u64 = 1_500_000_000;
/// Jak dlouho ukazat odznak bezramoveho okna po najeti k okraji.
pub const BADGE_SHOW: u64 = 3_000_000_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Zone {
    Content(i32, i32),
    /// Systemove tlacitko: 0 zavrit, 1 maximalizovat, 2 minimalizovat.
    Button(usize),
    Badge,
    /// Tazenim se presouva okno.
    Title,
    /// Okraj nebo roh: zmena velikosti. Tah za levy nebo horni okraj
    /// posouva i polohu (protejsi okraj zustava).
    Edge { left: bool, top: bool, right: bool, bottom: bool },
}

impl Zone {
    /// Kurzor nad zonou.
    pub fn cursor(&self) -> Cursor {
        match *self {
            Zone::Edge { left, top, right, bottom } => match (left || right, top || bottom) {
                (true, false) => Cursor::Horizontal,
                (false, true) => Cursor::Vertical,
                (true, true) if (left && top) || (right && bottom) => Cursor::DiagDown,
                (true, true) => Cursor::DiagUp,
                (false, false) => Cursor::Arrow,
            },
            _ => Cursor::Arrow,
        }
    }
}

/// Tvar kurzoru (kresli kompozitor).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cursor {
    Arrow,
    /// Zmena sirky.
    Horizontal,
    /// Zmena vysky.
    Vertical,
    /// Roh vlevo nahore / vpravo dole.
    DiagDown,
    /// Roh vpravo nahore / vlevo dole.
    DiagUp,
}

#[derive(Debug, Clone, Copy)]
enum Drag {
    Move { window: u32, dx: i32, dy: i32 },
    Resize { window: u32, left: bool, top: bool, right: bool, bottom: bool, start: Rect, px: i32, py: i32 },
}

pub struct InputState {
    pub px: i32,
    pub py: i32,
    drag: Option<Drag>,
    pointer_window: Option<u32>,
    press_target: Option<u32>,
    /// Pocet zdroju umeleho vstupu (ukazuje se v panelu).
    pub injecting: u32,
    /// Zdroje (zarizeni) cekajici na overeni: jejich udalosti se ignoruji.
    pub untrusted: alloc::vec::Vec<u16>,
    /// Cas posledni klavesy (fokus novych oken).
    pub last_key: u64,
    /// Zvyrazneni pri najeti (tlacitko okna, polozka listy).
    pub hover: Option<Hover>,
    /// Tvar kurzoru (okraj okna: zmena velikosti).
    pub cursor: Cursor,
}

impl InputState {
    pub fn new(w: u32, h: u32) -> Self {
        Self {
            px: w as i32 / 2,
            py: h as i32 / 2,
            drag: None,
            pointer_window: None,
            press_target: None,
            cursor: Cursor::Arrow,
            injecting: 0,
            untrusted: alloc::vec::Vec::new(),
            last_key: 0,
            hover: None,
        }
    }

    pub(crate) fn window_gone(&mut self, gid: u32) {
        if self.pointer_window == Some(gid) {
            self.pointer_window = None;
        }
        if self.press_target == Some(gid) {
            self.press_target = None;
        }
        if matches!(self.drag, Some(Drag::Move { window, .. } | Drag::Resize { window, .. }) if window == gid) {
            self.drag = None;
        }
    }
}

impl Comp {
    pub(crate) fn send_input(&mut self, c: ClientId, local: u32, event: InputEvent, mods: u16, synthetic: bool, time: u64) {
        self.send(
            c,
            ServerMsg::Input {
                window: local,
                records: vec![InputRecord {
                    event,
                    mods,
                    synthetic,
                    time,
                }],
            },
        );
    }

    pub(crate) fn cursor_rect(&self) -> Rect {
        match self.input.cursor {
            Cursor::Arrow => Rect::new(self.input.px, self.input.py, CURSOR_W, CURSOR_H),
            // Kurzory zmeny velikosti maji aktivni bod uprostred.
            _ => {
                let e = crate::render::RESIZE_CURSOR as i32;
                Rect::new(self.input.px - e, self.input.py - e, 2 * e as u32 + 1, 2 * e as u32 + 1)
            }
        }
    }

    /// Zmena tvaru kurzoru: poskodit stare i nove misto.
    fn set_cursor(&mut self, k: Cursor) {
        if self.input.cursor == k {
            return;
        }
        let old = self.cursor_rect();
        self.input.cursor = k;
        let new = self.cursor_rect();
        self.damage_top(old);
        self.damage_top(new);
    }

    /// Stisk a tazeni pro diagnosticky snimek.
    pub(crate) fn input_summary(&self) -> alloc::string::String {
        let press = self.input.press_target.map_or(alloc::string::String::from("zadne"), |g| self.window_name(g));
        alloc::format!(
            "tlacitko drzene nad {press}, tazeni {}, zdroju umeleho vstupu {}",
            if self.input.drag.is_some() { "ano" } else { "ne" },
            self.input.injecting
        )
    }

    /// Okno a zona pod bodem (odshora).
    pub fn hit(&self, x: i32, y: i32) -> Option<(u32, Zone)> {
        for &gid in self.z.iter().rev() {
            let w = self.windows.get(&gid)?;
            if !w.mapped || w.minimized || !w.grab().contains(x, y) {
                continue;
            }
            // Neviditelny okraj vne okna: zmena velikosti.
            if !w.outer().contains(x, y) {
                let o = w.outer();
                // Rohy: v delce zaobleni od rohu.
                let c = w.geo.radius.max(w.geo.edge) as i64;
                let (x, y) = (x as i64, y as i64);
                return Some((
                    gid,
                    Zone::Edge { left: x < o.x as i64 + c, top: y < o.y as i64 + c, right: x >= o.right() - c, bottom: y >= o.bottom() - c },
                ));
            }
            if w.mode.has_frame() {
                // Cile tlacitek se muzou prekryvat: plati nejblizsi stred.
                if let Some(i) = w.button_at(x, y) {
                    return Some((gid, Zone::Button(i)));
                }
                let cl = self.clients.get(&w.client)?;
                if w.badge(self.badge_width(cl)).contains(x, y) {
                    return Some((gid, Zone::Badge));
                }
            }
            if w.mode == Mode::Standard && w.title_bar().is_some_and(|t| t.contains(x, y)) {
                return Some((gid, Zone::Title));
            }
            let (lx, ly) = (x - w.rect.x, y - w.rect.y);
            if matches!(w.mode, Mode::CustomTitle | Mode::Frameless) && w.rect.contains(x, y) {
                let inside = |k: RegionKind| w.regions.iter().any(|(r, kk)| *kk == k && r.contains(lx, ly));
                if inside(RegionKind::Resize) {
                    return Some((
                        gid,
                        Zone::Edge {
                            left: lx < w.rect.w as i32 / 2,
                            top: ly < w.rect.h as i32 / 2,
                            right: lx >= w.rect.w as i32 / 2,
                            bottom: ly >= w.rect.h as i32 / 2,
                        },
                    ));
                }
                if inside(RegionKind::Drag) && !inside(RegionKind::NoDrag) {
                    return Some((gid, Zone::Title));
                }
            }
            if w.rect.contains(x, y) {
                // Pravy dolni roh obsahu muze i menit velikost (bez ramu ne).
                return Some((gid, Zone::Content(lx, ly)));
            }
            if w.mode != Mode::Fullscreen {
                let o = w.outer();
                let e = w.geo.edge as i64;
                let (x, y) = (x as i64, y as i64);
                return Some((gid, Zone::Edge { left: x < o.x as i64 + e, top: y < o.y as i64 + e, right: x >= o.right() - e, bottom: y >= o.bottom() - e }));
            }
        }
        None
    }

    /// Zaznamy od `inputd` (kanal fokusu).
    pub fn input(&mut self, records: &[Record], now: u64) {
        self.input_now = now;
        for r in records {
            self.input_one(r, now);
        }
    }

    fn input_one(&mut self, r: &Record, now: u64) {
        if r.source != 0 && self.input.untrusted.contains(&r.source) {
            if let Event::Button { button, .. } = r.event {
                self.trace(r.time, format_args!("tlacitko {button:?} zahozeno: neovereny zdroj {}", r.source));
            }
            return;
        }
        let syn = r.is_synthetic();
        let mods = r.mods;
        match r.event {
            Event::Motion { dx, dy } => {
                let (x, y) = (self.input.px.saturating_add(dx), self.input.py.saturating_add(dy));
                self.pointer_to(x, y, mods, syn, r.time, now);
            }
            Event::Absolute { x, y } => {
                let x = (x as i64 * self.w as i64 / 65536) as i32;
                let y = (y as i64 * self.h as i64 / 65536) as i32;
                self.pointer_to(x, y, mods, syn, r.time, now);
            }
            Event::Button { button, action } => self.button(button, action == Action::Press, mods, syn, r.time, now),
            Event::Scroll { dx, dy } => {
                if self.dialogs.active().is_some() {
                    return;
                }
                if let Some((g, Zone::Content(..))) = self.hit(self.input.px, self.input.py) {
                    let w = &self.windows[&g];
                    let (c, l) = (w.client, w.local);
                    self.send_input(c, l, InputEvent::Scroll { dx, dy }, mods, syn, r.time);
                }
            }
            Event::Key { code, action, ch, .. } => {
                if !syn {
                    self.input.last_key = now;
                }
                // Diagnosticky snimek: jen fyzicka klavesnice, aplikaci nejde.
                // Esc zavre panel nad listou (aplikaci nejde).
                if code == inputproto::code::KEY_ESC && self.flyout.is_some() && !syn {
                    if action == Action::Press {
                        self.set_flyout(None);
                    }
                    return;
                }
                if self.snapshot_cfg.is_shortcut(code, mods) {
                    if !syn && action == Action::Press {
                        self.snapshot_request = Some(alloc::string::String::from("zkratka"));
                        self.trace(r.time, format_args!("zkratka diagnostickeho snimku"));
                    }
                    return;
                }
                if self.dialogs.active().is_some() {
                    if !syn {
                        self.dialog_key(code, action, ch, mods, now);
                    }
                    return;
                }
                if let Some((c, l)) = self.focused_target() {
                    let a = action.to_u32() as u8;
                    self.send_input(c, l, InputEvent::Key { code, action: a, ch }, mods, syn, r.time);
                }
            }
            Event::Text(ch) => {
                if self.dialogs.active().is_some() {
                    if !syn {
                        self.dialog_text(ch);
                    }
                    return;
                }
                if let Some((c, l)) = self.focused_target() {
                    self.send_input(c, l, InputEvent::Text(ch), mods, syn, r.time);
                }
            }
            Event::Injection { active } => {
                self.input.injecting = active;
                self.damage_panel();
            }
            Event::Dropped { .. } => {
                // Uvolneni mohlo byt ztraceno: ukoncit tazeni.
                self.input.drag = None;
                self.input.press_target = None;
            }
            // Kompozitor ztratil fokus vstupu (restart `inputd`, zmena
            // vlastnika): stisknute klavesy a tlacitka se povazuji za
            // uvolnene. Ukoncit tazeni a oknu s fokusem (a oknu se
            // stiskem) poslat `Focus(false)` - aplikace uvolni klavesy.
            // Po `Focus(true)` znovu `Focus(true)`.
            Event::Focus(false) => {
                self.input.drag = None;
                let pressed = self.input.press_target.take();
                let target = self.focused_target();
                if let Some(g) = pressed
                    && let Some(w) = self.windows.get(&g)
                    && Some((w.client, w.local)) != target
                {
                    let (c, l) = (w.client, w.local);
                    self.send_input(c, l, InputEvent::Focus(false), mods, false, r.time);
                }
                if let Some((c, l)) = target {
                    self.send_input(c, l, InputEvent::Focus(false), mods, false, r.time);
                }
            }
            Event::Focus(true) => {
                if let Some((c, l)) = self.focused_target() {
                    self.send_input(c, l, InputEvent::Focus(true), mods, false, r.time);
                }
            }
            Event::SecureAttention => self.secure_attention(now),
            Event::NewDevice { source, state } => self.new_device(source, state),
            _ => {}
        }
    }

    fn focused_target(&self) -> Option<(ClientId, u32)> {
        let w = self.windows.get(&self.focus?)?;
        if w.minimized {
            return None;
        }
        Some((w.client, w.local))
    }

    fn pointer_to(&mut self, x: i32, y: i32, mods: u16, syn: bool, time: u64, now: u64) {
        let old = self.cursor_rect();
        self.input.px = x.clamp(0, self.w as i32 - 1);
        self.input.py = y.clamp(0, self.h as i32 - 1);
        let new = self.cursor_rect();
        self.damage_top(old);
        self.damage_top(new);
        let (px, py) = (self.input.px, self.input.py);
        if let Some(d) = self.input.drag {
            return self.drag_to(d, px, py, now);
        }
        if self.dialogs.active().is_some() {
            return self.set_hover(None);
        }
        let hit = self.hit(px, py);
        self.update_hover(px, py, hit);
        let target = match hit {
            Some((g, Zone::Content(..))) => Some(g),
            _ => None,
        };
        if self.input.pointer_window != target
            && let Some(old) = self.input.pointer_window
            && let Some(w) = self.windows.get(&old)
        {
            let (c, l) = (w.client, w.local);
            self.send_input(c, l, InputEvent::PointerLeave, mods, syn, time);
        }
        self.input.pointer_window = target;
        // Stisknute tlacitko: pohyb jde dal oknu, ktere dostalo stisk.
        let dest = self.input.press_target.or(target);
        if let Some(g) = dest
            && let Some(w) = self.windows.get(&g)
        {
            let (c, l, lx, ly) = (w.client, w.local, px - w.rect.x, py - w.rect.y);
            self.send_input(c, l, InputEvent::PointerMove { x: lx, y: ly }, mods, syn, time);
        }
        // Bezramove okno: odznak pri najeti k hornimu okraji.
        if let Some((g, _)) = hit
            && let Some(w) = self.windows.get_mut(&g)
            && matches!(w.mode, Mode::Frameless | Mode::Fullscreen)
            && py - w.rect.y < 6
        {
            let was = w.badge_until > now;
            w.badge_until = now + BADGE_SHOW;
            if !was {
                self.damage_window(g);
            }
        }
    }

    fn update_hover(&mut self, px: i32, py: i32, hit: Option<(u32, Zone)>) {
        let h = if self.panel_rect().contains(px, py) {
            let bar = self.bar_layout();
            if bar.launcher.contains(px, py) {
                Some(Hover::Launcher)
            } else if let Some((g, _)) = bar.apps.iter().find(|(_, r)| r.contains(px, py)) {
                Some(Hover::BarApp(*g))
            } else if bar.tasks.is_some_and(|r| r.contains(px, py)) {
                Some(Hover::Tasks)
            } else {
                None
            }
        } else {
            match hit {
                Some((g, Zone::Button(i))) => Some(Hover::Button(g, i)),
                _ => None,
            }
        };
        self.set_hover(h);
        let k = if self.panel_rect().contains(px, py) { Cursor::Arrow } else { hit.map_or(Cursor::Arrow, |(_, z)| z.cursor()) };
        self.set_cursor(k);
    }

    /// Zmena zvyrazneni: poskodit puvodni i nove misto.
    pub(crate) fn set_hover(&mut self, h: Option<Hover>) {
        if self.input.hover == h {
            return;
        }
        for x in [self.input.hover, h].into_iter().flatten() {
            match x {
                Hover::Button(g, i) => {
                    if let Some(w) = self.windows.get(&g) {
                        let r = w.button_hits()[i];
                        self.damage(r);
                    }
                }
                Hover::Tasks => {
                    self.damage_panel();
                    // Popisek nad ikonou uloh.
                    if let Some(r) = self.tasks_tip_rect() {
                        self.damage(r);
                    }
                }
                Hover::BarApp(_) | Hover::Launcher => self.damage_panel(),
            }
        }
        self.input.hover = h;
    }

    fn drag_to(&mut self, d: Drag, px: i32, py: i32, now: u64) {
        match d {
            Drag::Move { window, dx, dy } => {
                let (sw, sh) = (self.w as i32, self.h as i32);
                let reserve = self.bar_reserve() as i32;
                self.damage_window(window);
                if let Some(w) = self.windows.get_mut(&window) {
                    // Titulek nesmi zmizet z obrazovky.
                    w.rect.x = (px - dx).clamp(-(w.rect.w as i32) + 80, sw - 80);
                    let top = if w.mode == Mode::Standard { (w.geo.title_h + w.geo.border) as i32 } else { 0 };
                    w.rect.y = (py - dy).clamp(top, sh - reserve - top.max(20));
                }
                self.damage_window(window);
            }
            Drag::Resize {
                window,
                left,
                top,
                right,
                bottom,
                start,
                px: sx,
                py: sy,
            } => {
                let Some(w) = self.windows.get(&window) else { return };
                let (dx, dy) = (px - sx, py - sy);
                let nw = if right { start.w as i32 + dx } else if left { start.w as i32 - dx } else { start.w as i32 };
                let nh = if bottom { start.h as i32 + dy } else if top { start.h as i32 - dy } else { start.h as i32 };
                // Titulek nesmi odjet nad obrazovku (tah za horni okraj).
                let title = if w.mode == Mode::Standard { (w.geo.title_h + w.geo.border) as i32 } else { 0 };
                let nh = if top { nh.min(start.bottom() as i32 - title) } else { nh };
                let nw = (nw.max(w.min.0 as i32) as u32).min(guiproto::MAX_DIM);
                let nh = (nh.max(w.min.1 as i32) as u32).min(guiproto::MAX_DIM);
                self.configure(window, nw, nh, now);
                // Levy a horni okraj: protejsi okraj zustava na miste.
                if let Some(w) = self.windows.get_mut(&window) {
                    w.anchor = (left.then_some(start.right() as i32), top.then_some(start.bottom() as i32));
                }
            }
        }
    }

    /// Pozada aplikaci o novou velikost (synchronizovana zmena: ram a obsah
    /// se ukazou zaroven po potvrzeni, nejpozdeji po `SYNC_TIMEOUT`).
    pub(crate) fn configure(&mut self, gid: u32, nw: u32, nh: u32, now: u64) {
        let focused = self.focus == Some(gid);
        let Some(w) = self.windows.get_mut(&gid) else { return };
        // Kotvu nastavi jen tah za levy / horni okraj (po tomto volani).
        w.anchor = (None, None);
        if w.configure.is_some_and(|(_, cw, ch, _)| cw == nw && ch == nh) || (w.configure.is_none() && w.rect.w == nw && w.rect.h == nh) {
            return;
        }
        w.serial = w.serial.wrapping_add(1);
        let serial = w.serial;
        // Termin bezi od prvni nepotvrzene zadosti.
        let deadline = w.configure.map_or(now + SYNC_TIMEOUT, |(_, _, _, d)| d);
        w.configure = Some((serial, nw, nh, deadline));
        let (c, l, state) = (w.client, w.local, w.state(focused));
        self.send(
            c,
            ServerMsg::Configure {
                window: l,
                w: nw,
                h: nh,
                state,
                serial,
            },
        );
    }

    fn button(&mut self, button: Button, pressed: bool, mods: u16, syn: bool, time: u64, now: u64) {
        let (px, py) = (self.input.px, self.input.py);
        let act = if pressed { "stisk" } else { "pusteni" };
        if self.dialogs.active().is_some() && pressed {
            let why = if syn { "zahozen (umely vstup)" } else { "do systemoveho dialogu" };
            self.trace(time, format_args!("{button:?} {act} na {px},{py}: modalni dialog, {why}"));
            if !syn && button == Button::Left {
                self.dialog_click(px, py, now);
            }
            return;
        }
        // Uvolneni vzdy ukonci tazeni a dojde oknu se stiskem (i kdyz mezi
        // tim vyskocil dialog).
        if !pressed {
            self.input.drag = None;
            if let Some(g) = self.input.press_target.take()
                && let Some(w) = self.windows.get(&g)
            {
                let (c, l) = (w.client, w.local);
                let (x, y) = (px - w.rect.x, py - w.rect.y);
                let ev = InputEvent::Button {
                    code: button.code(),
                    pressed: false,
                    x,
                    y,
                };
                self.send_input(c, l, ev, mods, syn, time);
                let n = self.window_name(g);
                self.trace(time, format_args!("{button:?} {act} na {px},{py} -> {n} obsah {x},{y} (okno se stiskem)"));
            } else {
                self.trace(time, format_args!("{button:?} {act} na {px},{py}: zadne okno nedostalo stisk, nic neposlano"));
            }
            return;
        }
        // Otevreny panel nad listou: klik do nej nic nedela, klik jinam ho
        // zavre (a jde dal).
        if self.flyout_rect().is_some_and(|r| r.contains(px, py)) {
            let f = self.flyout;
            self.trace(time, format_args!("{button:?} {act} na {px},{py}: panel {f:?}"));
            return;
        }
        if self.panel_rect().contains(px, py) {
            let bar = self.bar_layout();
            let icon = [
                (bar.net, crate::flyout::Flyout::Net),
                (bar.volume, crate::flyout::Flyout::Volume),
                (Some(bar.bell), crate::flyout::Flyout::Bell),
            ]
            .into_iter()
            .find(|(r, _)| r.is_some_and(|r| r.contains(px, py)))
            .map(|(_, f)| f);
            self.trace(time, format_args!("{button:?} {act} na {px},{py}: lista{}", icon.map_or(alloc::string::String::new(), |f| alloc::format!(", ikona {f:?}"))));
            if button != Button::Left {
                return;
            }
            if let Some(f) = icon {
                self.toggle_flyout(f);
            } else if let Some((g, _)) = self.panel_items().into_iter().find(|(_, r)| r.contains(px, py)) {
                self.set_flyout(None);
                self.panel_click(g);
            } else if self.tasks_icon().is_some_and(|r| r.contains(px, py)) {
                // Spravce uloh do popredi (obnovit, kdyz je minimalizovany);
                // kdyz nebezi, panel s poctem uloh.
                let g = self.windows.iter().find(|(_, w)| self.clients.get(&w.client).is_some_and(|c| c.info.id == crate::render::TASKS_APP)).map(|(&g, _)| g);
                if let Some(g) = g {
                    self.set_flyout(None);
                    if let Some(w) = self.windows.get_mut(&g) {
                        w.minimized = false;
                    }
                    self.damage_window(g);
                    self.set_focus(Some(g));
                } else {
                    self.toggle_flyout(crate::flyout::Flyout::Tasks);
                }
            } else {
                self.set_flyout(None);
            }
            return;
        }
        if self.flyout.is_some() {
            self.set_flyout(None);
        }
        let Some((g, zone)) = self.hit(px, py) else {
            self.trace(time, format_args!("{button:?} {act} na {px},{py}: mimo okna (plocha)"));
            return;
        };
        let n = self.window_name(g);
        self.trace(time, format_args!("{button:?} {act} na {px},{py} -> {n} zona {zone:?}{}", if self.focus == Some(g) { "" } else { ", fokus na okno" }));
        self.set_focus(Some(g));
        let w = &self.windows[&g];
        let (c, l, rect) = (w.client, w.local, w.rect);
        match zone {
            Zone::Content(x, y) => {
                self.input.press_target = Some(g);
                let ev = InputEvent::Button {
                    code: button.code(),
                    pressed: true,
                    x,
                    y,
                };
                self.send_input(c, l, ev, mods, syn, time);
            }
            Zone::Button(0) if button == Button::Left => self.send(c, ServerMsg::Close { window: l }),
            Zone::Button(1) if button == Button::Left => self.toggle_maximize(g, now),
            Zone::Button(2) if button == Button::Left => self.minimize(g),
            Zone::Title if button == Button::Left => {
                self.input.drag = Some(Drag::Move {
                    window: g,
                    dx: px - rect.x,
                    dy: py - rect.y,
                });
            }
            Zone::Edge { left, top, right, bottom } if button == Button::Left && (left || top || right || bottom) => {
                self.input.drag = Some(Drag::Resize {
                    window: g,
                    left,
                    top,
                    right,
                    bottom,
                    start: rect,
                    px,
                    py,
                });
            }
            _ => {}
        }
    }

    fn panel_click(&mut self, g: u32) {
        let Some(w) = self.windows.get_mut(&g) else { return };
        if w.minimized {
            w.minimized = false;
            self.damage_window(g);
            self.set_focus(Some(g));
        } else if self.focus == Some(g) {
            self.minimize(g);
        } else {
            self.set_focus(Some(g));
        }
        self.damage_panel();
    }

    pub(crate) fn minimize(&mut self, g: u32) {
        if let Some(w) = self.windows.get_mut(&g) {
            w.minimized = true;
        }
        self.damage_window(g);
        if self.focus == Some(g) {
            let next = self.z.iter().rev().copied().find(|&x| x != g && self.windows.get(&x).is_some_and(|w| w.mapped && !w.minimized));
            self.set_focus(next);
        }
        self.damage_panel();
    }

    pub(crate) fn toggle_maximize(&mut self, g: u32, now: u64) {
        let (sw, sh) = (self.w, self.h);
        let reserve = self.bar_reserve();
        let Some(w) = self.windows.get_mut(&g) else { return };
        if w.mode == Mode::Fullscreen {
            return;
        }
        let b = w.geo.border;
        let top = if w.mode == Mode::Standard { w.geo.title_h } else { 0 };
        // Poskodit puvodni misto (i se stinem), nove se poskodi po zmene.
        let before = w.visual();
        let (nw, nh) = match w.restore.take() {
            Some(r) => {
                w.rect.x = r.x;
                w.rect.y = r.y;
                (r.w, r.h)
            }
            None => {
                w.restore = Some(w.rect);
                w.rect.x = b as i32;
                w.rect.y = (top + b) as i32;
                (sw - 2 * b, sh - reserve - top - 2 * b)
            }
        };
        self.damage(before);
        self.damage_window(g);
        self.configure(g, nw, nh, now);
    }
}
