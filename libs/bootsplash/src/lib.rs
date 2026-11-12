//! Startovni obrazovka MagnumOpus (navrh `design/start.html`: varianta A,
//! log z B, prechod do avataru).
//!
//! Jedna knihovna pro vsechny, kdo obraz kresli: loader (prvni snimek hned
//! po prevzeti GOP), init (rozsviceni, dychani, prstenec prubehu) a
//! kompozitor (prvni snimek plochy navazuje, znak se zmensi a rozplyne).
//! Diky tomu je obraz ve vsech fazich pixel po pixelu stejny.
//!
//! Obraz: teple pozadi plochy (stejne jako `gfx::backdrop` kompozitoru),
//! uprostred znak 132 px (logo 84 px, prstenec polomer 62 px a tah 3 px,
//! dychajici zare) a pod nim nazev 44 px. Rozmery jsou logicke pixely
//! prototypu, meritko je `vyska / 800` (1280 x 800 = 1:1).
//!
//! Kresli se jen plne barvy, prechody a pruhlednost (zadne rozmazani):
//! prekreslit oblast znaku stoji desetiny ms, cela obrazovka jednotky ms.
//!
//! Pixel vystupu je `0xFFRRGGBB` (format `gfx`, framebuffer BGR); pro
//! framebuffer RGB se kanaly prohodi ([`Splash::set_rgb`]).

#![cfg_attr(not(test), no_std)]

extern crate alloc;

pub mod art;
mod geom;

#[cfg(test)]
mod tests;

use alloc::vec::Vec;

use art::Mask;
pub use geom::Rect;

/// Rozmery prototypu (logicke px pri meritku 1).
pub mod size {
    /// Znak (prstenec s logem).
    pub const MARK: f32 = 132.0;
    /// Logo uvnitr prstence.
    pub const LOGO: f32 = 84.0;
    /// Polomer prstence (stred tahu) a tloustka tahu.
    pub const RING_R: f32 = 62.0;
    pub const RING_W: f32 = 3.0;
    /// Zare: konec prechodu (65 % paprsku k rohu ctverce 184 px).
    pub const GLOW_R: f32 = 84.57;
    /// Pruhlednost akcentu ve stredu zare a v podkladu prstence.
    pub const GLOW_ALPHA: f32 = 0.42;
    pub const RING_BG_ALPHA: f32 = 0.22;
    /// Mezera znak - nazev (`--s5`) a vyska nazvu.
    pub const GAP: f32 = 16.0;
    pub const WORD_H: f32 = 44.0;
    /// Mezera nazev - radek "ceka na" (`--s3`) a jeho vyska.
    pub const STALL_GAP: f32 = 8.0;
    pub const STALL_H: f32 = 20.0;
    /// Referencni vyska obrazovky pro meritko 1.
    pub const REF_H: f32 = 800.0;
    /// Nejvetsi meritko dychani zare (keyframes 0.92 - 1.05).
    pub const GLOW_MAX_SCALE: f32 = 1.05;
}

/// Casovani z prototypu (ms).
pub mod timing {
    /// Rozsviceni pozadi.
    pub const WALL_FADE: u64 = 900;
    /// Nazev se objevi (od zacatku rozsviceni) a doba jeho prolnuti.
    pub const WORD_DELAY: u64 = 400;
    pub const WORD_FADE: u64 = 600;
    /// Perioda dychani zare.
    pub const BREATHE: u64 = 2400;
    /// Prstenec dojede na novou hodnotu.
    pub const RING: u64 = 700;
    /// Zdrzeni na kroku, po kterem se ukaze "ceka na: ...".
    pub const STALL_AFTER: u64 = 2000;
    /// Prechod do plochy: presun a zmenseni znaku, pak rozplynuti.
    pub const HANDOFF_MOVE: u64 = 600;
    pub const HANDOFF_FADE: u64 = 300;
    /// Pri omezenem pohybu jen prolnuti.
    pub const HANDOFF_REDUCED: u64 = 90;
}

/// Kroky startu (prstenec se plni po krocich, jako v prototypu).
pub const STEPS: [&str; 5] = ["jádro", "zařízení", "soubory", "síť", "plocha"];

/// Stav obrazu v jednom okamziku.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct State {
    /// Teple pozadi: 0 = cerna (firmware), 1 = plne rozsvicene.
    pub wall: f32,
    /// Kryti pozadi obrazu (cerna/teple) nad tim, co uz ve vystupu je:
    /// 1 = prekryje vse (loader, init), mene = kompozitor pod nim
    /// ukazuje plochu.
    pub backdrop: f32,
    /// Pruhlednost nazvu.
    pub word: f32,
    /// Prstenec 0..=1.
    pub progress: f32,
    /// Zare: kryti (0..1) a meritko.
    pub glow: f32,
    pub glow_scale: f32,
    /// Znak jako jedna vrstva: meritko, posun stredu (px) a kryti.
    pub mark_scale: f32,
    pub mark_dx: f32,
    pub mark_dy: f32,
    pub mark_opacity: f32,
}

