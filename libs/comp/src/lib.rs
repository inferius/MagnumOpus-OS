//! Jadro kompozitoru `compd` bez zavislosti na jadre OS.
//!
//! Sluzba (`user/compd`) prevadi kanaly a objekty pameti na volani
//! [`Comp`] a rozesila zpravy z [`Comp::take_out`]. Diky tomu jde cela
//! logika (transakce, skupiny, takt, skladani, vstup, dialogy, akce)
//! testovat na hostiteli (`cargo test -p comp`).
//!
//! - [`scene`]: klienti, okna, buffery, vrstvy, transakce.
//! - [`render`]: skladani do snimku (dekorace, odznak, lista, kurzor).
//! - [`style`]: vzhled z tokenu (`design/tokens.toml`): motiv, pisma,
//!   ikony, stiny, plocha.
//! - [`input`]: rozdeleni vstupu oknum, presun a zmena velikosti.
//! - [`dialog`]: systemove dialogy a bezpecna obrazovka.
//! - [`greeter`]: prihlasovaci a zamykaci obrazovka.
//! - [`services`]: akce aplikaci a inspekce.
//! - [`frame`]: takt snimku, animace, skupiny transakci, ping.
//! - [`gpu`]: seznam operaci pro GPU backend (`compd` + `libs/vkcomp`).
//! - [`output`]: zmena velikosti obrazovky (rezim displeje).

#![cfg_attr(not(test), no_std)]

extern crate alloc;

pub mod appearance;
pub mod clipboard;
pub mod diag;
pub mod dialog;
mod fade;
pub mod flyout;
pub mod fullscreen;
pub mod launch;
pub mod menu;
pub mod palette;
pub mod pointer;
pub mod prof;
pub mod frame;
pub mod gpu;
pub mod greeter;
pub mod input;
pub mod output;
pub mod render;
pub mod scene;
pub mod tooltip;
pub mod winsize;
pub mod services;
pub mod style;
mod sun;
mod sysui;

#[cfg(test)]
mod tests;
#[cfg(test)]
mod output_tests;

use alloc::boxed::Box;
use alloc::collections::BTreeMap;
use alloc::vec::Vec;

use gfx::damage::Damage;
use alloc::string::String;

use gfx::text::Family;
use guiproto::control::{CallerReply, DialogReply, InspectReply, Status};
use guiproto::window::ServerMsg;
use guiproto::Rect;

pub type ClientId = u32;
/// Id vlastnika kanalu `dialog`, `actions` nebo `inspect` (prideluje sluzba).
pub type PeerId = u32;

/// Vychozi perioda snimku (60 Hz) v ns, dokud ovladac nehlasi obnovovaci
/// frekvenci vystupu (`Comp::set_refresh`).
pub const PERIOD: u64 = 16_666_667;
/// Rezerva pred koncem periody pro skladani (termin pro aplikace) pri
/// 60 Hz; jinak ctvrtina periody.
pub const COMPOSE_BUDGET: u64 = 4_000_000;
/// Jak dlouho cekat na potvrzeni `CONFIGURE` nebo na skupinu transakci.
pub const SYNC_TIMEOUT: u64 = 200_000_000;

/// Pamet bufferu aplikace (sluzba ji ma namapovanou jen pro cteni).
pub trait Pixels {
    /// Pixely bufferu (`stride * h` u32); `None` = pamet neni dostupna.
    fn pixels(&self) -> Option<&[u32]>;
}

/// Buffer v pameti procesu (testy, vnitrni buffery).
impl Pixels for Vec<u32> {
    fn pixels(&self) -> Option<&[u32]> {
        Some(self)
    }
}

/// Zprava ven, kterou sluzba posle prislusnemu kanalu.
#[derive(Debug, Clone, PartialEq)]
pub enum Out {
    Client(ClientId, ServerMsg),
    Dialog(PeerId, DialogReply),
    Caller(PeerId, CallerReply),
    Inspect(PeerId, InspectReply),
    /// Uzivatel zmenil volbu vzhledu v liste: init ji ulozi
    /// (`appearance user <uid> <slova>` na kanal `prefs`).
    Appearance { uid: u64, pref: theme::Pref },
    /// Konec bezpecne obrazovky: poslat `release` na kanal `secure`.
    SecureRelease,
    /// Rozhodnuti o novem vstupnim zarizeni: `trust <id>` / `reject <id>`
    /// na kanal `secure`.
    Device { source: u16, trust: bool },
}

