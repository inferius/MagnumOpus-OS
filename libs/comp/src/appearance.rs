//! Volba vzhledu (svetly, tmavy, podle denni doby; kontrast a omezeny
//! pohyb z Pristupnosti) a plynuly prechod rezimu.
//!
//! - Volbu uzivatele posila init po otevreni relace (`appearance user`),
//!   systemovou volbu (prihlasovaci obrazovka) pri startu (`appearance
//!   system`). Zmenu z listy hlasi kompozitor zpet jako
//!   [`Out::Appearance`], init ji ulozi.
//! - Platna volba: odemcena nebo zamcena relace = volba jejiho uzivatele,
//!   prihlasovaci obrazovka = systemova.
//! - `Auto`: den od vychodu do zapadu slunce v miste casoveho pasma
//!   (`sun.rs`); dokud sluzba cas nedodala, plati den. Automaticka zmena
//!   pocka, dokud uzivatel pise ([`TYPING_PAUSE`], nejdele
//!   [`DEFER_MAX`]); zmena volby rucne plati hned.
//! - Prechod (`fade.rs`): kompozitor jednou slozi novy stav, aplikace
//!   dostanou hned konecny `Theme` a kazdy snimek se stary a novy snimek
//!   smichaji po pixelech v linearnim svetle po krivce `ease_in_out` za
//!   `motion.theme` ms (rucne) nebo `motion.theme_auto` ms (vychod a zapad
//!   slunce). Vstup se nezdrzuje a okno, ktere se behem prechodu
//!   prekresli, se do noveho snimku promitne hned.
//!   Omezit pohyb = zmena okamzite.

use alloc::collections::BTreeMap;

use theme::{Mode, Pref, Scheme};

use crate::{Comp, Out};

/// Auto zmena pocka, dokud od posledni klavesy neuplyne tato doba (ns).
pub const TYPING_PAUSE: u64 = 2_000_000_000;
/// Nejdelsi odklad auto zmeny kvuli psani (ns).
pub const DEFER_MAX: u64 = 120_000_000_000;

/// Stav vzhledu.
pub(crate) struct Appearance {
    pub system: Pref,
    /// Volba uzivatele podle uid (relace).
    pub users: BTreeMap<u64, Pref>,
    /// Misto casoveho pasma (sirka, delka); `None` = pevne hodiny.
    pub place: Option<(f32, f32)>,
    pub day: bool,
    /// Cilovy rezim (behem prolnuti uz novy).
    pub target: Mode,
    /// Prechod ceka na dalsi snimek CPU (rezim, delka ms) a bezici prechod.
    pub fade_pending: Option<(Mode, u32)>,
    pub fade: Option<crate::fade::Fade>,
    /// Posledni stisk klavesy (ns) a od kdy ceka odlozena auto zmena.
    pub last_key: u64,
    pub held_since: Option<u64>,
}

impl Default for Appearance {
    fn default() -> Self {
        Self {
            // Dokud init volbu neposle (testy, start): svetly jako drive.
            system: Pref { scheme: Scheme::Light, ..Pref::default() },
            users: BTreeMap::new(),
            place: None,
            day: true,
            target: Mode::Light,
            fade_pending: None,
            fade: None,
            last_key: 0,
            held_since: None,
        }
    }
}

impl Comp {
    /// Systemova volba (prihlasovaci obrazovka, init pri startu).
    pub fn set_appearance_system(&mut self, p: Pref, now: u64) {
        self.appearance.system = p;
        self.appearance_changed(now, false);
    }

    /// Volba uzivatele `uid` (init po otevreni relace, prikaz `vzhled`).
    pub fn set_appearance_user(&mut self, uid: u64, p: Pref, now: u64) {
        self.appearance.users.insert(uid, p);
        self.appearance_changed(now, false);
    }

    /// Uzivatel, jehoz relace je na obrazovce (odemcena, jinak zamcena
    /// zobrazena na zamykaci obrazovce); `None` = prihlasovaci obrazovka.
    pub(crate) fn appearance_uid(&self) -> Option<u64> {
        let s = &self.greeter.sessions;
        s.iter().rev().find(|s| !s.locked).or_else(|| s.iter().rev().find(|s| s.locked)).map(|s| s.uid)
    }

    /// Platna volba a ci je (`None` = systemova).
    pub fn appearance(&self) -> (Option<u64>, Pref) {
        match self.appearance_uid() {
            Some(uid) => (Some(uid), self.appearance.users.get(&uid).copied().unwrap_or(self.appearance.system)),
            None => (None, self.appearance.system),
        }
    }

    /// Misto casoveho pasma pro `Auto` (sluzba ze `tz::Zone::place`).
    pub fn set_sun_place(&mut self, place: Option<(f32, f32)>) {
        self.appearance.place = place;
    }

