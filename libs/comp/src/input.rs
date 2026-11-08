//! Rozdeleni vstupu: klavesnice do okna s fokusem (nebo do dialogu),
//! mys do okna pod ukazatelem; presun a zmena velikosti okna podle
//! titulku a deklarovanych oblasti - bez dotazu na aplikaci, takze to
//! funguje i se zaseknutou aplikaci.

use alloc::vec;

use guiproto::Rect;
use guiproto::window::{InputEvent, InputRecord, Mode, RegionKind, ServerMsg};
use inputproto::{Action, Button, Event, Record};

use crate::render::{CURSOR_H, CURSOR_W, PANEL_H};
use crate::scene::TITLE_H;
use crate::{ClientId, Comp, SYNC_TIMEOUT};

/// Sirka okraje pro zmenu velikosti.
const EDGE: i32 = 6;
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
    /// Okraj: zmena velikosti (vpravo, dole).
    Edge { right: bool, bottom: bool },
}

#[derive(Debug, Clone, Copy)]
enum Drag {
    Move { window: u32, dx: i32, dy: i32 },
    Resize { window: u32, right: bool, bottom: bool, start: Rect, px: i32, py: i32 },
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
}

impl InputState {
    pub fn new(w: u32, h: u32) -> Self {
        Self {
            px: w as i32 / 2,
            py: h as i32 / 2,
            drag: None,
            pointer_window: None,
            press_target: None,
            injecting: 0,
            untrusted: alloc::vec::Vec::new(),
            last_key: 0,
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

    fn cursor_rect(&self) -> Rect {
        Rect::new(self.input.px, self.input.py, CURSOR_W, CURSOR_H)
    }

    /// Okno a zona pod bodem (odshora).
    pub fn hit(&self, x: i32, y: i32) -> Option<(u32, Zone)> {
        for &gid in self.z.iter().rev() {
            let w = self.windows.get(&gid)?;
            if !w.mapped || w.minimized || !w.outer().contains(x, y) {
                continue;
            }
            if w.mode.has_frame() {
                for (i, b) in w.buttons().iter().enumerate() {
                    if b.contains(x, y) {
                        return Some((gid, Zone::Button(i)));
                    }
                }
                let cl = self.clients.get(&w.client)?;
                let bw = Comp::badge_width(&self.text, &Comp::badge_text(cl));
                if w.badge(bw).contains(x, y) {
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
                return Some((
                    gid,
                    Zone::Edge {
                        right: x as i64 >= o.right() - EDGE as i64,
                        bottom: y as i64 >= o.bottom() - EDGE as i64,
                    },
                ));
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
        self.damage(old);
        self.damage(new);
        let (px, py) = (self.input.px, self.input.py);
        if let Some(d) = self.input.drag {
            return self.drag_to(d, px, py, now);
        }
        if self.dialogs.active().is_some() {
            return;
        }
        let hit = self.hit(px, py);
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

    fn drag_to(&mut self, d: Drag, px: i32, py: i32, now: u64) {
        match d {
            Drag::Move { window, dx, dy } => {
                let (sw, sh) = (self.w as i32, self.h as i32);
                self.damage_window(window);
                if let Some(w) = self.windows.get_mut(&window) {
                    // Titulek nesmi zmizet z obrazovky.
                    w.rect.x = (px - dx).clamp(-(w.rect.w as i32) + 80, sw - 80);
                    let top = if w.mode == Mode::Standard { TITLE_H as i32 } else { 0 };
                    w.rect.y = (py - dy).clamp(top, sh - PANEL_H as i32 - 20);
                }
                self.damage_window(window);
            }
            Drag::Resize {
                window,
                right,
                bottom,
                start,
                px: sx,
                py: sy,
            } => {
                let Some(w) = self.windows.get_mut(&window) else { return };
                let nw = if right { (start.w as i32 + px - sx).max(w.min.0 as i32) as u32 } else { start.w };
                let nh = if bottom { (start.h as i32 + py - sy).max(w.min.1 as i32) as u32 } else { start.h };
                let (nw, nh) = (nw.min(guiproto::MAX_DIM), nh.min(guiproto::MAX_DIM));
                self.configure(window, nw, nh, now);
            }
        }
    }

    /// Pozada aplikaci o novou velikost (synchronizovana zmena: ram a obsah
    /// se ukazou zaroven po potvrzeni, nejpozdeji po `SYNC_TIMEOUT`).
    pub(crate) fn configure(&mut self, gid: u32, nw: u32, nh: u32, now: u64) {
        let focused = self.focus == Some(gid);
        let Some(w) = self.windows.get_mut(&gid) else { return };
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
        if self.dialogs.active().is_some() && pressed {
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
                let ev = InputEvent::Button {
                    code: button.code(),
                    pressed: false,
                    x: px - w.rect.x,
                    y: py - w.rect.y,
                };
                self.send_input(c, l, ev, mods, syn, time);
            }
            return;
        }
        if self.panel_rect().contains(px, py) {
            if button == Button::Left
                && let Some((g, _)) = self.panel_items().into_iter().find(|(_, r)| r.contains(px, py))
            {
                self.panel_click(g);
            } else if button == Button::Left && self.tasks_icon().is_some_and(|r| r.contains(px, py)) {
                // Spravce uloh do popredi (obnovit, kdyz je minimalizovany).
                let g = self.windows.iter().find(|(_, w)| self.clients.get(&w.client).is_some_and(|c| c.info.id == crate::render::TASKS_APP)).map(|(&g, _)| g);
                if let Some(g) = g {
                    if let Some(w) = self.windows.get_mut(&g) {
                        w.minimized = false;
                    }
                    self.damage_window(g);
                    self.set_focus(Some(g));
                }
            }
            return;
        }
        let Some((g, zone)) = self.hit(px, py) else { return };
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
            Zone::Edge { right, bottom } if button == Button::Left && (right || bottom) => {
                self.input.drag = Some(Drag::Resize {
                    window: g,
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
        let Some(w) = self.windows.get_mut(&g) else { return };
        if w.mode == Mode::Fullscreen {
            return;
        }
        let top = if w.mode == Mode::Standard { TITLE_H } else { 0 };
        let (nw, nh) = match w.restore.take() {
            Some(r) => {
                w.rect.x = r.x;
                w.rect.y = r.y;
                (r.w, r.h)
            }
            None => {
                w.restore = Some(w.rect);
                w.rect.x = 1;
                w.rect.y = top as i32 + 1;
                (sw - 2, sh - PANEL_H - top - 2)
            }
        };
        self.damage_window(g);
        self.configure(g, nw, nh, now);
    }
}