/// Kompozitor.
pub struct Comp {
    pub w: u32,
    pub h: u32,
    /// Vzhled: motiv z tokenu, pisma, ikony, mezipameti.
    pub(crate) style: style::Style,
    /// Hodiny v liste (cas, datum).
    pub(crate) clock: Option<(String, String)>,
    /// Sit a hlasitost v liste.
    pub(crate) status: render::BarStatus,
    /// Mezibuffer vrstvy nad okny (CPU cesta sklada stejne jako GPU).
    pub(crate) top_scratch: Vec<u32>,
    /// Rozmazane pozadi pod listou (mezipamet) a zda se pod ni neco zmenilo.
    pub(crate) bar_blur: Option<Vec<u32>>,
    pub(crate) bar_blur_dirty: bool,
    pub(crate) blur_scratch: Vec<u32>,
    /// Mereni skladani po castech (hodiny dodava sluzba).
    pub prof: prof::Prof,
    pub(crate) clients: BTreeMap<ClientId, scene::Client>,
    pub(crate) windows: BTreeMap<u32, scene::Window>,
    /// Poradi oken zdola nahoru (globalni id).
    pub(crate) z: Vec<u32>,
    pub(crate) focus: Option<u32>,
    pub(crate) next_client: ClientId,
    pub(crate) next_window: u32,
    pub(crate) dirty: Damage,
    pub(crate) out: Vec<Out>,
    pub(crate) frame: frame::FrameState,
    pub(crate) input: input::InputState,
    pub(crate) dialogs: dialog::Dialogs,
    /// Pohled palety nad knihovnou `ui` (`sysui.rs`).
    pub(crate) sysui: Option<sysui::SysUi>,
    /// Schranka (`clipboard.rs`).
    pub(crate) clipboard: clipboard::Clipboard,
    /// Generace kontextu oken (`winsize.rs`), roste s kazdou zmenou obrazovky.
    pub(crate) ctx_generation: u32,
    /// Prihlasovaci a zamykaci obrazovka.
    pub(crate) greeter: greeter::Greeter,
    pub(crate) services: services::Services,
    /// Zdroj tokenu skupin (sluzba dava nahodna cisla z jadra).
    pub(crate) token_source: Box<dyn FnMut() -> u128>,
    /// Posledni znamy cas (z volani se `now`).
    pub(crate) input_now: u64,
    /// Bezici ulohy a z toho ulohy agenta AI (ikona v panelu).
    pub(crate) tasks: (u32, u32),
    /// Radky trasovani vstupu (sluzba je vypise, `take_trace`).
    pub(crate) trace: Vec<alloc::string::String>,
    /// Poslednich [`diag::RECENT`] kroku trasovani (diagnosticky snimek).
    pub(crate) recent_input: alloc::collections::VecDeque<alloc::string::String>,
    /// Zadost o diagnosticky snimek s duvodem (vyzvedne sluzba).
    pub(crate) snapshot_request: Option<alloc::string::String>,
    pub(crate) snapshot_cfg: diag::Config,
    /// Cas posledniho automatickeho snimku (limit `interval_s`).
    pub(crate) last_auto_snapshot: Option<u64>,
    /// Otevreny panel nad listou (`flyout.rs`).
    pub(crate) flyout: Option<flyout::Flyout>,
    /// Spustitelne aplikace (id, jmeno) od initu (`launch.rs`).
    pub(crate) launchable: Vec<(alloc::string::String, alloc::string::String)>,
    /// Zadosti o spusteni aplikace (vyzvedne sluzba, posle initu).
    pub(crate) launch_requests: Vec<alloc::string::String>,
    /// Otevrena paleta prikazu (`palette.rs`) a nedavne polozky.
    pub(crate) palette: Option<palette::Palette>,
    pub(crate) palette_recent: Vec<alloc::string::String>,
    /// Cetnost pouziti polozek palety (razeni "Tato aplikace").
    pub(crate) palette_usage: palette::Usage,
    /// Zadosti palety pro init (vypnout, restartovat).
    pub(crate) system_requests: Vec<palette::SystemRequest>,
    /// Nerusit automaticky pri cele obrazovce (`fullscreen.rs`).
    pub(crate) dnd_fullscreen: bool,
    /// Nabidka u tlacitka maximalizace (okno) a od kdy je nad nim ukazatel.
    pub(crate) max_menu: Option<u32>,
    pub(crate) max_hover_since: Option<(u32, u64)>,
    /// Aplikace spustene uzivatelem (id, cas): jejich nove okno dostane
    /// fokus (`launch.rs`).
    pub(crate) expect_focus: Vec<(alloc::string::String, u64)>,
    /// Aplikace s vice okny (`multi` z manifestu).
    pub(crate) multi: Vec<alloc::string::String>,
    /// Od kdy je drzeny Esc pri cele obrazovce (`fullscreen.rs`).
    pub(crate) esc_since: Option<u64>,
    /// Zamek ukazatele: okno a do kdy ukazovat upozorneni (`pointer.rs`).
    pub(crate) plock: Option<(u32, u64)>,
    /// Bublina listy: prvek pod ukazatelem (od kdy), zobrazena, kdy zmizela.
    pub(crate) tip_hover: Option<(tooltip::Tip, u64)>,
    pub(crate) tip_shown: Option<tooltip::Tip>,
    pub(crate) tip_hidden_at: u64,
    /// Drzeny Ctrl pri otevrene palete (Ctrl+Enter).
    pub(crate) palette_ctrl: bool,
    /// Volba vzhledu a prolnuti rezimu (`appearance.rs`).
    pub(crate) appearance: appearance::Appearance,
    /// Tabulky gamma pro prechod rezimu (spocitaji se pri prvnim prechodu).
    pub(crate) fade_lut: Option<Box<fade::Lut>>,
}

