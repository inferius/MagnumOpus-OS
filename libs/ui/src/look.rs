//! Vzhled prvku z tokenu design systemu (`design/tokens.toml` pres
//! `libs/theme`): barvy, typograficka skala, rozestupy, zaobleni. Zadna
//! cisla vzhledu v kodu prvku.
//!
//! Knihovna nenabizi barvy `trust_*` (patri jen systemu: odznak duvery,
//! systemove dialogy) ani `ai_*` (jen agent AI).

use gfx::text::{Family, Text};
use theme::{Mode, TextStyle, Theme};

/// Motiv a pisma okna aplikace.
pub struct Look {
    pub theme: Theme,
    pub fonts: Family,
}

impl Look {
    pub fn new(fonts: Family, mode: Mode) -> Self {
        Self { theme: Theme::builtin(mode), fonts }
    }

    /// Pisma UI z vlozenych rezu (`libs/font`, vlastnosti z tokenu).
    pub fn ui_fonts() -> Family {
        let t = Theme::builtin(Mode::Light);
        let feats: alloc::vec::Vec<[u8; 4]> = t.font.ui_features.iter().filter(|f| f.1).map(|f| f.0).collect();
        Family::new(font::inter(), &feats)
    }

    pub fn set_mode(&mut self, mode: Mode) {
        self.theme = Theme::builtin(mode);
    }

    /// Zakladni text (`type.body`).
    pub fn body(&self) -> TextStyle {
        self.theme.text.body
    }

    pub fn heading(&self) -> TextStyle {
        self.theme.text.heading
    }

    /// Text tlacitka (`body`, vaha `caption` - 500 jako `.btn`).
    pub fn button(&self) -> TextStyle {
        TextStyle { weight: self.theme.text.caption.weight, ..self.theme.text.body }
    }

    /// Text stitku (`caption`, vaha `body_strong`; `.chip` v prototypu).
    pub fn chip(&self) -> TextStyle {
        TextStyle { weight: self.theme.text.body_strong.weight, ..self.theme.text.caption }
    }

    pub fn face(&mut self, s: TextStyle) -> &mut Text {
        self.fonts.face(s.weight)
    }

    pub fn face_ref(&self, s: TextStyle) -> &Text {
        self.fonts.face_ref(s.weight)
    }

    pub fn width(&self, text: &str, s: TextStyle) -> u32 {
        self.face_ref(s).width(text, s.size)
    }

    /// Okraj okna (`s5`) a mezera mezi prvky (`s3`).
    pub fn pad(&self) -> u32 {
        self.theme.space[5]
    }

    pub fn gap(&self) -> u32 {
        self.theme.space[3]
    }

    /// Vyska ovladaciho prvku (`size.control_h`).
    pub fn control_h(&self) -> u32 {
        self.theme.size.control_h
    }

    /// Vyska radku seznamu (`size.control_h`).
    pub fn row_h(&self) -> u32 {
        self.theme.size.control_h
    }
}
