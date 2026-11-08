//! Zmena velikosti obrazovky za behu (novy rezim displeje, jiny monitor
//! po hotplugu). Okna na celou obrazovku a maximalizovana dostanou
//! `CONFIGURE` na novou plochu, ostatni se posunou tak, aby jejich
//! titulek zustal na obrazovce. Kurzor se orizne, cela obrazovka se
//! prekresli.

use guiproto::Rect;
use guiproto::window::Mode;

use crate::Comp;
use crate::render::PANEL_H;
use crate::scene::{BORDER, TITLE_H};

/// Kolik okna musi zustat videt vodorovne (aby slo chytit za titulek).
const VISIBLE: i32 = 64;

impl Comp {
    /// Nova velikost obrazovky `w` x `h` (sluzba ji zavola po `MODE_SET`
    /// a pred novym `ATTACH`). Stejna velikost nic nedela.
    pub fn resize(&mut self, w: u32, h: u32, now: u64) {
        let (w, h) = (w.clamp(PANEL_H + 2 * TITLE_H, guiproto::MAX_DIM), h.clamp(PANEL_H + 2 * TITLE_H, guiproto::MAX_DIM));
        if (w, h) == (self.w, self.h) {
            return;
        }
        self.w = w;
        self.h = h;
        self.input.px = self.input.px.clamp(0, w as i32 - 1);
        self.input.py = self.input.py.clamp(0, h as i32 - 1);
        let ids: alloc::vec::Vec<u32> = self.windows.keys().copied().collect();
        for g in ids {
            let Some(win) = self.windows.get_mut(&g) else { continue };
            let top = if win.mode == Mode::Standard { (TITLE_H + BORDER) as i32 } else { 0 };
            if win.mode == Mode::Fullscreen {
                win.rect.x = 0;
                win.rect.y = 0;
                self.configure(g, w, h, now);
            } else if win.restore.is_some() {
                // Maximalizovane: nova pracovni plocha nad panelem.
                let t = if win.mode == Mode::Standard { TITLE_H } else { 0 };
                win.rect.x = 1;
                win.rect.y = t as i32 + 1;
                // Puvodni pozici pred maximalizaci take dostat na obrazovku.
                if let Some(r) = win.restore.as_mut() {
                    *r = clamp_rect(*r, w, h, top);
                }
                self.configure(g, w - 2, h - PANEL_H - t - 2, now);
            } else {
                win.rect = clamp_rect(win.rect, w, h, top);
            }
        }
        // Puvodni poskozeni patri stare velikosti.
        self.dirty = gfx::damage::Damage::new();
        self.damage_all();
    }
}

/// Posune obdelnik obsahu tak, aby titulek (nad nim `top` px) byl na
/// obrazovce a aspon [`VISIBLE`] px okna vodorovne videt.
fn clamp_rect(mut r: Rect, w: u32, h: u32, top: i32) -> Rect {
    let max_x = w as i32 - VISIBLE;
    let max_y = (h - PANEL_H) as i32 - VISIBLE.min(TITLE_H as i32);
    let lo = VISIBLE - r.w as i32;
    r.x = r.x.clamp(lo, max_x.max(lo));
    r.y = r.y.clamp(top, max_y.max(top));
    r
}