impl State {
    /// Prvni snimek loaderu: cerna, znak s prazdnym prstencem, bez nazvu.
    pub const FIRST: State = State {
        wall: 0.0,
        backdrop: 1.0,
        word: 0.0,
        progress: 0.0,
        glow: 0.0,
        glow_scale: 1.0,
        mark_scale: 1.0,
        mark_dx: 0.0,
        mark_dy: 0.0,
        mark_opacity: 1.0,
    };

    /// Plne rozsviceny obraz s danym prstencem.
    pub fn lit(progress: f32) -> State {
        State { wall: 1.0, word: 1.0, progress, ..State::FIRST }
    }
}

/// Dychani zare v case `t_ms` (od libovolneho spolecneho pocatku, napr.
/// startu systemu - init i kompozitor tak navazuji bez skoku):
/// `(kryti, meritko)` podle `@keyframes breathe` (0.25 - 0.7, 0.92 - 1.05).
pub fn breathe(t_ms: u64) -> (f32, f32) {
    let ph = (t_ms % timing::BREATHE) as f32 / timing::BREATHE as f32;
    // 0 -> 1 -> 0 hladce (kosinus), ease-standard je blizko.
    let k = 0.5 - 0.5 * geom::cos(ph * core::f32::consts::TAU);
    (0.25 + 0.45 * k, 0.92 + 0.13 * k)
}

/// Krivka `cubic-bezier(.2, 0, 0, 1)` (ease-enter prototypu) pro `t` 0..1.
pub fn ease(t: f32) -> f32 {
    geom::bezier_ease(t.clamp(0.0, 1.0), 0.2, 0.0, 0.0, 1.0)
}

/// Barvy (premultiplied `0xAARRGGBB`) ze svetleho motivu: start je pred
/// nactenim nastaveni uzivatele, "teple pozadi" prototypu.
#[derive(Debug, Clone, Copy)]
struct Palette {
    bg: u32,
    wall_a: u32,
    wall_b: u32,
    accent: u32,
    ink: u32,
}

impl Palette {
    fn light() -> Self {
        let t = theme::Theme::builtin(theme::Mode::Light);
        let c = &t.color;
        Self { bg: c.bg, wall_a: c.wall_a, wall_b: c.wall_b, accent: c.accent, ink: c.ink }
    }
}

/// Rozvrzeni pro obrazovku `w x h`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Layout {
    pub w: u32,
    pub h: u32,
    /// Meritko logickych px.
    pub s: f32,
    /// Stred znaku.
    pub cx: f32,
    pub cy: f32,
    /// Horni okraj nazvu a radku "ceka na".
    pub word_top: f32,
    pub stall_top: f32,
}

impl Layout {
    pub fn new(w: u32, h: u32) -> Self {
        use size::*;
        let s = (h as f32 / REF_H).clamp(0.75, 3.0).min(w as f32 / 600.0).max(0.5);
        // Sloupec: znak, mezera, nazev - svisle na stred.
        let col = (MARK + GAP + WORD_H) * s;
        let top = geom::round((h as f32 - col) / 2.0);
        let cx = geom::round(w as f32 / 2.0);
        let cy = top + MARK * s / 2.0;
        let word_top = top + (MARK + GAP) * s;
        Self { w, h, s, cx, cy, word_top, stall_top: word_top + (WORD_H + STALL_GAP) * s }
    }

    /// Obdelnik, ktery znak muze zasahnout pri danem meritku a posunu
    /// (vcetne nejvetsi zare).
    pub fn mark_rect(&self, scale: f32, dx: f32, dy: f32) -> Rect {
        use size::*;
        let r = (GLOW_R * GLOW_MAX_SCALE).max(RING_R + RING_W) * self.s * scale + 2.0;
        Rect::around(self.cx + dx, self.cy + dy, r).clip(self.w, self.h)
    }

    /// Radek "ceka na: ..." pod nazvem (sirka cele obrazovky).
    pub fn stall_rect(&self) -> Rect {
        Rect { x: 0, y: self.stall_top as i32, w: self.w, h: (size::STALL_H * self.s + 0.5) as u32 }.clip(self.w, self.h)
    }
}

