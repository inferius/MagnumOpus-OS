//! Vzhled kompozitoru: motiv z tokenu (`libs/theme`), pisma, ikony,
//! predpocitane stiny a obraz plochy.
//!
//! Vsechny barvy, rozmery a zaobleni systemovych prvku (ram, titulek,
//! odznak, tlacitka, lista, dialogy) jdou odsud; v `render.rs` nejsou
//! cisla vzhledu. Motiv jde prepnout za behu ([`Comp::set_mode`]).

use alloc::collections::BTreeMap;
use alloc::vec::Vec;

use gfx::backdrop::{self, Radial};
use gfx::icon::Icons;
use gfx::shadow::ShadowMask;
use gfx::text::{Family, Text};
use guiproto::Rect;
use theme::{Mode, Shadow, TextStyle, Theme, Tokens};

use crate::Comp;
use crate::scene::Geo;

/// Vzhled: motiv, pisma, ikony a mezipameti.
pub struct Style {
    tokens: Tokens,
    pub theme: Theme,
    pub fonts: Family,
    pub icons: Icons,
    masks: BTreeMap<(u32, u32), ShadowMask>,
    /// Obraz plochy pro (sirka, vyska, rezim).
    backdrop: Option<(u32, u32, Mode, Vec<u32>)>,
}

impl Style {
    pub fn new(fonts: Family, mode: Mode) -> Self {
        let tokens = Tokens::builtin();
        let theme = tokens.theme(mode).expect("design/tokens.toml je uplny (test tokens_builtin)");
        Self { tokens, theme, fonts, icons: Icons::new(), masks: BTreeMap::new(), backdrop: None }
    }

    /// Prepne rezim (svetly, tmavy, vysoky kontrast).
    pub fn set_mode(&mut self, mode: Mode) {
        if let Ok(t) = self.tokens.theme(mode) {
            self.theme = t;
            self.backdrop = None;
        }
    }

    pub fn mode(&self) -> Mode {
        self.theme.mode
    }

    /// Rozmery oken z tokenu.
    pub fn geo(&self) -> Geo {
        let t = &self.theme;
        let (mut l, mut tp, mut r, mut b) = (0i32, 0i32, 0i32, 0i32);
        for s in t.shadow.window.iter().chain(&t.shadow.window_inactive) {
            let e = s.blur as i32 + s.spread;
            l = l.max(e - s.x);
            r = r.max(e + s.x);
            tp = tp.max(e - s.y);
            b = b.max(e + s.y);
        }
        Geo {
            title_h: t.size.title_h,
            border: 1,
            btn: t.size.title_btn,
            btn_hit: t.size.hit_min,
            btn_slop: t.space[2],
            btn_gap: t.space[1],
            pad: t.space[3],
            badge_h: t.size.badge_h,
            radius: t.radius.window,
            edge: t.size.resize_edge,
            shadow: [l.max(0) as u32, tp.max(0) as u32, r.max(0) as u32, b.max(0) as u32],
        }
    }

    /// Rez pisma UI pro styl.
    pub fn face(&mut self, s: TextStyle) -> &mut Text {
        self.fonts.face(s.weight)
    }

    pub fn face_ref(&self, s: TextStyle) -> &Text {
        self.fonts.face_ref(s.weight)
    }

    /// Sirka textu ve stylu `s`.
    pub fn width(&self, text: &str, s: TextStyle) -> u32 {
        self.face_ref(s).width(text, s.size)
    }

    /// Maska stinu pro polomer a rozmazani (mezipamet).
    pub fn mask(&mut self, radius: u32, blur: u32) -> &ShadowMask {
        self.masks.entry((radius, blur)).or_insert_with(|| ShadowMask::new(radius, blur))
    }

    /// Obraz plochy pro velikost `w x h` (spocita se jednou).
    pub fn backdrop(&mut self, w: u32, h: u32) -> &[u32] {
        let mode = self.theme.mode;
        let fresh = matches!(&self.backdrop, Some((bw, bh, m, _)) if *bw == w && *bh == h && *m == mode);
        if !fresh {
            let c = &self.theme.color;
            // Stejne jako `.desk` v prototypu (design/assets/mo.css).
            let layers = [
                Radial { cx: 0.18, cy: 0.22, stop: 0.52, color: c.wall_a },
                Radial { cx: 0.82, cy: 0.72, stop: 0.56, color: c.wall_b },
            ];
            let img = backdrop::render(w, h, c.bg, &layers);
            self.backdrop = Some((w, h, mode, img));
        }
        &self.backdrop.as_ref().unwrap().3
    }

    /// Nakresli vicevrstvy stin tvaru `shape` (vrstvy se nejdriv slozi do
    /// jedne barvy, pak jedno prolnuti: stejny vysledek na CPU i GPU).
    pub fn draw_shadow(&mut self, c: &mut gfx::Canvas<'_>, shadow: &Shadow, shape: Rect, radius: u32, skip: Rect) {
        if shadow.is_empty() {
            return;
        }
        // Masky pro vsechny vrstvy (nejdriv vytvorit, pak pujcit).
        for s in shadow {
            let r = (radius as i32 + s.spread).max(0) as u32;
            self.mask(r, s.blur);
        }
        let layers: Vec<(&ShadowMask, Rect, u32)> = shadow
            .iter()
            .rev()
            .map(|s| {
                let r = (radius as i32 + s.spread).max(0) as u32;
                let sp = s.spread;
                let rect = Rect::new(shape.x + s.x - sp, shape.y + s.y - sp, (shape.w as i32 + 2 * sp).max(0) as u32, (shape.h as i32 + 2 * sp).max(0) as u32);
                (&self.masks[&(r, s.blur)], rect, s.color)
            })
            .collect();
        c.shadow_layers(&layers, skip);
    }
}

/// Kod rezimu ve zprave `THEME`.
pub fn mode_code(m: Mode) -> u8 {
    match m {
        Mode::Light => 0,
        Mode::Dark => 1,
        Mode::Contrast => 2,
    }
}

/// Rezim ze zpravy `THEME` (neznamy kod = svetly).
pub fn mode_from_code(c: u8) -> Mode {
    match c {
        1 => Mode::Dark,
        2 => Mode::Contrast,
        _ => Mode::Light,
    }
}

impl Comp {
    /// Prepne rezim barev a prekresli vse.
    pub fn set_mode(&mut self, mode: Mode) {
        self.style.set_mode(mode);
        let g = self.style.geo();
        for w in self.windows.values_mut() {
            w.geo = g;
        }
        // Aplikace kresli stejnym motivem.
        let ids: Vec<crate::ClientId> = self.clients.iter().filter(|(_, c)| c.hello).map(|(&id, _)| id).collect();
        for id in ids {
            self.send(id, guiproto::window::ServerMsg::Theme { mode: mode_code(mode) });
        }
        self.damage_all();
    }

    pub fn mode(&self) -> Mode {
        self.style.mode()
    }
}
