//! Kurzor aplikace, cela obrazovka na zadost aplikace a zamek ukazatele
//! (guiproto 5: `SET_CURSOR`, `WINDOW_STATE`, `POINTER_LOCK`; docs/cs/gui.md,
//! kapitola Kurzor a zamek ukazatele).
//!
//! - **Kurzor:** okno si nastavi tvar nad svym obsahem. Systemove tvary
//!   kresli kompozitor (cerna s bilym obrysem jako sipka a kurzory zmeny
//!   velikosti), vlastni obrazek nejvys 64 x 64 se micha pres alfu. Nad
//!   ramem, listou a pod systemovym dialogem plati systemovy kurzor.
//! - **Cela obrazovka:** vstup jen v reakci na fyzicky stisk v okne
//!   s fokusem (cislo gesta jako u schranky), navrat kdykoli; stejna cesta
//!   jako systemova zkratka (`fullscreen.rs`), takze funguji i odznak,
//!   tlacitko navratu a drzeny Esc.
//! - **Zamek ukazatele:** jen okno s fokusem a po fyzickem stisku. Kurzor
//!   se skryje a stoji, okno dostava `InputEvent::PointerRelative`. Nahore
//!   se na [`NOTICE_SHOW`] ukaze upozorneni systemu. Zamek vzdy zrusi
//!   klavesa Super (aplikaci nejde), ztrata fokusu, zavreni okna,
//!   systemovy dialog a prihlasovaci obrazovka; aplikace dostane
//!   `PointerLockState { locked: false }`.

use gfx::Canvas;
use guiproto::Rect;
use guiproto::cursor::{Cursor as AppCursor, Shape};
use guiproto::window::{InputEvent, ServerMsg};

use crate::input::Cursor;
use crate::render::{CURSOR_H, CURSOR_W, RESIZE_CURSOR, draw_cursor, draw_resize_cursor};
use crate::{ClientId, Comp};

/// Jak dlouho ukazovat upozorneni na zamek ukazatele.
pub const NOTICE_SHOW: u64 = 3_000_000_000;
/// Text upozorneni (system, aplikace ho nemuze zakryt).
pub const NOTICE: &str = "Aplikace převzala myš. Uvolnění: klávesa Super";

/// Ruka (odkaz): 1 = cerna, 2 = bila, aktivni bod na spicce ukazovacku.
const HAND: [&str; 19] = [
    ".....11.........",
    "....1221........",
    "....1221........",
    "....1221........",
    "....1221........",
    "....122111......",
    "....12212211....",
    "....122122121...",
    "11..1221221221..",
    "1221122222221221",
    "1222122222222221",
    ".122222222222221",
    "..12222222222221",
    "..1222222222221.",
    "...122222222221.",
    "...12222222221..",
    "....122222221...",
    "....122222221...",
    "....111111111...",
];
const HAND_HOT: (i32, i32) = (5, 0);

/// Body cerne cary systemovych tvaru s aktivnim bodem uprostred.
fn shape_points(s: Shape) -> alloc::vec::Vec<(i32, i32)> {
    use crate::render::resize_cursor_points as rp;
    let mut v = alloc::vec::Vec::new();
    match s {
        Shape::Text => {
            for t in -7..=7 {
                v.push((0, t));
            }
            v.extend([(-2, -8), (-1, -8), (1, -8), (2, -8), (-2, 8), (-1, 8), (1, 8), (2, 8)]);
        }
        Shape::Crosshair => {
            for t in -8..=8 {
                v.push((t, 0));
                v.push((0, t));
            }
        }
        Shape::Move => {
            v = rp(Cursor::Horizontal);
            v.extend(rp(Cursor::Vertical));
        }
        Shape::NotAllowed => {
            // Kruh (polomer 7) a sikma cara.
            for a in 0..64 {
                let (x, y) = circle(a, 7);
                v.push((x, y));
            }
            for t in -5..=5 {
                v.push((t, t));
            }
        }
        Shape::Wait | Shape::Progress => {
            // Presypaci hodiny: horni a dolni vodorovna, uhlopricky.
            for t in -5..=5 {
                v.push((t, -8));
                v.push((t, 8));
            }
            for t in -7..=7 {
                let x = t * 5 / 7;
                v.push((x, t));
                v.push((-x, t));
            }
        }
        _ => {}
    }
    v
}