impl Comp {
    /// Kompozitor obrazovky `w x h`; `fonts` = rezy pisma UI (Inter z
    /// `libs/font`, v testech zalozni pismo). Vychozi rezim je svetly.
    pub fn new(w: u32, h: u32, fonts: Family, token_source: Box<dyn FnMut() -> u128>) -> Self {
        let mut c = Self {
            w,
            h,
            style: style::Style::new(fonts, theme::Mode::Light),
            clock: None,
            status: render::BarStatus::default(),
            top_scratch: Vec::new(),
            bar_blur: None,
            bar_blur_dirty: true,
            blur_scratch: Vec::new(),
            prof: prof::Prof::default(),
            clients: BTreeMap::new(),
            windows: BTreeMap::new(),
            z: Vec::new(),
            focus: None,
            next_client: 1,
            next_window: 1,
            dirty: Damage::new(),
            out: Vec::new(),
            frame: frame::FrameState::default(),
            input: input::InputState::new(w, h),
            dialogs: dialog::Dialogs::default(),
            clipboard: clipboard::Clipboard::default(),
            sysui: None,
            ctx_generation: 1,
            greeter: greeter::Greeter::default(),
            services: services::Services::default(),
            token_source,
            input_now: 0,
            tasks: (0, 0),
            trace: Vec::new(),
            recent_input: alloc::collections::VecDeque::new(),
            snapshot_request: None,
            snapshot_cfg: diag::Config::default(),
            last_auto_snapshot: None,
            flyout: None,
            launchable: Vec::new(),
            launch_requests: Vec::new(),
            palette: None,
            palette_recent: Vec::new(),
            palette_usage: Default::default(),
            system_requests: Vec::new(),
            dnd_fullscreen: true,
            max_menu: None,
            max_hover_since: None,
            expect_focus: Vec::new(),
            multi: Vec::new(),
            palette_ctrl: false,
            esc_since: None,
            plock: None,
            tip_hover: None,
            tip_shown: None,
            tip_hidden_at: 0,
            appearance: appearance::Appearance::default(),
            fade_lut: None,
        };
        c.damage_all();
        c
    }

