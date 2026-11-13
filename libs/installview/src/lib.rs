//! Graficky instalator (`design/instalace.html`): stejne kroky jako
//! textovy pruvodce `install` v shellu, jen jako okno nad `libs/ui`.
//!
//! - Kroky, volby disku, plan, nahled zmen, mapa disku a pravidla jmena
//!   a hesla jsou v `libs/installplan`; zapis dela sluzba `installd`.
//!   Tady je jen stav pruvodce a rozhrani, zadna logika instalace navic.
//! - Se sluzbou mluvi protokolem `installplan::proto` (pozadavky odesle
//!   zapisovac `send`, odpovedi prijdou do [`App::external`]). V jednu
//!   chvili je rozpracovany nejvys jeden pozadavek, dalsi cekaji ve fronte.
//! - Rozvrzeni podle tridy sirky okna: od `medium` seznam kroku vlevo,
//!   v `compact` jen nadpis "Krok N z 8".
//! - Testovaci beh (`cargo xtask test --only instalace-gui`): [`Installer::autopilot`]
//!   vyplni odpovedi ze souboru `install-answers` a pruvodce se posouva
//!   zpravou [`AUTO_STEP`] (snimek obrazovky kazdeho kroku).

#![cfg_attr(not(test), no_std)]

extern crate alloc;

mod view;

#[cfg(test)]
mod tests;

use alloc::boxed::Box;
use alloc::collections::VecDeque;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use installplan::answers::Answers;
use installplan::config::Settings;
use installplan::map::Seg;
use installplan::proto::{AccountInput, DiskEntry, KeyboardEntry, Reply, Request};
use installplan::{Line, Mode, PROGRESS, STEPS};

/// Zprava testovaciho behu: dalsi krok pruvodce (posila ji vlakno
/// autopilota po snimku obrazovky). Nezacina kodem operace installd.
pub const AUTO_STEP: &[u8] = b"auto-krok";

/// Krok pruvodce (index do `installplan::STEPS`).
pub mod step {
    pub const KEYBOARD: usize = 0;
    pub const CHECK: usize = 1;
    pub const TARGET: usize = 2;
    pub const PREVIEW: usize = 3;
    pub const ACCOUNT: usize = 4;
    pub const SECURITY: usize = 5;
    pub const SUMMARY: usize = 6;
    pub const PROGRESS: usize = 7;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Msg {
    Next,
    Back,
    /// Vyber v seznamu rozlozeni (nahled).
    KbCursor(Vec<u64>),
    KbAdd,
    KbRemove,
    KbMain,
    Zone(String),
    /// Vyber disku (index do seznamu disku od installd).
    Disk(Vec<u64>),
    /// Zpusob instalace (index mezi dostupnymi volbami disku).
    Mode(usize),
    Confirm(String),
    Agree(bool),
    /// Vychozi system: 0 = MagnumOpus, 1 = druhy system.
    DefaultOs(usize),
    RtcLocal(bool),
    Name(String),
    Display(String),
    Password(String),
    Password2(String),
    Pin(String),
}

/// Na co se ceka od installd.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Wait {
    Keyboards,
    Disks,
    Preview,
    Install,
}

/// Nahled zmen od installd.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Preview {
    pub lines: Vec<Line>,
    pub before: Vec<Seg>,
    pub after: Vec<Seg>,
    /// Opsat nazev disku (mazani celeho disku).
    pub type_name: Option<String>,
    pub other_os: bool,
}

/// Vysledek instalace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Done(String),
    Failed(String),
}

pub struct Installer {
    send: Box<dyn FnMut(Vec<u8>)>,
    pub step: usize,
    waiting: Option<Wait>,
    queue: VecDeque<(Wait, Request)>,
    /// Chyba posledniho pozadavku (mimo instalaci).
    pub error: String,
    pub keyboards: Vec<KeyboardEntry>,
    /// Zvolena rozlozeni (indexy), prvni je hlavni.
    pub chosen: Vec<usize>,
    /// Rozlozeni v nahledu.
    pub kb_cursor: Option<usize>,
    pub zone: String,
    pub disks: Option<Vec<DiskEntry>>,
    pub disk: Option<usize>,
    pub mode: Option<Mode>,
    pub preview: Option<Preview>,
    pub confirm: String,
    pub agree: bool,
    pub default_magnum: bool,
    pub rtc_local: bool,
    pub account: AccountInput,
    pub password2: String,
    /// Posledni hlaseni prubehu (krok, text).
    pub progress: Option<(u32, String)>,
    pub outcome: Option<Outcome>,
    /// Testovaci beh: odpovedi a cislo kroku pro znacku do logu.
    auto: Option<Answers>,
    /// Krok autopilota prisel, kdyz se cekalo na installd: provede se po
    /// odpovedi.
    auto_pending: bool,
    /// Zpravy pro log (znacky kroku testovaciho behu); s [`Installer::on_log`]
    /// jdou rovnou do vystupu programu.
    pub log: Vec<String>,
    sink: Option<Box<dyn FnMut(&str)>>,
}