    /// Hodiny: `utc` (s) a mistni posun (s). Prepocita den a noc; vraci
    /// okamzik (UTC, s) dalsi zmeny.
    pub fn sun_tick(&mut self, utc: i64, offset: i32, now: u64) -> i64 {
        let (day, next) = crate::sun::is_day(utc, offset, self.appearance.place);
        if day != self.appearance.day {
            self.appearance.day = day;
            self.appearance_changed(now, true);
        }
        next
    }

    /// Volba schematu z listy: ulozi se uzivateli na obrazovce (init ji
    /// zapise, `Out::Appearance`). Na prihlasovaci obrazovce lista neni.
    pub(crate) fn choose_scheme(&mut self, s: Scheme, now: u64) {
        let (uid, mut p) = self.appearance();
        let Some(uid) = uid else { return };
        if p.scheme == s {
            return;
        }
        p.scheme = s;
        self.appearance.users.insert(uid, p);
        self.out.push(Out::Appearance { uid, pref: p });
        self.appearance_changed(now, false);
    }

    /// Klavesa (odklad auto zmeny pri psani).
    pub(crate) fn appearance_key(&mut self, now: u64) {
        self.appearance.last_key = now;
    }

    /// Prepocita cilovy rezim; `auto` = zmena dne a noci (smi se odlozit).
    pub(crate) fn appearance_changed(&mut self, now: u64, auto: bool) {
        let (_, p) = self.appearance();
        let mode = p.resolve(self.appearance.day);
        if mode == self.appearance.target {
            self.appearance.held_since = None;
            return;
        }
        if auto {
            let typing = self.appearance.last_key != 0 && now.saturating_sub(self.appearance.last_key) < TYPING_PAUSE;
            let since = *self.appearance.held_since.get_or_insert(now);
            if typing && now.saturating_sub(since) < DEFER_MAX {
                return;
            }
        }
        self.appearance.held_since = None;
        self.start_theme_transition(mode, now, p.reduce_motion, auto);
    }

    /// Zacne prechod do `mode` (s dalsim snimkem CPU, `fade.rs`);
    /// `auto` = pomaly prechod pri vychodu a zapadu slunce.
    fn start_theme_transition(&mut self, mode: Mode, now: u64, reduce: bool, auto: bool) {
        let Some(to) = self.style.theme_for(mode) else { return };
        let ms = if auto { to.motion.theme_auto } else { to.motion.theme };
        if reduce || ms == 0 || now == 0 {
            self.appearance.fade_pending = None;
            self.set_mode(mode);
            return;
        }
        self.appearance.target = mode;
        self.appearance.fade_pending = Some((mode, ms));
        self.damage_all();
    }

    /// Je prolnuti rozbehnute? (snimek kazdy takt)
    pub(crate) fn theme_animating(&self) -> bool {
        self.appearance.fade.is_some() || self.appearance.fade_pending.is_some()
    }

    /// Dalsi probuzeni kvuli odlozene auto zmene.
    pub(crate) fn appearance_wakeup(&self, now: u64) -> Option<u64> {
        self.appearance.held_since.map(|_| now + TYPING_PAUSE / 4)
    }

    /// Casovac: odlozena auto zmena (psani skoncilo nebo vyprsel odklad).
    pub(crate) fn appearance_timer(&mut self, now: u64) {
        if self.appearance.held_since.is_some() {
            self.appearance_changed(now, true);
        }
    }
}

impl Comp {
    /// Ikona vzhledu v liste podle ciloveho rezimu.
    pub(crate) fn look_icon(&self) -> &'static str {
        match self.mode() {
            Mode::Light => "sun",
            Mode::Dark => "moon",
            Mode::Contrast => "contrast",
        }
    }

    fn mode_word(m: Mode) -> &'static str {
        match m {
            Mode::Light => "světlý",
            Mode::Dark => "tmavý",
            Mode::Contrast => "vysoký kontrast",
        }
    }

    /// Bublina ikony vzhledu.
    pub(crate) fn look_tip(&self) -> alloc::string::String {
        let (_, p) = self.appearance();
        match p.scheme {
            _ if p.contrast => alloc::string::String::from("Vzhled: vysoký kontrast (Přístupnost)"),
            Scheme::Auto => alloc::format!("Vzhled: automaticky (teď {})", Self::mode_word(self.mode())),
            s => alloc::format!("Vzhled: {}", s.label()),
        }
    }

    /// Radky karty Vzhled: tri volby, pak stav.
    pub(crate) fn look_lines(&self) -> alloc::vec::Vec<alloc::string::String> {
        let (_, p) = self.appearance();
        let mut v: alloc::vec::Vec<alloc::string::String> = Scheme::ALL.iter().map(|s| alloc::string::String::from(s.label())).collect();
        if p.contrast {
            v.push(alloc::string::String::from("Vysoký kontrast je zapnutý (Přístupnost)"));
        } else if p.scheme == Scheme::Auto {
            let src = if self.appearance.place.is_some() { "podle východu a západu slunce" } else { "světlý 7-19 h" };
            v.push(alloc::format!("Teď {}, {src}", Self::mode_word(self.mode())));
        }
        v
    }
}