/// Startovni obraz pro jednu obrazovku.
pub struct Splash {
    pub layout: Layout,
    pal: Palette,
    /// Teple pozadi (az kdyz je potreba: loader kresli jen cerne).
    bg: Option<Vec<u32>>,
    symbol: Vec<art::Contour>,
    /// Logo v posledni pouzite velikosti (px).
    logo: (u32, Mask),
    word: Mask,
    word_x: i32,
    rgb: bool,
}

impl Splash {
    pub fn new(w: u32, h: u32) -> Self {
        let layout = Layout::new(w, h);
        let s = layout.s;
        // Nazev: viewBox = ohraniceni + 4 jednotky okraj (jako prototyp),
        // vyska WORD_H.
        let wm = art::wordmark();
        let (x0, y0, x1, y1) = art::bounds(&wm);
        let k = size::WORD_H * s / (y1 - y0 + 8.0);
        let ww = ((x1 - x0 + 8.0) * k + 0.5) as u32;
        let wh = (size::WORD_H * s + 0.5) as u32;
        let word = art::render(&wm, ww, wh, x0 - 4.0, y0 - 4.0, k);
        let word_x = (layout.cx - ww as f32 / 2.0) as i32;
        Self { layout, pal: Palette::light(), bg: None, symbol: art::symbol(), logo: (0, Mask::default()), word, word_x, rgb: false }
    }

    /// Framebuffer s poradim R, G, B v pameti (prohodit kanaly).
    pub fn set_rgb(&mut self, rgb: bool) {
        self.rgb = rgb;
    }

    /// Teple pozadi (spocita se jednou, ~jednotky ms pro 1080p).
    pub fn backdrop(&mut self) -> &[u32] {
        if self.bg.is_none() {
            let (w, h) = (self.layout.w, self.layout.h);
            let layers = [
                gfx::backdrop::Radial { cx: 0.18, cy: 0.22, stop: 0.52, color: self.pal.wall_a },
                gfx::backdrop::Radial { cx: 0.82, cy: 0.72, stop: 0.56, color: self.pal.wall_b },
            ];
            self.bg = Some(gfx::backdrop::render(w, h, self.pal.bg, &layers));
        }
        self.bg.as_deref().unwrap()
    }

    fn logo_mask(&mut self, px: u32) {
        if self.logo.0 != px {
            let (ox, oy, side) = art::SYMBOL_VIEW;
            self.logo = (px, art::render(&self.symbol, px, px, ox, oy, px as f32 / side));
        }
    }