impl Installer {
    pub fn new(send: impl FnMut(Vec<u8>) + 'static) -> Self {
        let mut s = Self {
            send: Box::new(send),
            step: step::KEYBOARD,
            waiting: None,
            queue: VecDeque::new(),
            error: String::new(),
            keyboards: Vec::new(),
            chosen: Vec::new(),
            kb_cursor: None,
            zone: String::from("Europe/Prague"),
            disks: None,
            disk: None,
            mode: None,
            preview: None,
            confirm: String::new(),
            agree: false,
            default_magnum: true,
            rtc_local: false,
            account: AccountInput { name: String::new(), display: String::new(), password: String::new(), pin: String::new() },
            password2: String::new(),
            progress: None,
            outcome: None,
            auto: None,
            auto_pending: false,
            log: Vec::new(),
            sink: None,
        };
        s.request(Wait::Keyboards, Request::Keyboards);
        s.request(Wait::Disks, Request::Disks);
        s
    }

    /// Zpravy pro log posilat rovnou do `f` (vystup programu).
    pub fn on_log(&mut self, f: impl FnMut(&str) + 'static) {
        self.sink = Some(Box::new(f));
    }

    fn note(&mut self, line: String) {
        match &mut self.sink {
            Some(f) => f(&line),
            None => self.log.push(line),
        }
    }

    fn request(&mut self, w: Wait, r: Request) {
        self.queue.push_back((w, r));
        self.pump();
    }

    fn pump(&mut self) {
        if self.waiting.is_none()
            && let Some((w, r)) = self.queue.pop_front()
        {
            self.waiting = Some(w);
            (self.send)(r.encode());
        }
    }

    /// Ceka se na odpoved installd.
    pub fn busy(&self) -> bool {
        self.waiting.is_some()
    }

    /// Disky, na ktere jde instalovat (ne zive USB): indexy do `disks`.
    pub fn targets(&self) -> Vec<usize> {
        self.disks.as_ref().map(|d| (0..d.len()).filter(|&i| !d[i].live).collect()).unwrap_or_default()
    }

    pub fn disk_entry(&self) -> Option<&DiskEntry> {
        self.disks.as_ref().zip(self.disk).and_then(|(d, i)| d.get(i))
    }

    /// Nastaveni systemu z voleb pruvodce.
    pub fn settings(&self) -> Settings {
        Settings {
            keyboards: self.chosen.iter().map(|&i| self.keyboards[i].name.clone()).collect(),
            zone: self.zone.trim().into(),
            default_magnum: self.default_magnum,
            rtc_local: self.rtc_local,
            ..Settings::default()
        }
    }

    /// Preinstalace: ucty zustanou, krok Ucet jen informuje.
    pub fn keeps_accounts(&self) -> bool {
        self.mode == Some(Mode::Reinstall)
    }

