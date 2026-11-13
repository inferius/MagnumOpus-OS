//! Cela obrazovka jako systemova funkce pro kazde okno (`docs/cs/gui.md`,
//! Cela obrazovka): zkratka Super+Shift+F (F11 patri aplikacim), nabidka
//! pri najeti na tlacitko maximalizace, polozka palety. Aplikace dostane
//! jen CONFIGURE s velikosti obrazovky (nic nemusi umet).
//!
//! Okno prejde do rezimu `FULLSCREEN` (puvodni rezim, obdelnik
//! a maximalizace se ulozi a pri navratu obnovi). Lista se skryje.
//! Navrat: stejna zkratka, nebo tlacitko "Opustit celou obrazovku" vedle
//! odznaku duvery, ktery se ukaze pri najeti k hornimu okraji. Bezpecna
//! zkratka a systemove dialogy funguji vzdy (kresli se nad vsim).
//!
//! Nerusit: pri cele obrazovce automaticky zapnuto (`dnd_fullscreen`,
//! vypnout prikazem `dnd-fullscreen off`); oznameni se pak tise ukladaji.

use alloc::string::String;
use alloc::vec::Vec;

use gfx::Canvas;
use guiproto::Rect;
use guiproto::window::Mode;
use inputproto::code::KEY_F;
use inputproto::mods;

use crate::Comp;

/// Jak dlouho drzet ukazatel nad tlacitkem maximalizace, nez se ukaze
/// nabidka.
pub const MAX_MENU_DELAY: u64 = 400_000_000;
/// Volby nabidky u tlacitka maximalizace.
pub const MAX_MENU: [&str; 4] = ["Maximalizovat", "Celá obrazovka", "Připnout vlevo", "Připnout vpravo"];
pub const EXIT_LABEL: &str = "Ukončit celou obrazovku";
/// Zona u horniho okraje, ktera vysune odznak a tlacitko navratu.
pub const EDGE_ZONE: i32 = 24;
/// Jak dlouho zustane vysunute po odjeti ukazatele.
pub const EXIT_SHOW: u64 = 2_500_000_000;
/// Drzeni Esc, ktere vzdy ukonci celou obrazovku.
pub const ESC_HOLD: u64 = 1_000_000_000;

/// Ulozeny stav okna pred celou obrazovkou: rezim, obdelnik, maximalizace.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Saved {
    pub mode: Mode,
    pub rect: Rect,
    pub restore: Option<Rect>,
}

impl Comp {
    /// Zkratka Super+Shift+F (jen fyzicka klavesnice).
    pub(crate) fn is_fullscreen_shortcut(code: u16, m: u16) -> bool {
        code == KEY_F && m & mods::META != 0 && m & mods::SHIFT != 0 && m & (mods::CTRL | mods::ALT) == 0
    }

    /// Je okno `g` v systemove cele obrazovce?
    pub fn is_fullscreen(&self, g: u32) -> bool {
        self.windows.get(&g).is_some_and(|w| w.fs_saved.is_some())
    }

    /// Okno s fokusem pres celou obrazovku (lista skryta, nerusit).
    pub fn fullscreen_active(&self) -> bool {
        self.focus.and_then(|g| self.windows.get(&g)).is_some_and(|w| w.mapped && !w.minimized && w.mode == Mode::Fullscreen)
    }

    /// Nerusit: zapnuto automaticky pri cele obrazovce (lze vypnout).
    pub fn do_not_disturb(&self) -> bool {
        self.dnd_fullscreen && self.fullscreen_active()
    }

    pub fn set_dnd_fullscreen(&mut self, on: bool) {
        self.dnd_fullscreen = on;
        self.damage_panel();
    }

    /// Prepne celou obrazovku okna `g`.
    pub fn toggle_fullscreen(&mut self, g: u32, now: u64) {
        self.toggle_fullscreen_state(g, now);
        self.context_of_window_changed(g);
    }

