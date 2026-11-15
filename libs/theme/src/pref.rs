//! Volba vzhledu uzivatele (nebo systemu pro prihlasovaci obrazovku):
//! schema barev (svetle, tmave, podle denni doby) a volby Pristupnosti,
//! ktere vzhled meni (vysoky kontrast prebiji schema, omezit pohyb = zmena
//! bez animace).
//!
//! Textovy tvar (ulozeny soubor, zprava `appearance` kompozitoru; radky
//! `klic = hodnota`, `#` = komentar, CRLF se toleruje):
//!
//! ```text
//! schema = auto        # svetly | tmavy | auto
//! kontrast = ne        # ano | ne
//! omezit_pohyb = ne    # ano | ne
//! ```

use alloc::format;
use alloc::string::String;

use crate::Mode;

/// Schema barev.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Scheme {
    Light,
    Dark,
    /// Svetle od vychodu do zapadu slunce, jinak tmave.
    #[default]
    Auto,
}

impl Scheme {
    /// Slovo v ulozenem tvaru a v prikazu `vzhled`.
    pub fn word(self) -> &'static str {
        match self {
            Scheme::Light => "svetly",
            Scheme::Dark => "tmavy",
            Scheme::Auto => "auto",
        }
    }

    pub fn from_word(w: &str) -> Option<Self> {
        match w {
            "svetly" | "světlý" => Some(Scheme::Light),
            "tmavy" | "tmavý" => Some(Scheme::Dark),
            "auto" | "automaticky" => Some(Scheme::Auto),
            _ => None,
        }
    }

    /// Popisek pro uzivatele (lista, vypis prikazu).
    pub fn label(self) -> &'static str {
        match self {
            Scheme::Light => "Světlý",
            Scheme::Dark => "Tmavý",
            Scheme::Auto => "Automaticky",
        }
    }

    pub const ALL: [Scheme; 3] = [Scheme::Light, Scheme::Dark, Scheme::Auto];
}

/// Volba vzhledu.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Pref {
    pub scheme: Scheme,
    /// Vysoky kontrast (Pristupnost): prebiji schema.
    pub contrast: bool,
    /// Omezit pohyb (Pristupnost): zmena rezimu okamzite, bez prolnuti.
    pub reduce_motion: bool,
}

/// Chyba v textu volby: cislo radku (od 1) a co je spatne.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ParseError {
    pub line: usize,
    pub what: &'static str,
}

fn yes_no(v: &str) -> Option<bool> {
    match v {
        "ano" | "zap" => Some(true),
        "ne" | "vyp" => Some(false),
        _ => None,
    }
}

fn word(b: bool) -> &'static str {
    if b { "ano" } else { "ne" }
}

impl Pref {
    /// Platny rezim: kontrast, jinak schema; `day` = je den (pro `Auto`).
    pub fn resolve(&self, day: bool) -> Mode {
        if self.contrast {
            return Mode::Contrast;
        }
        match self.scheme {
            Scheme::Light => Mode::Light,
            Scheme::Dark => Mode::Dark,
            Scheme::Auto if day => Mode::Light,
            Scheme::Auto => Mode::Dark,
        }
    }

    /// Precte volbu; chybejici klice maji vychozi hodnotu, neznamy klic je
    /// chyba (preklep se neztrati potichu).
    pub fn parse(text: &str) -> Result<Self, ParseError> {
        let mut p = Pref::default();
        for (i, line) in text.lines().enumerate() {
            let line = line.split('#').next().unwrap_or("").trim();
            if line.is_empty() {
                continue;
            }
            let err = |what| ParseError { line: i + 1, what };
            let (k, v) = line.split_once('=').ok_or(err("chybi '='"))?;
            let (k, v) = (k.trim(), v.trim());
            match k {
                "schema" => p.scheme = Scheme::from_word(v).ok_or(err("schema: svetly, tmavy nebo auto"))?,
                "kontrast" => p.contrast = yes_no(v).ok_or(err("kontrast: ano nebo ne"))?,
                "omezit_pohyb" => p.reduce_motion = yes_no(v).ok_or(err("omezit_pohyb: ano nebo ne"))?,
                _ => return Err(err("neznamy klic")),
            }
        }
        Ok(p)
    }

    /// Textovy tvar (LF), ktery [`Pref::parse`] precte zpet.
    pub fn to_text(&self) -> String {
        format!("schema = {}\nkontrast = {}\nomezit_pohyb = {}\n", self.scheme.word(), word(self.contrast), word(self.reduce_motion))
    }

    /// Jednoradkovy tvar (zprava `appearance` kompozitoru):
    /// `schema kontrast omezit_pohyb`, napr. `auto ne ne`.
    pub fn to_words(&self) -> String {
        format!("{} {} {}", self.scheme.word(), word(self.contrast), word(self.reduce_motion))
    }

    /// Zmena jedne volby (prikaz `vzhled`): `schema svetly|tmavy|auto`,
    /// `kontrast ano|ne`, `pohyb ano|ne` (omezit pohyb). Chyba = popis pro
    /// uzivatele.
    pub fn apply(&mut self, change: &str) -> Result<(), &'static str> {
        let mut it = change.split_whitespace();
        let (Some(k), Some(v), None) = (it.next(), it.next(), it.next()) else {
            return Err("cekam: schema <svetly|tmavy|auto>, kontrast <ano|ne> nebo pohyb <ano|ne>");
        };
        match k {
            "schema" => self.scheme = Scheme::from_word(v).ok_or("schema: svetly, tmavy nebo auto")?,
            "kontrast" => self.contrast = yes_no(v).ok_or("kontrast: ano nebo ne")?,
            "pohyb" => self.reduce_motion = yes_no(v).ok_or("pohyb: ano (omezit) nebo ne")?,
            _ => return Err("nezname nastaveni vzhledu"),
        }
        Ok(())
    }

    pub fn from_words(s: &str) -> Option<Self> {
        let mut it = s.split_whitespace();
        let p = Pref { scheme: Scheme::from_word(it.next()?)?, contrast: yes_no(it.next()?)?, reduce_motion: yes_no(it.next()?)? };
        it.next().is_none().then_some(p)
    }
}