/// Bod kruznice: krok `a` z 64, polomer `r` (celociselne, bez float).
fn circle(a: i32, r: i32) -> (i32, i32) {
    // Tabulka sin pro 16 kroku ctvrtiny (x1000).
    const S: [i32; 17] = [0, 98, 195, 290, 383, 471, 556, 634, 707, 773, 831, 882, 924, 957, 981, 995, 1000];
    let q = a / 16;
    let i = (a % 16) as usize;
    let (s, c) = (S[i], S[16 - i]);
    let (x, y) = match q {
        0 => (c, s),
        1 => (-s, c),
        2 => (-c, -s),
        _ => (s, -c),
    };
    ((x * r + 500 * x.signum()) / 1000, (y * r + 500 * y.signum()) / 1000)
}

fn draw_points(c: &mut Canvas<'_>, x: i32, y: i32, pts: &[(i32, i32)]) {
    for &(dx, dy) in pts {
        c.fill(Rect::new(x + dx - 1, y + dy - 1, 3, 3), gfx::rgb(255, 255, 255));
    }
    for &(dx, dy) in pts {
        c.fill(Rect::new(x + dx, y + dy, 1, 1), gfx::rgb(0, 0, 0));
    }
}

fn draw_bitmap(c: &mut Canvas<'_>, x: i32, y: i32, rows: &[&str]) {
    for (row, line) in rows.iter().enumerate() {
        for (col, ch) in line.bytes().enumerate() {
            let p = match ch {
                b'1' => gfx::rgb(0, 0, 0),
                b'2' => gfx::rgb(255, 255, 255),
                _ => continue,
            };
            c.fill(Rect::new(x + col as i32, y + row as i32, 1, 1), p);
        }
    }
}

/// Kurzor zmeny velikosti kompozitoru pro tvar aplikace.
fn resize_of(s: Shape) -> Option<Cursor> {
    match s {
        Shape::ResizeNwse => Some(Cursor::DiagDown),
        Shape::ResizeNesw => Some(Cursor::DiagUp),
        Shape::ResizeEw => Some(Cursor::Horizontal),
        Shape::ResizeNs => Some(Cursor::Vertical),
        _ => None,
    }
}

impl Comp {
    fn window_of(&self, c: ClientId, local: u32) -> Option<u32> {
        self.clients.get(&c).and_then(|cl| cl.windows.get(&local)).copied()
    }

    /// Kurzor podle polohy ukazatele (po zmene kurzoru okna nebo zamku).
    fn refresh_cursor(&mut self) {
        if self.plock.is_some() {
            return self.set_cursor(Cursor::Hidden);
        }
        let (px, py) = (self.input.px, self.input.py);
        let k = match self.hit(px, py) {
            _ if self.panel_rect().contains(px, py) => Cursor::Arrow,
            Some((g, crate::input::Zone::Content(..))) if self.dialogs.active().is_none() && self.windows.get(&g).is_some_and(|w| w.app_cursor.is_some()) => Cursor::App(g),
            h => h.map_or(Cursor::Arrow, |(_, z)| z.cursor()),
        };
        // Stejny druh, jiny obrazek: poskodit i tak.
        let r = self.cursor_rect();
        self.damage_top(r);
        self.set_cursor(k);
        let r = self.cursor_rect();
        self.damage_top(r);
    }

    /// `SET_CURSOR`: kurzor nad obsahem okna (`Default` = systemova sipka).
    pub(crate) fn set_app_cursor(&mut self, c: ClientId, local: u32, cursor: AppCursor) {
        let Some(g) = self.window_of(c, local) else {
            return self.error(c, guiproto::window::op::SET_CURSOR, guiproto::Error::InvalidArgs);
        };
        // Stare misto kurzoru pred zmenou obrazku (jinak by zustalo).
        let old = self.cursor_rect();
        self.damage_top(old);
        if let Some(w) = self.windows.get_mut(&g) {
            w.app_cursor = match cursor {
                AppCursor::Shape(Shape::Default) => None,
                k => Some(k),
            };
        }
        self.refresh_cursor();
    }

