//! Motiv vzhledu: tokeny design systemu jako data.
//!
//! Jediny zdroj pravdy je `design/tokens.toml` (barvy, pisma, rozestupy,
//! zaobleni, stiny, pohyb, vrstvy kompozitoru). Knihovna ho cte pres
//! `libs/conf`; v kodu prvku ani kompozitoru nejsou zadna cisla vzhledu.
//!
//! - [`DEFAULT`] je `design/tokens.toml` vlozeny pri prekladu (vychozi
//!   motiv, start bez souboroveho systemu),
//! - [`Tokens::parse`] precte libovolny text ve stejnem formatu (uzivatelske
//!   motivy pozdeji, bez prekladu),
//! - [`Tokens::theme`] da hodnoty pro jeden rezim: svetly, tmavy nebo
//!   vysoky kontrast.
//!
//! Barvy jsou premultiplied `0xAARRGGBB` (stejny format jako `libs/gfx`).
//! Rozmery jsou logicke pixely, casy v ms.
//!
//! ```
//! let t = theme::Tokens::parse(theme::DEFAULT).unwrap();
//! let light = t.theme(theme::Mode::Light).unwrap();
//! assert_eq!(light.color.accent, 0xFF6B3FD6);
//! assert_eq!(light.radius.window, 18);
//! ```

#![cfg_attr(not(test), no_std)]

extern crate alloc;

mod color;
mod motion;

#[cfg(test)]
mod tests;

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use conf::Table;

pub use color::{contrast_ratio, parse_color, to_premul, unpremul};
pub use motion::{Curve, Motion};

/// Tokeny vlozene pri prekladu (`design/tokens.toml`).
pub static DEFAULT: &str = include_str!("../../../design/tokens.toml");

/// Rezim barev.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Light,
    Dark,
    /// Vysoky kontrast (Pristupnost): prebiji svetly i tmavy.
    Contrast,
}

impl Mode {
    /// Jmeno sekce v tokenech (`color.<jmeno>`, `shadow.<jmeno>`).
    pub fn key(self) -> &'static str {
        match self {
            Mode::Light => "light",
            Mode::Dark => "dark",
            Mode::Contrast => "contrast",
        }
    }

    pub const ALL: [Mode; 3] = [Mode::Light, Mode::Dark, Mode::Contrast];
}

/// Chyba tokenu: chybejici nebo neplatny klic (cesta v TOML).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    Syntax(String),
    Missing(String),
    Invalid(String),
}

pub type Result<T> = core::result::Result<T, Error>;

macro_rules! colors {
    ($($f:ident),* $(,)?) => {
        /// Semanticke barvy (role), premultiplied `0xAARRGGBB`.
        #[derive(Debug, Clone, PartialEq, Eq)]
        pub struct Colors { $(pub $f: u32,)* }

        impl Colors {
            fn parse(t: &Table, section: &str) -> Result<Self> {
                Ok(Self { $($f: color_key(t, section, stringify!($f))?,)* })
            }

            /// Barva podle jmena role (kontrola kontrastu, ladeni).
            pub fn get(&self, name: &str) -> Option<u32> {
                match name { $(stringify!($f) => Some(self.$f),)* _ => None }
            }
        }
    };
}

colors!(
    bg, wall_a, wall_b, surface, surface_2, surface_3, border, border_strong, ink, ink_muted, ink_faint, accent,
    on_accent, accent_soft, accent_ink, focus, trust_bg, trust_ink, trust_border, ai, on_ai, ai_soft, ai_ink, ok,
    ok_soft, warn, warn_soft, danger, on_danger, danger_soft, scrim, term_bg, term_ink, term_dim, term_red,
    term_green, term_yellow, term_blue, term_magenta, term_cyan, icon_files_bg, icon_files, icon_system_bg,
    icon_system, icon_network_bg, icon_network, icon_devices_bg, icon_devices, icon_time_bg, icon_time,
);

/// Jeden radek typograficke skaly.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TextStyle {
    /// Velikost pisma (em) v px.
    pub size: f32,
    /// Vyska radku v px.
    pub line: u32,
    /// Vaha (100-900).
    pub weight: u16,
}

