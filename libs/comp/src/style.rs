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
    /// Obraz plochy pro (sirka, vyska, barvy bg, wall_a, wall_b): pri
    /// prolinani motivu se meni barvy, ne rezim.
    /// Sdileny (`Arc`): prechod ze startovni obrazovky ho pouziva bez kopie.
    backdrop: Option<(u32, u32, [u32; 3], alloc::sync::Arc<Vec<u32>>)>,
    /// Prolnuti rezimu: obraz plochy stareho a noveho rezimu se spocita
    /// jednou a kazdy snimek se jen smicha (`t` 0..=256). Vykresleni
    /// prechodu barev plochy znovu by stalo vetsinu snimku.
    fade: Option<BackdropFade>,
}

struct BackdropFade {
    w: u32,
    h: u32,
    from: Vec<u32>,
    to: Vec<u32>,
    t: u32,
    /// `t`, pro ktere je smichany obraz v `backdrop`.
    done: Option<u32>,
}

/// Obraz plochy z barev motivu (stejne jako `.desk` v prototypu,
/// design/assets/mo.css).
fn render_backdrop(c: &theme::Colors, w: u32, h: u32) -> Vec<u32> {
    let layers = [
        Radial { cx: 0.18, cy: 0.22, stop: 0.52, color: c.wall_a },
        Radial { cx: 0.82, cy: 0.72, stop: 0.56, color: c.wall_b },
    ];
    backdrop::render(w, h, c.bg, &layers)
}

/// `out = a + (b - a) * t / 256` po kanalech (dva kanaly naraz v u32).
fn blend_into(out: &mut [u32], a: &[u32], b: &[u32], t: u32) {
    let t = t.min(256);
    for ((o, &x), &y) in out.iter_mut().zip(a).zip(b) {
        let rb = |v: u32| v & 0x00FF_00FF;
        let ag = |v: u32| (v >> 8) & 0x00FF_00FF;
        let mix = |p: u32, q: u32| ((p * (256 - t) + q * t) >> 8) & 0x00FF_00FF;
        *o = mix(rb(x), rb(y)) | mix(ag(x), ag(y)) << 8;
    }
}

impl Style {
    pub fn new(fonts: Family, mode: Mode) -> Self {
        let tokens = Tokens::builtin();
        let theme = tokens.theme(mode).expect("design/tokens.toml je uplny (test tokens_builtin)");
        Self { tokens, theme, fonts, icons: Icons::new(), masks: BTreeMap::new(), backdrop: None, fade: None }
    }

    /// Prepne rezim (svetly, tmavy, vysoky kontrast).
    pub fn set_mode(&mut self, mode: Mode) {
        if let Ok(t) = self.tokens.theme(mode) {
            self.theme = t;
            self.backdrop = None;
            self.fade = None;
        }
    }

    pub fn mode(&self) -> Mode {
        self.theme.mode
    }

    /// Motiv rezimu `mode` z tokenu (cil prolnuti).
    pub fn theme_for(&self, mode: Mode) -> Option<Theme> {
        self.tokens.theme(mode).ok()
    }

    /// Prolnuty motiv (snimek prechodu rezimu, `appearance.rs`).
    pub fn set_theme(&mut self, t: Theme) {
        self.theme = t;
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

    /// Zacatek prolnuti plochy z motivu `from` do `to` pro obrazovku `w x h`.
    pub fn begin_backdrop_fade(&mut self, from: &Theme, to: &Theme, w: u32, h: u32) {
        let from = render_backdrop(&from.color, w, h);
        let to = render_backdrop(&to.color, w, h);
        self.fade = Some(BackdropFade { w, h, from, to, t: 0, done: None });
    }

    /// Bod prolnuti plochy (0.0..=1.0).
    pub fn set_backdrop_fade(&mut self, k: f32) {
        if let Some(f) = self.fade.as_mut() {
            f.t = (k.clamp(0.0, 1.0) * 256.0 + 0.5) as u32;
        }
    }

    /// Obraz plochy pro velikost `w x h` (spocita se jednou).
    pub fn backdrop(&mut self, w: u32, h: u32) -> &[u32] {
        if let Some(f) = self.fade.as_mut()
            && f.w == w
            && f.h == h
        {
            let key = [0, 0, 0];
            if f.done != Some(f.t) || !matches!(&self.backdrop, Some((bw, bh, k, _)) if *bw == w && *bh == h && *k == key) {
                let mut img = match self.backdrop.take() {
                    Some((bw, bh, _, a)) if bw == w && bh == h => alloc::sync::Arc::try_unwrap(a).unwrap_or_else(|a| (*a).clone()),
                    _ => alloc::vec![0; (w * h) as usize],
                };
                blend_into(&mut img, &f.from, &f.to, f.t);
                f.done = Some(f.t);
                self.backdrop = Some((w, h, key, alloc::sync::Arc::new(img)));
            }
            return &self.backdrop.as_ref().unwrap().3;
        }
        let key = [self.theme.color.bg, self.theme.color.wall_a, self.theme.color.wall_b];
        let fresh = matches!(&self.backdrop, Some((bw, bh, k, _)) if *bw == w && *bh == h && *k == key);
        if !fresh {
            let img = render_backdrop(&self.theme.color, w, h);
            self.backdrop = Some((w, h, key, alloc::sync::Arc::new(img)));
        }
        &self.backdrop.as_ref().unwrap().3
    }

    /// Jako [`backdrop`](Self::backdrop), sdileny obraz (bez kopie).
    pub fn backdrop_shared(&mut self, w: u32, h: u32) -> alloc::sync::Arc<Vec<u32>> {
        self.backdrop(w, h);
        self.backdrop.as_ref().unwrap().3.clone()
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
    /// Prepne rezim barev hned (bez prolnuti) a prekresli vse. Volba
    /// uzivatele a prolnuti: [`Comp::set_appearance_user`].
    pub fn set_mode(&mut self, mode: Mode) {
        self.appearance.anim = None;
        self.appearance.target = mode;
        self.style.set_mode(mode);
        // Pozadi pod listou ma nove barvy.
        self.bar_blur_dirty = true;
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

    /// Cilovy rezim (behem prolnuti uz ten novy).
    pub fn mode(&self) -> Mode {
        self.appearance.target
    }
}