    /// `WINDOW_STATE`: cela obrazovka na zadost aplikace. Vstup jen po
    /// fyzickem stisku v okne s fokusem; jinak `ACCESS_DENIED`.
    pub(crate) fn window_state(&mut self, c: ClientId, local: u32, fullscreen: bool, gesture: u32, now: u64) {
        let Some(g) = self.window_of(c, local) else {
            return self.error(c, guiproto::window::op::WINDOW_STATE, guiproto::Error::InvalidArgs);
        };
        if self.is_fullscreen(g) == fullscreen {
            return;
        }
        if fullscreen && (self.focus != Some(g) || !self.clip_take_gesture(c, gesture, now)) {
            return self.error(c, guiproto::window::op::WINDOW_STATE, guiproto::Error::AccessDenied);
        }
        self.toggle_fullscreen(g, now);
    }

    fn lock_state(&mut self, g: u32, locked: bool) {
        if let Some(w) = self.windows.get(&g) {
            let (c, l) = (w.client, w.local);
            self.send(c, ServerMsg::PointerLockState { window: l, locked });
        }
    }

    /// `POINTER_LOCK`: zapnout (okno s fokusem, fyzicky stisk, zadny
    /// systemovy dialog) nebo zrusit. Odpoved vzdy `PointerLockState`.
    pub(crate) fn pointer_lock(&mut self, c: ClientId, local: u32, lock: bool, gesture: u32, now: u64) {
        let Some(g) = self.window_of(c, local) else {
            return self.error(c, guiproto::window::op::POINTER_LOCK, guiproto::Error::InvalidArgs);
        };
        let locked_here = self.plock.is_some_and(|(w, _)| w == g);
        if !lock {
            if locked_here {
                self.pointer_unlock();
            } else {
                self.lock_state(g, false);
            }
            return;
        }
        if locked_here {
            return self.lock_state(g, true);
        }
        let ok = self.focus == Some(g) && self.dialogs.active().is_none() && !self.greeter_visible() && self.clip_take_gesture(c, gesture, now);
        if !ok {
            return self.lock_state(g, false);
        }
        if let Some(r) = self.pointer_notice_rect() {
            self.damage_top(r);
        }
        self.plock = Some((g, now + NOTICE_SHOW));
        if let Some(r) = self.pointer_notice_rect() {
            self.damage_top(r);
        }
        self.set_cursor(Cursor::Hidden);
        self.lock_state(g, true);
    }

    /// Zrusi zamek ukazatele (systemem nebo aplikaci).
    pub(crate) fn pointer_unlock(&mut self) {
        if let Some(r) = self.pointer_notice_rect() {
            self.damage_top(r);
        }
        let Some((g, _)) = self.plock.take() else { return };
        self.lock_state(g, false);
        self.refresh_cursor();
    }

    /// Zamek patri oknu `g` (ztrata fokusu, zavreni): zrusit.
    pub(crate) fn pointer_unlock_window(&mut self, g: u32) {
        if self.plock.is_some_and(|(w, _)| w == g) {
            self.pointer_unlock();
        }
    }

    /// Pohyb pri zamku: relativni udalost oknu, kurzor stoji. `false` =
    /// zamek neplati (bezne zpracovani pohybu).
    pub(crate) fn pointer_locked_motion(&mut self, dx: i32, dy: i32, mods: u16, syn: bool, time: u64) -> bool {
        let Some((g, _)) = self.plock else { return false };
        if self.dialogs.active().is_some() || self.greeter_visible() {
            self.pointer_unlock();
            return false;
        }
        let Some(w) = self.windows.get(&g) else {
            self.plock = None;
            return false;
        };
        let (c, l) = (w.client, w.local);
        if dx != 0 || dy != 0 {
            self.send_input(c, l, InputEvent::PointerRelative { dx, dy }, mods, syn, time);
        }
        true
    }

    /// Obdelnik kurzoru aplikace v okne `g` (poskozeni).
    pub(crate) fn app_cursor_rect(&self, g: u32) -> Rect {
        let (px, py) = (self.input.px, self.input.py);
        let e = RESIZE_CURSOR as i32;
        let centered = Rect::new(px - e, py - e, 2 * e as u32 + 1, 2 * e as u32 + 1);
        match self.windows.get(&g).and_then(|w| w.app_cursor.as_ref()) {
            None | Some(AppCursor::Shape(Shape::Default)) => Rect::new(px, py, CURSOR_W, CURSOR_H),
            Some(AppCursor::Shape(Shape::Hidden)) => Rect::new(px, py, 0, 0),
            Some(AppCursor::Shape(Shape::Pointer)) => Rect::new(px - HAND_HOT.0, py - HAND_HOT.1, HAND[0].len() as u32, HAND.len() as u32),
            // Sipka a male hodiny vpravo dole.
            Some(AppCursor::Shape(Shape::Progress)) => Rect::new(px, py, CURSOR_W + 2 * e as u32, CURSOR_H + 2 * e as u32),
            Some(AppCursor::Shape(_)) => centered,
            Some(AppCursor::Custom(k)) => Rect::new(px - k.hot_x as i32, py - k.hot_y as i32, k.w as u32, k.h as u32),
        }
    }