    /// Radky trasovani vstupu od posledniho volani ("vstup: id=..."; cil
    /// logu `vstup`, `docs/cs/gui.md`).
    pub fn take_trace(&mut self) -> Vec<alloc::string::String> {
        core::mem::take(&mut self.trace)
    }

    /// Zapise krok trasovani kliku `time` (cas zaznamu z `inputd` = id).
    pub(crate) fn trace(&mut self, time: u64, text: core::fmt::Arguments<'_>) {
        // Bez odberu (testy) jen posledni radky.
        if self.trace.len() >= 64 {
            self.trace.remove(0);
        }
        let line = alloc::format!("vstup: id={} compd {text}", time / 1000);
        diag::remember(&mut self.recent_input, &line);
        self.trace.push(line);
    }

    /// Popis okna pro trasovani: aplikace a id.
    pub(crate) fn window_name(&self, g: u32) -> alloc::string::String {
        let app = self.windows.get(&g).and_then(|w| self.clients.get(&w.client)).map_or("?", |c| c.info.id.as_str());
        alloc::format!("okno {g} ({app})")
    }

    pub fn screen(&self) -> Rect {
        Rect::new(0, 0, self.w, self.h)
    }

    pub(crate) fn damage(&mut self, r: Rect) {
        let s = self.screen();
        if !r.intersect(&self.blur_source()).is_empty() {
            // Rozmazane pozadi se spocita znovu: prekreslit celou listu,
            // jinak by zbytek listy mel stare pozadi.
            self.bar_blur_dirty = true;
            let p = self.panel_visual();
            self.dirty.add(p, s);
        }
        self.dirty.add(r, s);
    }

    /// Poskozeni jen vrstvy nad okny (lista, kurzor): pozadi pod listou se
    /// nemeni, rozmazani se nepocita znovu.
    pub(crate) fn damage_top(&mut self, r: Rect) {
        let s = self.screen();
        self.dirty.add(r, s);
    }

    pub(crate) fn damage_all(&mut self) {
        let s = self.screen();
        self.dirty.add(s, s);
    }

    pub(crate) fn send(&mut self, c: ClientId, m: ServerMsg) {
        self.out.push(Out::Client(c, m));
    }

    /// Zpravy k odeslani (vyprazdni frontu).
    pub fn take_out(&mut self) -> Vec<Out> {
        core::mem::take(&mut self.out)
    }

    /// Barva pozadi plochy (prvni snimek pri pripojeni displeje).
    pub fn background(&self) -> u32 {
        self.style.theme.color.bg
    }

    /// Obraz pozadi plochy pro aktualni velikost (spocita se jednou, pak
    /// z mezipameti). Pouziva ho i prechod ze startovni obrazovky, aby se
    /// stejny obraz nepocital dvakrat.
    pub fn backdrop(&mut self) -> alloc::sync::Arc<Vec<u32>> {
        let (w, h) = (self.w, self.h);
        self.style.backdrop_shared(w, h)
    }

    /// Prekreslit celou obrazovku (napr. po zmene backendu skladani).
    pub fn redraw_all(&mut self) {
        self.damage_all();
    }

    /// Pocet bezicich uloh pro ikonu v panelu (od sluzby uloh).
    pub fn set_tasks(&mut self, running: u32, agent: u32) {
        if self.tasks != (running, agent) {
            // Bublina a panel nad listou ukazuji totez cislo: prekreslit je
            // pred i po zmene (jinak by zustalo stare cislo).
            let old = [self.tip_rect(), self.flyout_rect()];
            self.tasks = (running, agent);
            for r in old.into_iter().chain([self.tip_rect(), self.flyout_rect()]).flatten() {
                self.damage(r);
            }
            self.damage_panel();
        }
    }

    pub fn status(&self) -> Status {
        Status {
            clients: self.clients.len() as u32,
            windows: self.windows.len() as u32,
            frames: self.frame.frames,
            presented: self.frame.presented,
            dialogs: self.dialogs.len() as u32,
            secure: self.dialogs.secure_active(),
            // Prechod ze startovni obrazovky ridi sluzba (compd).
            splash: false,
        }
    }
}