    fn toggle_fullscreen_state(&mut self, g: u32, now: u64) {
        let (sw, sh) = (self.w, self.h);
        let Some(w) = self.windows.get_mut(&g) else { return };
        let before = w.visual();
        match w.fs_saved.take() {
            Some(s) => {
                w.mode = s.mode;
                w.restore = s.restore;
                w.rect.x = s.rect.x;
                w.rect.y = s.rect.y;
                let (nw, nh) = (s.rect.w, s.rect.h);
                self.damage_all();
                self.configure(g, nw, nh, now);
            }
            None => {
                // Aplikace sama v rezimu FULLSCREEN: nic neukladat.
                if w.mode == Mode::Fullscreen {
                    return;
                }
                w.fs_saved = Some(Saved { mode: w.mode, rect: w.rect, restore: w.restore.take() });
                w.mode = Mode::Fullscreen;
                w.rect.x = 0;
                w.rect.y = 0;
                self.damage(before);
                self.set_flyout(None);
                self.set_focus(Some(g));
                self.damage_all();
                self.configure(g, sw, sh, now);
            }
        }
    }

    /// Pripne okno k leve nebo prave polovine pracovni plochy.
    pub(crate) fn snap(&mut self, g: u32, right: bool, now: u64) {
        let (sw, sh, reserve) = (self.w, self.h, self.bar_reserve());
        let Some(w) = self.windows.get_mut(&g) else { return };
        if w.mode == Mode::Fullscreen {
            return;
        }
        let before = w.visual();
        let b = w.geo.border;
        let top = if w.mode == Mode::Standard { w.geo.title_h } else { 0 };
        if w.restore.is_none() {
            w.restore = Some(w.rect);
        }
        let half = sw / 2;
        w.rect.x = if right { (half + b) as i32 } else { b as i32 };
        w.rect.y = (top + b) as i32;
        let (nw, nh) = (half - 2 * b, sh - reserve - top - 2 * b);
        self.damage(before);
        self.damage_window(g);
        self.configure(g, nw, nh, now);
    }

    /// Tlacitko "Opustit celou obrazovku" vedle odznaku u horniho okraje
    /// (jen kdyz je odznak videt).
    pub(crate) fn fs_exit_rect(&self, g: u32, now: u64) -> Option<Rect> {
        let w = self.windows.get(&g)?;
        if w.fs_saved.is_none() || w.badge_until <= now {
            return None;
        }
        let cl = self.clients.get(&w.client)?;
        let b = w.badge(self.badge_width(cl));
        let t = &self.style.theme;
        let bw = self.style.width(EXIT_LABEL, t.text.small) + 2 * t.space[4];
        Some(Rect::new(b.right() as i32 + t.space[3] as i32, b.y, bw, b.h))
    }

    /// Esc stisknut (cas) / pusten: drzeni [`ESC_HOLD`] ukonci celou
    /// obrazovku okna s fokusem. Klavesa jde dal i aplikaci.
    pub(crate) fn fs_esc(&mut self, down: bool, now: u64) {
        let fs = self.focus.is_some_and(|g| self.is_fullscreen(g));
        self.esc_since = match (down, fs) {
            (true, true) => self.esc_since.or(Some(now)),
            _ => None,
        };
    }

    /// Budik drzeneho Esc.
    pub(crate) fn fs_esc_due(&self) -> Option<u64> {
        self.esc_since.map(|t| t + ESC_HOLD)
    }

    pub(crate) fn fs_esc_timer(&mut self, now: u64) {
        if let Some(t) = self.esc_since
            && now >= t + ESC_HOLD
        {
            self.esc_since = None;
            if let Some(g) = self.focus.filter(|&g| self.is_fullscreen(g)) {
                self.toggle_fullscreen(g, now);
            }
        }
    }

    /// Nabidka u tlacitka maximalizace: obdelnik a radky.
    pub(crate) fn max_menu_rect(&self) -> Option<Rect> {
        let g = self.max_menu?;
        let w = self.windows.get(&g)?;
        let b = w.buttons()[1];
        let t = &self.style.theme;
        let s = t.text.body;
        let mw = MAX_MENU.iter().map(|l| self.style.width(l, s)).max().unwrap_or(0) + 2 * t.space[4];
        let rh = t.size.control_h;
        let h = MAX_MENU.len() as u32 * rh + 2 * t.space[2];
        let x = (b.x + b.w as i32 - mw as i32).clamp(0, (self.w - mw) as i32);
        Some(Rect::new(x, b.bottom() as i32 + t.space[2] as i32, mw, h))
    }