    /// Chyba jmena uctu (prazdny = v poradku nebo jeste nevyplneno).
    pub fn name_error(&self) -> &'static str {
        let n = &self.account.name;
        if n.is_empty() || installplan::valid_user_name(n) { "" } else { "a-z, 0-9, - a _, začíná písmenem, nejvýš 32 znaků" }
    }

    pub fn password_error(&self) -> &'static str {
        let p = &self.account.password;
        if p.is_empty() || installplan::password_ok(p) { "" } else { "Aspoň 10 znaků nebo 4 slova" }
    }

    pub fn password2_error(&self) -> &'static str {
        if self.password2.is_empty() || self.password2 == self.account.password { "" } else { "Hesla se neshodují" }
    }

    /// Jde pokracovat z aktualniho kroku?
    pub fn can_next(&self) -> bool {
        if self.busy() && self.step != step::CHECK {
            return false;
        }
        match self.step {
            step::KEYBOARD => !self.chosen.is_empty() && !self.zone.trim().is_empty(),
            step::CHECK => self.disks.is_some() && !self.targets().is_empty(),
            step::TARGET => {
                self.mode.is_some_and(|m| self.disk_entry().is_some_and(|d| d.choices.iter().any(|c| c.mode == m && c.available)))
            }
            step::PREVIEW => match &self.preview {
                Some(p) => match &p.type_name {
                    Some(name) => self.confirm.trim() == name,
                    None => self.agree,
                },
                None => false,
            },
            step::ACCOUNT => {
                self.keeps_accounts()
                    || (installplan::valid_user_name(&self.account.name)
                        && installplan::password_ok(&self.account.password)
                        && self.password2 == self.account.password)
            }
            step::SECURITY | step::SUMMARY => true,
            _ => false,
        }
    }

    pub fn can_back(&self) -> bool {
        (step::CHECK..=step::SUMMARY).contains(&self.step)
    }

    /// Procenta prubehu instalace.
    pub fn percent(&self) -> u32 {
        match (&self.outcome, &self.progress) {
            (Some(Outcome::Done(_)), _) => 100,
            (_, Some((s, _))) => (*s * 100 / PROGRESS.len() as u32).min(99),
            _ => 0,
        }
    }

    fn next(&mut self) {
        if !self.can_next() {
            return;
        }
        self.error.clear();
        match self.step {
            step::TARGET => {
                // Nahled pro zvoleny disk a zpusob (stary uz neplati).
                self.preview = None;
                self.confirm.clear();
                self.agree = false;
                let (Some(d), Some(mode)) = (self.disk_entry(), self.mode) else { return };
                let disk = d.index;
                self.request(Wait::Preview, Request::Preview { disk, mode });
            }
            step::SUMMARY => {
                let Some(d) = self.disk_entry() else { return };
                let req = Request::Install {
                    disk: d.index,
                    mode: self.mode.unwrap_or(Mode::WholeDisk),
                    confirm: self.preview.as_ref().and_then(|p| p.type_name.clone()).map(|_| self.confirm.trim().into()).unwrap_or_default(),
                    settings: self.settings().to_toml(),
                    account: if self.keeps_accounts() {
                        AccountInput { name: String::new(), display: String::new(), password: String::new(), pin: String::new() }
                    } else {
                        let mut a = self.account.clone();
                        if a.display.trim().is_empty() {
                            a.display = a.name.clone();
                        }
                        a
                    },
                };
                self.request(Wait::Install, req);
            }
            _ => {}
        }
        self.step += 1;
        self.mark();
    }

    /// Znacka kroku do logu (testovaci beh: snimek obrazovky).
    fn mark(&mut self) {
        if self.auto.is_some() {
            self.note(format!("gui-install: krok {} {}", self.step + 1, STEPS[self.step]));
        }
    }

    /// Odpoved installd (`App::external`).
    pub fn reply(&mut self, data: &[u8]) -> bool {
        if data == AUTO_STEP {
            self.auto_step();
            return true;
        }
        let Ok(r) = Reply::decode(data) else {
            if self.auto.is_some() {
                self.note(format!("gui-install: neplatna zprava installd ({} B)", data.len()));
            }
            return false;
        };
        if self.auto.is_some() {
            let kind = match &r {
                Reply::Failed(..) => "chyba",
                Reply::Disks(_) => "disky",
                Reply::Keyboards(_) => "rozlozeni",
                Reply::Preview { .. } => "nahled",
                Reply::Progress { .. } => "prubeh",
                Reply::Done(_) => "hotovo",
            };
            self.note(format!("gui-install: odpoved installd: {kind}"));
        }
        if let Reply::Progress { step, text } = r {
            self.progress = Some((step, text));
            return true;
        }
        let w = self.waiting.take();
        match (w, r) {
            (_, Reply::Failed(_, text)) => {
                if w == Some(Wait::Install) {
                    self.outcome = Some(Outcome::Failed(text));
                } else {
                    self.error = text;
                }
            }
            (Some(Wait::Keyboards), Reply::Keyboards(k)) => {
                self.keyboards = k;
                if !self.keyboards.is_empty() && self.chosen.is_empty() {
                    // Vychozi rozlozeni systemu (installplan), jinak prvni.
                    let def = Settings::default().keyboards;
                    let i = self.keyboards.iter().position(|e| def.contains(&e.name)).unwrap_or(0);
                    self.chosen.push(i);
                    self.kb_cursor = Some(i);
                }
            }
            (Some(Wait::Disks), Reply::Disks(d)) => {
                self.disks = Some(d);
                let t = self.targets();
                if t.len() == 1 && self.disk.is_none() {
                    self.select_disk(t[0]);
                }
            }
            (Some(Wait::Preview), Reply::Preview { lines, before, after, type_name, other_os, .. }) => {
                self.preview = Some(Preview { lines, before, after, type_name, other_os });
            }
            (Some(Wait::Install), Reply::Done(summary)) => {
                self.progress = Some((PROGRESS.len() as u32, String::from("Hotovo")));
                self.outcome = Some(Outcome::Done(summary));
            }
            _ => self.error = String::from("neplatná odpověď instalátoru"),
        }
        if let Some(Outcome::Done(s)) = &self.outcome
            && self.auto.is_some()
        {
            let line = format!("gui-install: HOTOVO; {}", s.lines().collect::<Vec<_>>().join(" "));
            self.note(line);
        } else if let Some(Outcome::Failed(t)) = &self.outcome
            && self.auto.is_some()
        {
            let line = format!("gui-install: instalace selhala: {t}");
            self.note(line);
        }
        self.pump();
        if self.auto_pending && !self.busy() {
            self.auto_pending = false;
            self.auto_step();
        }
        true
    }

    fn select_disk(&mut self, i: usize) {
        if self.disk == Some(i) {
            return;
        }
        self.disk = Some(i);
        // Prvni dostupna volba (stejne jako vychozi v textovem pruvodci).
        self.mode = self.disk_entry().and_then(|d| d.choices.iter().find(|c| c.available).map(|c| c.mode));
        self.preview = None;
    }

    /// Dostupne volby zvoleneho disku.
    pub fn available(&self) -> Vec<&installplan::Choice> {
        self.disk_entry().map(|d| d.choices.iter().filter(|c| c.available).collect()).unwrap_or_default()
    }

    /// Testovaci beh: odpovedi ze souboru `install-answers`.
    pub fn autopilot(&mut self, a: Answers) {
        self.auto = Some(a);
        self.mark();
    }

    /// Dalsi krok autopilota: vyplni krok podle odpovedi a pokracuje.
    fn auto_step(&mut self) {
        let Some(a) = self.auto.clone() else { return };
        if self.step == step::PROGRESS {
            return;
        }
        if self.busy() && self.step != step::PROGRESS {
            self.auto_pending = true;
            return;
        }
        match self.step {
            step::KEYBOARD => {
                let s = Settings::parse(&a.settings);
                self.chosen = s.keyboards.iter().filter_map(|k| self.keyboards.iter().position(|e| &e.name == k)).collect();
                self.kb_cursor = self.chosen.first().copied();
                self.zone = s.zone;
                self.default_magnum = s.default_magnum;
                self.rtc_local = s.rtc_local;
            }
            step::TARGET => {
                let found = self.disks.as_ref().and_then(|d| d.iter().position(|d| d.model == a.disk || format!("{}", d.index) == a.disk));
                if let Some(i) = found {
                    self.select_disk(i);
                }
                self.mode = Some(a.mode);
            }
            step::PREVIEW => {
                self.confirm = a.confirm.clone();
                self.agree = true;
            }
            step::ACCOUNT => {
                self.account = a.account.clone();
                self.password2 = a.account.password.clone();
            }
            _ => {}
        }
        if !self.can_next() {
            let why = if self.error.is_empty() { String::from("krok nejde dokoncit") } else { self.error.clone() };
            self.note(format!("gui-install: autopilot stoji v kroku {}: {why}", self.step + 1));
            return;
        }
        self.next();
    }

    pub fn update_msg(&mut self, msg: Msg) {
        match msg {
            Msg::Next => self.next(),
            Msg::Back => {
                if self.can_back() {
                    self.step -= 1;
                    self.error.clear();
                }
            }
            Msg::KbCursor(sel) => self.kb_cursor = sel.first().map(|&i| i as usize).filter(|&i| i < self.keyboards.len()),
            Msg::KbAdd => {
                if let Some(i) = self.kb_cursor
                    && !self.chosen.contains(&i)
                {
                    self.chosen.push(i);
                }
            }
            Msg::KbRemove => {
                // Aspon jedno rozlozeni zustane.
                if let Some(i) = self.kb_cursor
                    && self.chosen.len() > 1
                {
                    self.chosen.retain(|&c| c != i);
                }
            }
            Msg::KbMain => {
                if let Some(i) = self.kb_cursor {
                    self.chosen.retain(|&c| c != i);
                    self.chosen.insert(0, i);
                }
            }
            Msg::Zone(z) => self.zone = z,
            Msg::Disk(sel) => {
                if let Some(&i) = sel.first() {
                    self.select_disk(i as usize);
                }
            }
            Msg::Mode(i) => self.mode = self.available().get(i).map(|c| c.mode),
            Msg::Confirm(t) => self.confirm = t,
            Msg::Agree(v) => self.agree = v,
            Msg::DefaultOs(i) => self.default_magnum = i == 0,
            Msg::RtcLocal(v) => self.rtc_local = v,
            Msg::Name(v) => self.account.name = v,
            Msg::Display(v) => self.account.display = v,
            Msg::Password(v) => self.account.password = v,
            Msg::Password2(v) => self.password2 = v,
            Msg::Pin(v) => self.account.pin = v,
        }
    }
}

impl ui::App for Installer {
    type Msg = Msg;

    fn title(&self) -> String {
        String::from("Instalace MagnumOpus")
    }

    fn view(&self, cx: &ui::Context) -> ui::Node<Msg> {
        view::view(self, cx)
    }

    fn update(&mut self, msg: Msg) {
        self.update_msg(msg);
    }

    fn external(&mut self, data: &[u8]) -> bool {
        self.reply(data)
    }
}