macro_rules! styles {
    ($($f:ident),* $(,)?) => {
        /// Typograficka skala.
        #[derive(Debug, Clone, PartialEq)]
        pub struct Typography {
            $(pub $f: TextStyle,)*
            /// Uzivatelske zvetseni (normal, large, larger, largest).
            pub scale: [f32; 4],
        }

        impl Typography {
            fn parse(t: &Table) -> Result<Self> {
                Ok(Self {
                    $($f: style(t, stringify!($f))?,)*
                    scale: [
                        f(t, "type.scale.normal")? as f32,
                        f(t, "type.scale.large")? as f32,
                        f(t, "type.scale.larger")? as f32,
                        f(t, "type.scale.largest")? as f32,
                    ],
                })
            }
        }
    };
}

styles!(caption, small, body, body_strong, body_lg, title, heading, display, mono, clock);

macro_rules! uints {
    ($name:ident, $doc:literal, $section:literal, $($f:ident),* $(,)?) => {
        #[doc = $doc]
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub struct $name { $(pub $f: u32,)* }

        impl $name {
            fn parse(t: &Table) -> Result<Self> {
                Ok(Self { $($f: u(t, concat!($section, ".", stringify!($f)))?,)* })
            }
        }
    };
}

uints!(
    Sizes,
    "Rozmery prvku (px).",
    "size",
    grid,
    hit_min,
    control_h,
    control_h_compact,
    title_h,
    title_btn,
    badge_h,
    bar_h,
    bar_inset,
    bar_icon,
    icon_s,
    icon_m,
    icon_l,
    icon_xl,
    focus_ring,
    focus_offset,
    resize_edge,
    scrollbar,
    icon_xs,
    title_btn_stroke,
    live_dot,
    dialog_w,
    notification_w,
    bar_indicator_w,
    bar_indicator_h,
    bar_indicator_active_w,
    switch_w,
    switch_h,
    switch_knob,
    side_panel,
);

/// Kryti (0..=255 pro `gfx::scale`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Opacity {
    /// Pozadi listy (ve vysokem kontrastu vzdy plne).
    pub bar: u32,
    /// Obsah nereagujiciho okna.
    pub hung_content: u32,
    /// Nedostupny prvek.
    pub disabled: u32,
}

impl Opacity {
    fn parse(t: &Table, mode: Mode) -> Result<Self> {
        let o = |k: &str| -> Result<u32> {
            let v = f(t, k)?;
            if !(0.0..=1.0).contains(&v) {
                return Err(Error::Invalid(k.into()));
            }
            Ok((v * 255.0 + 0.5) as u32)
        };
        let bar = o("opacity.bar")?;
        Ok(Self { bar: if mode == Mode::Contrast { 255 } else { bar }, hung_content: o("opacity.hung_content")?, disabled: o("opacity.disabled")? })
    }
}
uints!(Radii, "Zaobleni (px).", "radius", xs, badge, control, card, window, pill);
uints!(
    Breakpoints,
    "Prahy trid sirky okna (logicke px) a hystereze pro adaptivni UI.",
    "breakpoint",
    medium,
    expanded,
    hysteresis,
);
uints!(
    Layers,
    "Vrstvy kompozitoru (poradi skladani).",
    "layer",
    desktop,
    window,
    bar,
    popover,
    notification,
    scrim,
    system_dialog,
    secure,
);

/// Jedna vrstva stinu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShadowLayer {
    pub x: i32,
    pub y: i32,
    /// Rozmazani (CSS `blur`: sirka prechodu v px, smerodatna odchylka
    /// Gaussova jadra je polovina).
    pub blur: u32,
    pub spread: i32,
    /// Premultiplied.
    pub color: u32,
}

/// Stin: zadna nebo vic vrstev (kresli se odspodu, v poradi zapisu
/// obracene jako v CSS - prvni vrstva je navrchu).
pub type Shadow = Vec<ShadowLayer>;

macro_rules! shadows {
    ($($f:ident),* $(,)?) => {
        /// Stiny podle toho, co je vys.
        #[derive(Debug, Clone, PartialEq, Eq)]
        pub struct Shadows { $(pub $f: Shadow,)* }

        impl Shadows {
            fn parse(t: &Table, section: &str) -> Result<Self> {
                Ok(Self { $($f: shadow_key(t, section, stringify!($f))?,)* })
            }
        }
    };
}

shadows!(window, window_inactive, popover, bar, dialog);

