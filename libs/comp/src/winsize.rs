//! Kontext okna (`WINDOW_CONTEXT`) a pozadovana velikost (`WINDOW_SIZE`).
//!
//! Kontext dostane kazde okno zvlast (vystup, cela obrazovka, pracovni
//! plocha, skala, frekvence): pred prvnim `CONFIGURE` a po kazde zmene
//! (s vyssi generaci). Aplikace podle sirky sveho okna voli rozvrzeni
//! (tridy sirky v `libs/ui`), ne podle obrazovky.
//!
//! Pozadovana velikost pred prvnim commitem urci velikost pri otevreni
//! (okno se ukaze rovnou spravne, bez druheho `CONFIGURE` a probliknuti),
//! pozdeji je to zadost o zmenu. Maximalizovane okno a okno na cele
//! obrazovce si ji zapamatuji jako velikost po obnoveni.

use guiproto::Rect;
use guiproto::window::{Mode, ServerMsg};

use crate::{ClientId, Comp};

/// Okraj od pravého kraje obrazovky, ktery okno bez stropu nechava volny.
const SIDE_GAP: u32 = 40;

impl Comp {
    /// Obnovovaci frekvence vystupu v mHz (0 = neznama).
    fn refresh_mhz(&self) -> u32 {
        1_000_000_000_000u64.checked_div(self.period()).map_or(0, |v| v as u32)
    }

    /// Posle kontext okna `gid` jeho klientovi.
    pub(crate) fn send_context(&mut self, gid: u32) {
        let Some(w) = self.windows.get(&gid) else { return };
        let (c, local) = (w.client, w.local);
        let m = ServerMsg::WindowContext {
            window: local,
            generation: self.ctx_generation,
            output: 0,
            screen: Rect::new(0, 0, self.w, self.h),
            work_area: self.work_area(),
            scale: 100,
            refresh_mhz: self.refresh_mhz(),
            ext: self.context_ext(gid),
        };
        self.send(c, m);
    }

    /// Poloha obsahu oken pro aplikace (`WINDOW_PLACE`): poslat oknum, ktera
    /// se od posledni zpravy posunula nebo zmenila velikost; behem tahu
    /// mysi az po jeho konci. Vola se pred skladanim snimku.
    pub(crate) fn places_changed(&mut self) {
        let dragging = match self.input.drag {
            Some(crate::input::Drag::Move { window, .. }) | Some(crate::input::Drag::Resize { window, .. }) => Some(window),
            _ => None,
        };
        let ids: alloc::vec::Vec<u32> = self.windows.iter().filter(|(g, w)| w.mapped && w.rect != w.placed && Some(**g) != dragging).map(|(g, _)| *g).collect();
        for g in ids {
            let Some(w) = self.windows.get_mut(&g) else { continue };
            w.placed = w.rect;
            let (c, local, rect) = (w.client, w.local, w.rect);
            self.send(c, ServerMsg::WindowPlace { window: local, rect });
        }
    }

    /// Rozsireni kontextu okna `gid`: pocitac s mysi a klavesnici (tablet
    /// a TV dodaji sluzby zarizeni, az budou), dlazdice podle stavu okna,
    /// nejmensi cil z tokenu `size.hit_min_fine`.
    fn context_ext(&self, gid: u32) -> guiproto::context::ContextExt {
        use guiproto::context::{ContextExt, Tile};
        let tile = match self.windows.get(&gid) {
            Some(w) if w.mode == Mode::Fullscreen || w.restore.is_some() || w.fs_saved.is_some() => Tile::Full,
            _ => Tile::Floating,
        };
        ContextExt { tile, min_hit: self.style.theme.size.hit_min_fine as u16, ..Default::default() }
    }

    /// Okno zmenilo stav (maximalizace, cela obrazovka): novy kontext
    /// (dlazdice).
    pub(crate) fn context_of_window_changed(&mut self, gid: u32) {
        self.ctx_generation = self.ctx_generation.wrapping_add(1);
        self.send_context(gid);
    }

