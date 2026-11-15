//! Plynuly prechod motivu (svetly <-> tmavy <-> kontrast): barvy se
//! prolinaji v OKLab (vnimove rovnomerny prostor - stred prechodu neni
//! sedivy ani "spinavy" jako v sRGB), pruhlednost linearne. Stiny maji ve
//! vsech rezimech stejnou geometrii, prolina se jen jejich barva; vrstva,
//! ktera v jednom rezimu chybi (kontrast nema stiny), se rozplyne do
//! pruhledna.
//!
//! Rozmery, pisma a pohyb se neprolinaji: berou se z ciloveho motivu (jsou
//! ve vsech rezimech stejne). Rezim `mode` prolnuteho motivu je cilovy od
//! poloviny prechodu (rozhoduje treba o tloustce ramecku kontrastu).

use crate::color::unpremul;
use crate::{Colors, Shadow, ShadowLayer, Shadows, Theme};

/// Barva v OKLab s pruhlednosti (0..=1).
#[derive(Debug, Clone, Copy, PartialEq)]
struct Lab {
    l: f32,
    a: f32,
    b: f32,
    alpha: f32,
}

fn to_linear(c: u32) -> f32 {
    let c = c as f32 / 255.0;
    if c <= 0.04045 { c / 12.92 } else { libm::powf((c + 0.055) / 1.055, 2.4) }
}

fn to_srgb(c: f32) -> u32 {
    let c = c.clamp(0.0, 1.0);
    let s = if c <= 0.003_130_8 { c * 12.92 } else { 1.055 * libm::powf(c, 1.0 / 2.4) - 0.055 };
    (s * 255.0 + 0.5).clamp(0.0, 255.0) as u32
}

/// Premultiplied `0xAARRGGBB` -> OKLab (Bjorn Ottosson, 2020).
fn lab(c: u32) -> Lab {
    let (r, g, b, a) = unpremul(c);
    let (r, g, b) = (to_linear(r), to_linear(g), to_linear(b));
    let l = libm::cbrtf(0.412_221_46 * r + 0.536_332_55 * g + 0.051_445_995 * b);
    let m = libm::cbrtf(0.211_903_5 * r + 0.680_699_5 * g + 0.107_396_96 * b);
    let s = libm::cbrtf(0.088_302_46 * r + 0.281_718_85 * g + 0.629_978_7 * b);
    Lab {
        l: 0.210_454_26 * l + 0.793_617_8 * m - 0.004_072_047 * s,
        a: 1.977_998_5 * l - 2.428_592_2 * m + 0.450_593_7 * s,
        b: 0.025_904_037 * l + 0.782_771_77 * m - 0.808_675_77 * s,
        alpha: a as f32 / 255.0,
    }
}

/// OKLab -> premultiplied `0xAARRGGBB`.
fn premul(c: Lab) -> u32 {
    let l = c.l + 0.396_337_78 * c.a + 0.215_803_76 * c.b;
    let m = c.l - 0.105_561_346 * c.a - 0.063_854_17 * c.b;
    let s = c.l - 0.089_484_18 * c.a - 1.291_485_5 * c.b;
    let (l, m, s) = (l * l * l, m * m * m, s * s * s);
    let r = to_srgb(4.076_741_7 * l - 3.307_711_6 * m + 0.230_969_94 * s);
    let g = to_srgb(-1.268_438 * l + 2.609_757_4 * m - 0.341_319_38 * s);
    let b = to_srgb(-0.004_196_086_3 * l - 0.703_418_6 * m + 1.707_614_7 * s);
    let a = (c.alpha.clamp(0.0, 1.0) * 255.0 + 0.5) as u32;
    crate::to_premul(r, g, b, a)
}

/// Prolnuti dvou barev v OKLab; `t` 0 = `a`, 1 = `b`. Plne pruhledna
/// barva nema odstin: prevezme ho od druhe (pruhledna -> cervena neni pres
/// cernou).
pub fn mix(a: u32, b: u32, t: f32) -> u32 {
    let t = t.clamp(0.0, 1.0);
    if t <= 0.0 || a == b {
        return a;
    }
    if t >= 1.0 {
        return b;
    }
    let (mut x, mut y) = (lab(a), lab(b));
    if a >> 24 == 0 {
        x = Lab { alpha: 0.0, ..y };
    }
    if b >> 24 == 0 {
        y = Lab { alpha: 0.0, ..x };
    }
    let f = |p: f32, q: f32| p + (q - p) * t;
    premul(Lab { l: f(x.l, y.l), a: f(x.a, y.a), b: f(x.b, y.b), alpha: f(x.alpha, y.alpha) })
}

fn mix_shadow(a: &Shadow, b: &Shadow, t: f32) -> Shadow {
    let n = a.len().max(b.len());
    (0..n)
        .map(|i| {
            // Chybejici vrstva = stejna geometrie, pruhledna barva.
            let (la, lb) = match (a.get(i), b.get(i)) {
                (Some(x), Some(y)) => (*x, *y),
                (Some(x), None) => (*x, ShadowLayer { color: 0, ..*x }),
                (None, Some(y)) => (ShadowLayer { color: 0, ..*y }, *y),
                (None, None) => unreachable!(),
            };
            let g = if t < 0.5 { la } else { lb };
            ShadowLayer { color: mix(la.color, lb.color, t), ..g }
        })
        .filter(|l| l.color >> 24 != 0)
        .collect()
}

impl Colors {
    /// Prolnuti vsech roli.
    pub fn mix(&self, other: &Colors, t: f32) -> Colors {
        let mut out = other.clone();
        out.each_mut(self, |o, a| *o = mix(a, *o, t));
        out
    }
}

impl Shadows {
    pub fn mix(&self, other: &Shadows, t: f32) -> Shadows {
        Shadows {
            window: mix_shadow(&self.window, &other.window, t),
            window_inactive: mix_shadow(&self.window_inactive, &other.window_inactive, t),
            popover: mix_shadow(&self.popover, &other.popover, t),
            bar: mix_shadow(&self.bar, &other.bar, t),
            dialog: mix_shadow(&self.dialog, &other.dialog, t),
        }
    }
}

impl Theme {
    /// Vestaveny motiv prolnuty z rezimu `from` do `to` v bode `t`
    /// (0-1000, zprava kompozitoru `ThemeMix`).
    pub fn builtin_mix(from: crate::Mode, to: crate::Mode, t: u16) -> Theme {
        Theme::builtin(from).mix(&Theme::builtin(to), t as f32 / 1000.0)
    }

    /// Motiv v case `t` (0..=1, uz po krivce) prechodu z `self` do `to`.
    pub fn mix(&self, to: &Theme, t: f32) -> Theme {
        let t = t.clamp(0.0, 1.0);
        let mut out = to.clone();
        if t < 0.5 {
            out.mode = self.mode;
        }
        out.color = self.color.mix(&to.color, t);
        out.shadow = self.shadow.mix(&to.shadow, t);
        // Pruhlednost listy (kontrast ma plnou).
        out.opacity.bar = (self.opacity.bar as f32 + (to.opacity.bar as f32 - self.opacity.bar as f32) * t + 0.5) as u32;
        out
    }
}

#[cfg(test)]
pub(crate) fn lab_l(c: u32) -> f32 {
    lab(c).l
}