/// Pisma a jejich vlastnosti OpenType.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fonts {
    pub ui: String,
    pub ui_fallback: String,
    pub mono: String,
    pub mono_fallback: String,
    /// Zapnute vlastnosti (`tnum`, `cv11`; `calt=0` = vypnuto).
    pub ui_features: Vec<([u8; 4], bool)>,
    pub mono_features: Vec<([u8; 4], bool)>,
}

/// Povinna dvojice kontrastu.
#[derive(Debug, Clone, PartialEq)]
pub struct ContrastPair {
    pub name: String,
    pub fg: String,
    pub bg: String,
    pub min: f32,
}

/// Hodnoty jednoho rezimu.
#[derive(Debug, Clone, PartialEq)]
pub struct Theme {
    pub mode: Mode,
    pub color: Colors,
    pub shadow: Shadows,
    pub text: Typography,
    /// Rozestupy `s0` az `s11` (mrizka 4 px).
    pub space: [u32; 12],
    pub size: Sizes,
    /// Tloustka tahu ikon (px pri velikosti 24).
    pub icon_stroke: f32,
    pub radius: Radii,
    pub opacity: Opacity,
    pub motion: Motion,
    pub layer: Layers,
    pub breakpoint: Breakpoints,
    pub font: Fonts,
}

/// Prectene tokeny (vsechny rezimy).
#[derive(Debug, Clone)]
pub struct Tokens {
    table: Table,
}

impl Tokens {
    pub fn parse(text: &str) -> Result<Self> {
        let table = conf::parse(text).map_err(|e| Error::Syntax(format!("{e}")))?;
        Ok(Self { table })
    }

    /// Vychozi tokeny ([`DEFAULT`]).
    pub fn builtin() -> Self {
        Self::parse(DEFAULT).expect("design/tokens.toml je platny (test tokens_builtin)")
    }

    pub fn table(&self) -> &Table {
        &self.table
    }

    /// Hodnoty pro rezim `mode`.
    pub fn theme(&self, mode: Mode) -> Result<Theme> {
        let t = &self.table;
        let mut space = [0u32; 12];
        for (i, s) in space.iter_mut().enumerate() {
            *s = u(t, &format!("space.s{i}"))?;
        }
        Ok(Theme {
            mode,
            color: Colors::parse(t, &format!("color.{}", mode.key()))?,
            shadow: Shadows::parse(t, &format!("shadow.{}", mode.key()))?,
            text: Typography::parse(t)?,
            space,
            size: Sizes::parse(t)?,
            icon_stroke: f(t, "size.icon_stroke")? as f32,
            radius: Radii::parse(t)?,
            opacity: Opacity::parse(t, mode)?,
            motion: Motion::parse(t)?,
            layer: Layers::parse(t)?,
            breakpoint: Breakpoints::parse(t)?,
            font: Fonts {
                ui: s(t, "font.ui")?.into(),
                ui_fallback: s(t, "font.ui_fallback")?.into(),
                mono: s(t, "font.mono")?.into(),
                mono_fallback: s(t, "font.mono_fallback")?.into(),
                ui_features: features(t, "font.ui_features")?,
                mono_features: features(t, "font.mono_features")?,
            },
        })
    }

    /// Povinne dvojice kontrastu (sekce `[contrast]`).
    pub fn contrast_pairs(&self) -> Result<Vec<ContrastPair>> {
        let Some(sec) = self.table.get_table("contrast") else {
            return Err(Error::Missing("contrast".into()));
        };
        let mut out = Vec::new();
        for (name, v) in &sec.entries {
            let bad = || Error::Invalid(format!("contrast.{name}"));
            let spec = v.as_str().ok_or_else(bad)?;
            let mut it = spec.split_whitespace();
            let (Some(fg), Some(bg), Some(min), None) = (it.next(), it.next(), it.next(), it.next()) else {
                return Err(bad());
            };
            out.push(ContrastPair { name: name.clone(), fg: fg.into(), bg: bg.into(), min: min.parse().map_err(|_| bad())? });
        }
        Ok(out)
    }
}

impl Theme {
    /// Vychozi motiv v rezimu `mode` z vlozenych tokenu.
    pub fn builtin(mode: Mode) -> Self {
        Tokens::builtin().theme(mode).expect("design/tokens.toml je uplny (test tokens_builtin)")
    }