    /// Kresli kurzor aplikace okna `g`.
    pub(crate) fn draw_app_cursor(&mut self, c: &mut Canvas<'_>, g: u32) {
        let (px, py) = (self.input.px, self.input.py);
        let Some(k) = self.windows.get(&g).and_then(|w| w.app_cursor.clone()) else {
            return draw_cursor(c, px, py);
        };
        match k {
            AppCursor::Shape(Shape::Default) => draw_cursor(c, px, py),
            AppCursor::Shape(Shape::Hidden) => {}
            AppCursor::Shape(Shape::Pointer) => draw_bitmap(c, px - HAND_HOT.0, py - HAND_HOT.1, &HAND),
            AppCursor::Shape(Shape::Progress) => {
                draw_cursor(c, px, py);
                let e = RESIZE_CURSOR as i32;
                let small: alloc::vec::Vec<(i32, i32)> = shape_points(Shape::Wait).into_iter().map(|(x, y)| (x / 2, y / 2)).collect();
                draw_points(c, px + CURSOR_W as i32 + e / 2, py + CURSOR_H as i32 + e / 2, &small);
            }
            AppCursor::Shape(s) => match resize_of(s) {
                Some(r) => draw_resize_cursor(c, px, py, r),
                None => draw_points(c, px, py, &shape_points(s)),
            },
            AppCursor::Custom(k) => {
                let (x0, y0) = (px - k.hot_x as i32, py - k.hot_y as i32);
                for y in 0..k.h as i32 {
                    for x in 0..k.w as i32 {
                        let p = k.pixels[(y * k.w as i32 + x) as usize];
                        if p >> 24 != 0 {
                            c.blend(Rect::new(x0 + x, y0 + y, 1, 1), p);
                        }
                    }
                }
            }
        }
    }

    /// Obdelnik upozorneni na zamek ukazatele (nahore uprostred), dokud
    /// se ukazuje.
    pub(crate) fn pointer_notice_rect(&self) -> Option<Rect> {
        let (_, until) = self.plock?;
        if self.input_now >= until {
            return None;
        }
        let t = &self.style.theme;
        let w = self.style.width(NOTICE, t.text.small) + 2 * t.space[4];
        let h = t.size.control_h;
        Some(Rect::new((self.w.saturating_sub(w) / 2) as i32, t.space[4] as i32, w, h))
    }

    /// Kresli upozorneni (systemovy material jako bubliny listy).
    pub(crate) fn draw_pointer_notice(&mut self, c: &mut Canvas<'_>) {
        let Some(r) = self.pointer_notice_rect() else { return };
        let t = self.style.theme.clone();
        let s = t.text.small;
        c.round_fill_bordered(r, t.radius.control, t.color.surface, t.color.border_strong, 1);
        let face = self.style.face(s);
        let base = r.y + face.baseline_in(s.size, r.h);
        face.draw_ellipsized(c, r.x + t.space[4] as i32, base, NOTICE, s.size, t.color.ink, r.w.saturating_sub(2 * t.space[4]));
    }

    /// Budik konce upozorneni.
    pub(crate) fn pointer_notice_due(&self) -> Option<u64> {
        self.plock.map(|(_, until)| until).filter(|&u| u > self.input_now)
    }

    /// Konec upozorneni: poskodit jeho misto.
    pub(crate) fn pointer_notice_timer(&mut self, now: u64) {
        if let Some((g, until)) = self.plock
            && now >= until
            && until != 0
        {
            let saved = self.input_now;
            self.input_now = until.saturating_sub(1);
            if let Some(r) = self.pointer_notice_rect() {
                self.damage_top(r);
            }
            self.input_now = saved;
            self.plock = Some((g, 0));
        }
    }
}