    /// Zmena obrazovky (rozliseni, lista): nova generace kontextu vsem oknum.
    pub(crate) fn contexts_changed(&mut self) {
        self.ctx_generation = self.ctx_generation.wrapping_add(1);
        let ids: alloc::vec::Vec<u32> = self.windows.keys().copied().collect();
        for g in ids {
            self.send_context(g);
        }
    }

    /// Velikost obsahu okna `mode` pro pozadavek `w` x `h` se stropem
    /// v promile pracovni plochy (0 = bez stropu), oriznuta na pracovni
    /// plochu (okno s titulkem se vejde nad listu).
    pub(crate) fn fit_size(&self, mode: Mode, w: u32, h: u32, frac_w: u16, frac_h: u16) -> (u32, u32) {
        if mode == Mode::Fullscreen {
            return (self.w, self.h);
        }
        let geo = self.style.geo();
        let work = self.work_area();
        let top = if mode == Mode::Standard { geo.title_h + geo.border } else { geo.border };
        let mut max_w = self.w.saturating_sub(SIDE_GAP).max(16);
        let mut max_h = work.h.saturating_sub(top + geo.border).max(16);
        if frac_w > 0 {
            max_w = max_w.min(work.w * frac_w.min(1000) as u32 / 1000).max(16);
        }
        if frac_h > 0 {
            max_h = max_h.min(work.h * frac_h.min(1000) as u32 / 1000).max(16);
        }
        (w.clamp(16, max_w), h.clamp(16, max_h))
    }

    /// `WINDOW_SIZE` od klienta `c` pro jeho okno `local`.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn window_size(&mut self, c: ClientId, local: u32, w: u32, h: u32, min: (u32, u32), frac: (u16, u16), now: u64) {
        let Some(gid) = self.clients.get(&c).and_then(|cl| cl.windows.get(&local)).copied() else {
            return self.error(c, guiproto::window::op::WINDOW_SIZE, guiproto::Error::InvalidArgs);
        };
        let Some(win) = self.windows.get_mut(&gid) else { return };
        if min.0 > 0 || min.1 > 0 {
            win.min = (min.0.clamp(16, guiproto::MAX_DIM), min.1.clamp(16, guiproto::MAX_DIM));
        }
        let (mode, mapped, minw) = (win.mode, win.mapped, win.min);
        // Velikost "normalniho" okna: u maximalizovaneho / cele obrazovky
        // ta po obnoveni.
        let base = win.fs_saved.as_ref().map(|s| s.restore.unwrap_or(s.rect)).or(win.restore).unwrap_or(win.rect);
        let normal_mode = win.fs_saved.as_ref().map_or(mode, |s| s.mode);
        let (rw, rh) = (if w == 0 { base.w } else { w }, if h == 0 { base.h } else { h });
        let (tw, th) = self.fit_size(normal_mode, rw.max(minw.0), rh.max(minw.1), frac.0, frac.1);
        let Some(win) = self.windows.get_mut(&gid) else { return };
        // Maximalizovane nebo na cele obrazovce: zapamatovat na obnoveni.
        if let Some(s) = win.fs_saved.as_mut() {
            match s.restore.as_mut() {
                Some(r) => (r.w, r.h) = (tw, th),
                None => (s.rect.w, s.rect.h) = (tw, th),
            }
            return;
        }
        if let Some(r) = win.restore.as_mut() {
            (r.w, r.h) = (tw, th);
            return;
        }
        if mode == Mode::Fullscreen {
            return;
        }
        if !mapped {
            // Pred prvnim zobrazenim: rovnou nova velikost a misto (nad
            // listou), `CONFIGURE` s ni pujde pred prvnim commitem.
            let geo = win.geo;
            let top = if mode == Mode::Standard { (geo.title_h + geo.border) as i32 } else { geo.border as i32 };
            let work_h = self.work_area().h as i32;
            let win = self.windows.get_mut(&gid).unwrap();
            win.rect.w = tw;
            win.rect.h = th;
            win.rect.y = win.rect.y.min(work_h - th as i32 - geo.border as i32).max(top);
            win.serial = win.serial.wrapping_add(1);
            let (serial, focused) = (win.serial, false);
            let state = win.state(focused);
            self.send(c, ServerMsg::Configure { window: local, w: tw, h: th, state, serial });
            return;
        }
        self.configure(gid, tw, th, now);
    }
}