    /// Kontrola povinnych dvojic kontrastu (stejna pravidla jako
    /// `design/tools/build-tokens.mjs`): vrati nesplnene dvojice
    /// (jmeno, pomer, minimum).
    pub fn contrast_failures(&self, pairs: &[ContrastPair]) -> Vec<(String, f32, f32)> {
        let mut out = Vec::new();
        for p in pairs {
            let min = if self.mode == Mode::Contrast { if p.min >= 4.5 { 7.0 } else { 4.5 } } else { p.min };
            let (Some(a), Some(b)) = (self.color.get(&p.fg), self.color.get(&p.bg)) else {
                out.push((p.name.clone(), 0.0, min));
                continue;
            };
            let r = contrast_ratio(a, b);
            if r + 1e-3 < min {
                out.push((p.name.clone(), r, min));
            }
        }
        out
    }
}

fn get<'a>(t: &'a Table, path: &str) -> Result<&'a conf::Value> {
    t.get(path).ok_or_else(|| Error::Missing(path.into()))
}

fn s<'a>(t: &'a Table, path: &str) -> Result<&'a str> {
    get(t, path)?.as_str().ok_or_else(|| Error::Invalid(path.into()))
}

fn f(t: &Table, path: &str) -> Result<f64> {
    get(t, path)?.as_f64().ok_or_else(|| Error::Invalid(path.into()))
}

fn u(t: &Table, path: &str) -> Result<u32> {
    let v = get(t, path)?.as_int().ok_or_else(|| Error::Invalid(path.into()))?;
    u32::try_from(v).map_err(|_| Error::Invalid(path.into()))
}

fn style(t: &Table, name: &str) -> Result<TextStyle> {
    let size = f(t, &format!("type.{name}.size"))? as f32;
    let line = u(t, &format!("type.{name}.line"))?;
    let weight = u(t, &format!("type.{name}.weight"))?;
    if !(1.0..=512.0).contains(&size) || !(100..=900).contains(&weight) {
        return Err(Error::Invalid(format!("type.{name}")));
    }
    Ok(TextStyle { size, line, weight: weight as u16 })
}

fn color_key(t: &Table, section: &str, key: &str) -> Result<u32> {
    let path = format!("{section}.{key}");
    parse_color(s(t, &path)?).ok_or(Error::Invalid(path))
}

fn shadow_key(t: &Table, section: &str, key: &str) -> Result<Shadow> {
    let path = format!("{section}.{key}");
    parse_shadow(s(t, &path)?).ok_or(Error::Invalid(path))
}

/// Stin ve tvaru `"x y blur spread barva | ..."` nebo `"none"`.
pub fn parse_shadow(spec: &str) -> Option<Shadow> {
    let spec = spec.trim();
    if spec == "none" {
        return Some(Vec::new());
    }
    let mut out = Vec::new();
    for layer in spec.split('|') {
        let layer = layer.trim();
        // Barva muze obsahovat mezery (`rgba(0, 0, 0, 0.5)`): ctyri cisla
        // na zacatku, zbytek je barva.
        let mut rest = layer;
        let mut nums = [0i32; 4];
        for n in nums.iter_mut() {
            let end = rest.find(' ')?;
            *n = rest[..end].parse().ok()?;
            rest = rest[end..].trim_start();
        }
        if nums[2] < 0 {
            return None;
        }
        out.push(ShadowLayer { x: nums[0], y: nums[1], blur: nums[2] as u32, spread: nums[3], color: parse_color(rest)? });
    }
    Some(out)
}

fn features(t: &Table, path: &str) -> Result<Vec<([u8; 4], bool)>> {
    let mut out = Vec::new();
    for item in s(t, path)?.split_whitespace() {
        let (tag, on) = match item.split_once('=') {
            Some((tag, "0")) => (tag, false),
            Some((tag, "1")) => (tag, true),
            Some(_) => return Err(Error::Invalid(path.into())),
            None => (item, true),
        };
        let b = tag.as_bytes();
        if b.len() != 4 || !b.iter().all(u8::is_ascii_alphanumeric) {
            return Err(Error::Invalid(path.into()));
        }
        out.push(([b[0], b[1], b[2], b[3]], on));
    }
    Ok(out)
}