    /// Nakresli oblast `area` stavu `st` do `out` (radky po `stride`
    /// pixelech, rozmer obrazovky z [`Layout`]). Pri `backdrop < 1` cte
    /// z `out` to, co je pod obrazem.
    pub fn paint(&mut self, st: &State, out: &mut [u32], stride: usize, area: Rect) {
        let l = self.layout;
        let area = area.clip(l.w, l.h);
        if area.is_empty() || out.len() < stride * (area.y as usize + area.h as usize - 1) + area.x as usize + area.w as usize {
            return;
        }
        if st.wall > 0.0 {
            self.backdrop();
        }
        let s = l.s * st.mark_scale;
        let logo_px = ((size::LOGO * s) as u32).max(1);
        self.logo_mask(logo_px);
        let (mcx, mcy) = (l.cx + st.mark_dx, l.cy + st.mark_dy);
        let mark = l.mark_rect(st.mark_scale, st.mark_dx, st.mark_dy).intersect(area);
        let word_r = Rect { x: self.word_x, y: l.word_top as i32, w: self.word.w, h: self.word.h }.intersect(area);
        let wall = st.wall.clamp(0.0, 1.0);
        let back = st.backdrop.clamp(0.0, 1.0);
        let pal = self.pal;
        let accent = geom::rgbf(pal.accent);
        let ink = geom::rgbf(pal.ink);
        // Prstenec: konec oblouku (po smeru hodin od vrcholu).
        let ring_r = size::RING_R * s;
        let hw = size::RING_W * s / 2.0;
        let p = st.progress.clamp(0.0, 1.0);
        let end_a = p * core::f32::consts::TAU;
        let caps = [(mcx, mcy - ring_r), (mcx + ring_r * geom::sin(end_a), mcy - ring_r * geom::cos(end_a))];
        let glow_r = size::GLOW_R * s * st.glow_scale;
        let mo = st.mark_opacity.clamp(0.0, 1.0);
        let logo_x0 = mcx - logo_px as f32 / 2.0;
        let logo_y0 = mcy - logo_px as f32 / 2.0;
        let bg = self.bg.as_deref();
        let rgb = self.rgb;
        let logo = &self.logo.1;
        let word = &self.word;

        // Rychla cesta: pozadi kryje vse (cerna, rozsvicene nebo
        // rozsvecovane teple) = radek se jen zkopiruje nebo celociselne
        // ztlumi, pocitaji se jen pixely znaku a nazvu.
        let fast = back >= 1.0 && (wall <= 0.0 || bg.is_some());
        let dim = (wall * 256.0 + 0.5) as u32;
        let word_on = st.word > 0.0 && !word_r.is_empty();
        let mark_on = mo > 0.0 && !mark.is_empty();
        for y in area.y..area.y + area.h as i32 {
            let row = y as usize * stride;
            if fast {
                let dst = &mut out[row + area.x as usize..row + area.right() as usize];
                match bg {
                    Some(b) if wall > 0.0 => {
                        let src = &b[(y as u32 * l.w + area.x as u32) as usize..][..dst.len()];
                        for (d, &c) in dst.iter_mut().zip(src) {
                            let c = if dim >= 256 { c } else { geom::dim(c, dim) };
                            *d = geom::swap(c, rgb);
                        }
                    }
                    _ => dst.fill(0xFF00_0000),
                }
            }
            let spans: [(i32, i32); 2] = if !fast {
                [(area.x, area.right()), (0, 0)]
            } else {
                let m = if mark_on && y >= mark.y && y < mark.bottom() { (mark.x, mark.right()) } else { (0, 0) };
                let w = if word_on && y >= word_r.y && y < word_r.bottom() { (word_r.x, word_r.right()) } else { (0, 0) };
                [m, w]
            };
            for (k, &(x0, x1)) in spans.iter().enumerate() {
                for x in x0..x1 {
                    // Prekryv znaku a nazvu: pixel uz spocitany.
                    if k == 1 && x >= spans[0].0 && x < spans[0].1 {
                        continue;
                    }
                    let i = row + x as usize;
                    // Podklad: cerna nebo teple pozadi, pres to, co uz je.
                    let mut c = match bg {
                        Some(b) if wall > 0.0 => geom::scale_rgb(b[(y as u32 * l.w + x as u32) as usize], wall),
                        _ => [0.0; 3],
                    };
                    if back < 1.0 {
                        let under = geom::rgbf(geom::unswap(out[i], rgb));
                        c = geom::mix(under, c, back);
                    }
                    if mark.contains(x, y) && mo > 0.0 {
                        let (px, py) = (x as f32 + 0.5 - mcx, y as f32 + 0.5 - mcy);
                        let d = geom::sqrt(px * px + py * py);
                        if st.glow > 0.0 && d < glow_r {
                            let a = size::GLOW_ALPHA * st.glow * (1.0 - d / glow_r) * mo;
                            c = geom::mix(c, accent, a);
                        }
                        let ring = (hw + 0.5 - (d - ring_r).abs()).clamp(0.0, 1.0);
                        if ring > 0.0 {
                            c = geom::mix(c, accent, size::RING_BG_ALPHA * ring * mo);
                        }
                        if p > 0.0 {
                            let mut fg = 0.0;
                            if ring > 0.0 && (p >= 1.0 || geom::clockwise_angle(px, py) <= end_a) {
                                fg = ring;
                            }
                            if p < 1.0 {
                                for &(ex, ey) in &caps {
                                    let (qx, qy) = (x as f32 + 0.5 - ex, y as f32 + 0.5 - ey);
                                    fg = f32::max(fg, (hw + 0.5 - geom::sqrt(qx * qx + qy * qy)).clamp(0.0, 1.0));
                                }
                            }
                            if fg > 0.0 {
                                c = geom::mix(c, accent, fg * mo);
                            }
                        }
                        let lc = logo.get((x as f32 - logo_x0) as i32, (y as f32 - logo_y0) as i32);
                        if lc > 0 {
                            c = geom::mix(c, accent, lc as f32 / 255.0 * mo);
                        }
                    }
                    if st.word > 0.0 && word_r.contains(x, y) {
                        let wc = word.get(x - self.word_x, y - l.word_top as i32);
                        if wc > 0 {
                            c = geom::mix(c, ink, wc as f32 / 255.0 * st.word);
                        }
                    }
                    out[i] = geom::swap(geom::pack(c), rgb);
                }
            }
        }
    }

    /// Cela obrazovka.
    pub fn paint_all(&mut self, st: &State, out: &mut [u32], stride: usize) {
        let r = Rect { x: 0, y: 0, w: self.layout.w, h: self.layout.h };
        self.paint(st, out, stride, r);
    }
}