    pub(crate) fn max_menu_row(&self, y: i32) -> Option<usize> {
        let r = self.max_menu_rect()?;
        let t = &self.style.theme;
        let i = (y - r.y - t.space[2] as i32).div_euclid(t.size.control_h as i32);
        (0..MAX_MENU.len() as i32).contains(&i).then_some(i as usize)
    }

    pub(crate) fn set_max_menu(&mut self, g: Option<u32>) {
        if let Some(r) = self.max_menu_rect() {
            self.damage(r);
        }
        self.max_menu = g;
        if let Some(r) = self.max_menu_rect() {
            self.damage(r);
        }
    }

    /// Najeti na tlacitko maximalizace (spusti odpocet nabidky) nebo
    /// odjeti (zavre ji, kdyz ukazatel neni nad ni).
    pub(crate) fn max_hover(&mut self, over: Option<u32>, px: i32, py: i32, now: u64) {
        let in_menu = self.max_menu_rect().is_some_and(|r| r.contains(px, py));
        match over {
            Some(g) if self.max_hover_since.is_none_or(|(h, _)| h != g) => self.max_hover_since = Some((g, now)),
            Some(_) => {}
            None if !in_menu => {
                self.max_hover_since = None;
                if self.max_menu.is_some() {
                    self.set_max_menu(None);
                }
            }
            None => {}
        }
    }

    /// Cas, kdy se ma ukazat nabidka (budik kompozitoru).
    pub(crate) fn max_menu_due(&self) -> Option<u64> {
        let (_, since) = self.max_hover_since?;
        self.max_menu.is_none().then_some(since + MAX_MENU_DELAY)
    }

    /// Budik: odpocet nabidky vyprsel.
    pub(crate) fn max_menu_timer(&mut self, now: u64) {
        if let Some((g, since)) = self.max_hover_since
            && self.max_menu.is_none()
            && now >= since + MAX_MENU_DELAY
        {
            self.set_max_menu(Some(g));
        }
    }

    /// Klik do nabidky u maximalizace (`true` = vyrizeno).
    pub(crate) fn max_menu_click(&mut self, x: i32, y: i32, now: u64) -> bool {
        let Some(g) = self.max_menu else { return false };
        if !self.max_menu_rect().is_some_and(|r| r.contains(x, y)) {
            self.set_max_menu(None);
            self.max_hover_since = None;
            return false;
        }
        let row = self.max_menu_row(y);
        self.set_max_menu(None);
        self.max_hover_since = None;
        match row {
            Some(0) => self.toggle_maximize(g, now),
            Some(1) => self.toggle_fullscreen(g, now),
            Some(2) => self.snap(g, false, now),
            Some(3) => self.snap(g, true, now),
            _ => {}
        }
        true
    }

    /// Kresli nabidku u maximalizace (vrstva nad okny).
    pub(crate) fn draw_max_menu(&mut self, c: &mut Canvas<'_>) {
        let Some(r) = self.max_menu_rect() else { return };
        let t = self.style.theme.clone();
        let pop = t.shadow.popover.clone();
        self.style.draw_shadow(c, &pop, r, t.radius.card, Rect::default());
        c.round_fill_bordered(r, t.radius.card, t.color.surface, t.color.border_strong, 1);
        let s = t.text.body;
        let rh = t.size.control_h;
        let hover = self.max_menu_row(self.input.py).filter(|_| r.contains(self.input.px, self.input.py));
        let mut y = r.y + t.space[2] as i32;
        for (i, l) in MAX_MENU.iter().enumerate() {
            let row = Rect::new(r.x + t.space[1] as i32, y, r.w - 2 * t.space[1], rh);
            if hover == Some(i) {
                c.round_fill(row, t.radius.control, t.color.surface_3);
            }
            let face = self.style.face(s);
            let base = y + face.baseline_in(s.size, rh);
            face.draw(c, r.x + t.space[4] as i32, base, l, s.size, t.color.ink);
            y += rh as i32;
        }
    }

    /// Radky stavu nerusit pro panel zvonku.
    pub(crate) fn dnd_lines(&self) -> Vec<String> {
        if self.do_not_disturb() {
            alloc::vec![String::from("Nerušit: zapnuto (celá obrazovka)"), String::from("Oznámení se uloží a ukážou po návratu")]
        } else {
            alloc::vec![String::from("Žádná oznámení")]
        }
    }
}
