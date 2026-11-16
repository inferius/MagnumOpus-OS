//! Sluzba relaci: relace (interpret, pracovni slozka, historie, undo,
//! ulohy) a bloky vystupu patri sluzbe, ne oknu terminalu.
//!
//! Okno se k relaci pripoji (`Attach`), posila radky (`Run`) a dostava
//! bloky vystupu. Po odpojeni (`Detach`, zavreni nebo pad okna) relace bezi
//! dal: rozbehnuty prikaz dobehne, vystup se ulozi do bloku a po novem
//! pripojeni se prehraje. Dotaz (nahled zmen, zamek) bez pripojeneho okna
//! se bezpecne odmitne.

mod line;

use alloc::boxed::Box;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;

use shlang::eval::Interp;
use shlang::host::{AudioReq, ExecNeeds, Fs, Host, Opened, TaskDone, TaskSpec};
use shlang::sig::Builtins;
use shlang::value::Value;
use shlang::{ShError, Style, error::code};

use crate::proto::{ClientMsg, Item, ServerMsg, VERSION};
use crate::taskboard::{self, Board, Chan, Local};
use crate::tasks::{self, Clock, Joinable, Notify};

/// Nejdelsi cekani v `await` mezi kontrolami okna (Ctrl-C).
const WAIT_STEP: i64 = 50_000_000;
/// Prehrani relace po pripojeni: kolik poslednich bloku jde pred
/// `Attached` (viditelna cast) a po kolika se posilaji starsi.
const REPLAY_VISIBLE: usize = 12;
const REPLAY_BATCH: usize = 16;

/// Jeden blok prehrani: zacatek, polozky, konec (pokud dobehl).
fn replay_block(link: &mut dyn Link, session: u64, b: &Block) {
    link.send(&ServerMsg::BlockStart { session, block: b.id, line: b.line.clone() });
    for it in &b.items {
        link.send(&ServerMsg::Item { session, block: b.id, item: it.clone() });
    }
    if let Some(ok) = b.ok {
        link.send(&ServerMsg::BlockEnd { session, block: b.id, ok, duration: b.duration });
    }
}

/// Spojeni s oknem terminalu (kanal v systemu, fronta v testech).
pub trait Link {
    fn send(&mut self, msg: &ServerMsg);
    /// Zprava, pokud nejaka ceka (bez cekani).
    fn poll(&mut self) -> Option<ClientMsg>;
    /// Pocka na zpravu; `None` = okno je pryc.
    fn wait(&mut self) -> Option<ClientMsg>;
}

/// Zbytek prostredi (cas, spousteni programu, vlakna uloh).
///
/// Zprava instalatoru pro prikaz `install` (obsah = `installplan::proto::Reply`;
/// term ho nerozebira, jen rozlisi prubeh od konecne odpovedi).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstallerMsg {
    Progress(Vec<u8>),
    Done(Vec<u8>),
}

/// Ulohy `async` bezi ve vlaknech jen kdyz prostredi umi vsechny ctyri
/// casti (`spawn`, `task_fs`, `notifier`, `clock`); jinak probehnou na
/// miste (postupne, stejny vysledek).
pub trait Env {
    fn now(&mut self) -> Option<i64>;
    fn sleep(&mut self, ns: i64);
    fn exec(&mut self, _name: &str, _args: &[(Option<String>, Value)], _input: Value, _needs: &ExecNeeds)
        -> Option<Result<Value, ShError>> {
        None
    }
    /// Spusti vlakno.
    fn spawn(&mut self, _job: Box<dyn FnOnce() + Send>) -> Option<Box<dyn Joinable>> {
        None
    }
    /// Nova relace FS pro ulohu (vlastni koncept, stejna prava).
    fn task_fs(&mut self, _task: u64) -> Option<Box<dyn Fs + Send>> {
        None
    }
    /// Probouzeni hlavniho vlakna (uloha neco poslala nebo dobehla).
    fn notifier(&mut self) -> Option<Arc<dyn Notify>> {
        None
    }
    /// Nove probouzeni pro jednu ulohu (odpoved na vyzvu, zruseni).
    fn new_notifier(&mut self) -> Option<Arc<dyn Notify>> {
        None
    }
    /// Zprava do prehledu uloh (`taskd`); `false` = kanal neni.
    fn task_send(&mut self, _to: Chan, _data: Vec<u8>) -> bool {
        false
    }
    /// Prijate zpravy prehledu uloh (bez cekani).
    fn task_recv(&mut self) -> Vec<(Chan, Vec<u8>)> {
        Vec::new()
    }
    fn clock(&self) -> Option<Clock> {
        None
    }
    /// Zvuk (prikaz `audio`); `None` = relace zvukovou sluzbu nema.
    fn audio(&mut self, _req: AudioReq) -> Option<Result<Value, ShError>> {
        None
    }
    /// Vypnuti / restart systemu (prikazy `shutdown`, `reboot`); vychozi =
    /// relace napajeni nema.
    fn power(&mut self, _reboot: bool) -> Result<(), abi::Error> {
        Err(abi::Error::NotSupported)
    }
    /// Instalator (prikaz `install`): poslani pozadavku; vychozi = relace
    /// instalator nema.
    fn installer_send(&mut self, _req: &[u8]) -> Result<(), abi::Error> {
        Err(abi::Error::NotSupported)
    }
    /// Dalsi zprava instalatoru (prubeh nebo konecna odpoved), cekani
    /// nejvys `wait_ms`; `None` = zatim nic. Kratke cekani: radek ve
    /// vlakne vola pres hlavni vlakno a to mezitim obsluhuje okno.
    fn installer_recv(&mut self, _wait_ms: u64) -> Result<Option<InstallerMsg>, abi::Error> {
        Err(abi::Error::NotSupported)
    }
    /// Balicek pro vyvojare (prikaz `report`); vychozi = relace sluzbu
    /// logu nema.
    fn report(&mut self) -> Result<String, abi::Error> {
        Err(abi::Error::NotSupported)
    }
    /// Uroven logu (prikaz `loglevel`); vychozi = relace ji nema.
    fn log_level(&mut self, _target: &str, _level: &str) -> Result<String, abi::Error> {
        Err(abi::Error::NotSupported)
    }
    /// Volba vzhledu (prikaz `theme`, `shlang::host::Host::appearance`);
    /// vychozi = relace ji nema.
    fn appearance(&mut self, _system: bool, _change: &str) -> Result<String, abi::Error> {
        Err(abi::Error::NotSupported)
    }
    /// Povoleni aplikaci (prikaz `permissions`); vychozi = relace je nema.
    fn permissions(&mut self, _app: &str, _perm: &str) -> Result<String, abi::Error> {
        Err(abi::Error::NotSupported)
    }
    /// Zmena v `/system` (prikaz `system`, `shlang::host::Host::system_file`);
    /// vychozi = relace ji nema.
    fn system_file(&mut self, _op: u8, _path: &str, _data: &[u8]) -> Result<(), abi::Error> {
        Err(abi::Error::NotSupported)
    }
    /// Rezim vyvojare (prikaz `devmode`, jen vyvojove sestaveni).
    #[cfg(feature = "dev-mode")]
    fn dev_mode(&mut self, _set: u8) -> Result<bool, abi::Error> {
        Err(abi::Error::NotSupported)
    }
    /// Vzdaleny pristup SSH (`shlang::host::Host::ssh`); vychozi = relace
    /// ho nema.
    fn ssh(&mut self, _op: &str, _arg: &str, _password: &str) -> Result<alloc::string::String, abi::Error> {
        Err(abi::Error::NotSupported)
    }
    /// Ucty lidi pro jmena v prikazu `acl`; vychozi = relace je nema
    /// (`acl` pak zna jen `everyone`, `users` a cisla).
    fn accounts(&mut self) -> Result<alloc::vec::Vec<shlang::acl::Account>, abi::Error> {
        Err(abi::Error::NotSupported)
    }
    /// Sluzba "otevri cestu" (`start`, hola cesta k souboru): poslani
    /// zadosti; vychozi = relace sluzbu nema.
    fn open_send(&mut self, _path: &str, _app_only: bool) -> Result<(), abi::Error> {
        Err(abi::Error::NotSupported)
    }
    /// Odpoved sluzby, cekani nejvys `wait_ms`; `None` = zatim nic (kratke
    /// cekani jako u instalatoru: aplikace muze startovat i 20 s).
    fn open_recv(&mut self, _wait_ms: u64) -> Result<Option<Opened>, abi::Error> {
        Err(abi::Error::NotSupported)
    }
}

/// Kanaly prehledu uloh pres `Env`.
struct EnvLink<'a>(&'a mut dyn Env);

impl taskboard::Link for EnvLink<'_> {
    fn send(&mut self, to: Chan, data: Vec<u8>) -> bool {
        self.0.task_send(to, data)
    }
    fn recv(&mut self) -> Vec<(Chan, Vec<u8>)> {
        self.0.task_recv()
    }
}

/// Konce bloku uloh `async` z `log` do prehledu uloh (hotovo, chyba,
/// zruseno podle polozek bloku).
fn end_tasks(board: &mut Board, env: &mut dyn Env, log: &[ServerMsg]) {
    for m in log {
        let ServerMsg::BlockEnd { block, ok, .. } = m else { continue };
        let local = Local::Async(*block);
        if !board.known(local) {
            continue;
        }
        let err = log.iter().rev().find_map(|x| match x {
            ServerMsg::Item { block: b, item: Item::Error { code: c, message, .. }, .. } if b == block => {
                Some((c.clone(), message.clone()))
            }
            _ => None,
        });
        let state = match (ok, err) {
            (true, _) => taskproto::State::Done(String::new()),
            (false, Some((c, _))) if c == code::INTERRUPTED => taskproto::State::Cancelled,
            (false, Some((_, m))) => taskproto::State::Failed { code: 0, message: m },
            (false, None) => taskproto::State::Failed { code: 0, message: String::from("chyba") },
        };
        board.finish(&mut EnvLink(env), local, state);
    }
}

/// Pozadavky taskd na ulohy relace (zrusit): priznak ulohy `async`.
fn apply_controls(board: &mut Board, table: &tasks::Shared) {
    for (local, c) in core::mem::take(&mut board.controls) {
        if let (Local::Async(b), taskproto::Control::Cancel(_)) = (local, c) {
            tasks::request_cancel(table, b);
        }
    }
}

/// Blok vystupu jednoho prikazu.
#[derive(Debug, Clone, PartialEq)]
pub struct Block {
    pub id: u64,
    pub line: String,
    pub items: Vec<Item>,
    /// `None` = prikaz jeste bezi.
    pub ok: Option<bool>,
    pub duration: i64,
}

pub struct Session {
    pub id: u64,
    pub interp: Interp,
    pub blocks: Vec<Block>,
    pub attached: bool,
    /// Ulohy `async` ve vlaknech.
    pub tasks: tasks::Shared,
    /// Radek ve vlakne v poprede a ulohy na pozadi (Ctrl+Z, `&`).
    pub fg: Option<line::LineRun>,
    pub bg: Vec<line::LineRun>,
    /// Radky poslane behem radku v poprede (pobezi po nem).
    pub queue: Vec<String>,
    /// Cislo dalsi ulohy na pozadi (`jobs`, `fg N`).
    pub next_bg: u64,
}

impl Drop for Session {
    fn drop(&mut self) {
        tasks::cancel_all(&self.tasks);
        line::cancel_all(&self.fg, &self.bg);
    }
}

/// Zprava ulohy do bloku relace (blok ulohy je samostatny).
fn apply(blocks: &mut Vec<Block>, keep: usize, m: ServerMsg) {
    match m {
        ServerMsg::BlockStart { block, line, .. } => {
            blocks.push(Block { id: block, line, items: Vec::new(), ok: None, duration: 0 });
            if blocks.len() > keep {
                blocks.remove(0);
            }
        }
        ServerMsg::Item { block, item, .. } => {
            if let Some(b) = blocks.iter_mut().rev().find(|b| b.id == block) {
                b.items.push(item);
            }
        }
        ServerMsg::ItemUpdate { block, index, item, .. } => {
            if let Some(b) = blocks.iter_mut().rev().find(|b| b.id == block)
                && let Some(slot) = b.items.get_mut(index as usize)
            {
                *slot = item;
            }
        }
        ServerMsg::BlockEnd { block, ok, duration, .. } => {
            if let Some(b) = blocks.iter_mut().rev().find(|b| b.id == block) {
                b.ok = Some(ok);
                b.duration = duration;
            }
        }
        _ => {}
    }
}

/// Zpravy uloh: oknu (je-li pripojene) a do `log` (bloky relace).
/// Vyzva bez okna se odmitne (jako u radku).
fn pump(t: &tasks::Shared, next_prompt: &mut u64, attached: bool, link: &mut dyn Link, log: &mut Vec<ServerMsg>) {
    for m in tasks::drain(t, next_prompt) {
        if let ServerMsg::Prompt { prompt, .. } = &m {
            if attached {
                link.send(&m);
            } else {
                tasks::answer(t, *prompt, false);
            }
            continue;
        }
        if attached {
            link.send(&m);
        }
        log.push(m);
    }
}

pub struct Service {
    pub sessions: Vec<Session>,
    next_session: u64,
    next_block: u64,
    next_prompt: u64,
    pub home: String,
    /// Kolik bloku si relace pamatuje.
    pub keep_blocks: usize,
    /// Zprava, ktere prisly behem prikazu (zpracuji se potom).
    deferred: Vec<ClientMsg>,
    /// Rozpocet zasobniku interpretu (podle zasobniku sluzby).
    pub stack_budget: usize,
    /// Rozpocet zasobniku interpretu ve vlakne ulohy.
    pub task_stack: usize,
    /// Proces jedne relace (`shsess`): jedine povolene cislo relace.
    fixed: Option<u64>,
    /// Prehled uloh (`taskd`): hlasene ulohy a seznam pro okno.
    pub board: Board,
    /// Radky bezi ve vlaknech (Ctrl+Z, `&` na pozadi); zapina proces
    /// relace. Bez toho (testy) radek bezi v hlavnim vlakne.
    pub line_threads: bool,
}

/// Zvyrazneni jako zprava protokolu.
pub fn highlighted(seq: u32, h: &shlang::highlight::Highlight) -> ServerMsg {
    let spans = h.spans.iter().map(|s| (s.start as u32, (s.end - s.start) as u32, s.class as u8)).collect();
    let error = h.error.as_ref().map(|e| (e.start as u32, (e.end - e.start) as u32, e.message.clone()));
    ServerMsg::Highlighted { seq, spans, error }
}

/// Spojeni bez okna (uklid ve volne chvili nic neposila).
struct NullLink;

impl Link for NullLink {
    fn send(&mut self, _msg: &ServerMsg) {}
    fn poll(&mut self) -> Option<ClientMsg> {
        None
    }
    fn wait(&mut self) -> Option<ClientMsg> {
        None
    }
}

/// Hostitel interpretu pro jeden radek relace.
struct SessionHost<'a> {
    fs: &'a mut dyn Fs,
    link: &'a mut dyn Link,
    env: &'a mut dyn Env,
    session: u64,
    block: &'a mut Block,
    attached: &'a mut bool,
    interrupt: bool,
    next_prompt: &'a mut u64,
    deferred: &'a mut Vec<ClientMsg>,
    home: &'a str,
    polls: u32,
    /// Pruhy prubehu radku: (id od interpretu, poradi polozky v bloku).
    bars: Vec<(u32, usize)>,
    tasks: &'a tasks::Shared,
    next_block: &'a mut u64,
    task_stack: usize,
    /// Zpravy uloh pro bloky relace (zapisou se po radku).
    log: Vec<ServerMsg>,
    board: &'a mut Board,
}

impl SessionHost<'_> {
    /// Pruh prubehu: prvni hlaseni prida polozku, dalsi ji zmeni na miste.
    fn bar(&mut self, id: u32, done: u64, total: u64, label: &str) {
        let item = Item::Progress { done, total, label: label.into() };
        match self.bars.iter().find(|(b, _)| *b == id).map(|(_, i)| *i) {
            Some(index) => {
                self.block.items[index] = item.clone();
                if *self.attached {
                    self.link.send(&ServerMsg::ItemUpdate {
                        session: self.session,
                        block: self.block.id,
                        index: index as u32,
                        item,
                    });
                }
            }
            None => {
                self.bars.push((id, self.block.items.len()));
                self.push(item);
            }
        }
    }

    /// Vyzva a cekani na odpoved; `None` = zruseno (Ctrl-C, odpojeni).
    fn ask(&mut self, question: &str, detail: Item, choices: &[String]) -> Option<ClientMsg> {
        let id = *self.next_prompt;
        *self.next_prompt += 1;
        self.link.send(&ServerMsg::Prompt { prompt: id, question: question.into(), detail, choices: choices.to_vec() });
        loop {
            match self.link.wait() {
                Some(m @ ClientMsg::Answer { prompt, .. }) | Some(m @ ClientMsg::Choose { prompt, .. }) if prompt == id => {
                    return Some(m);
                }
                Some(m @ (ClientMsg::Interrupt { .. } | ClientMsg::Detach { .. })) => {
                    self.incoming(m);
                    return None;
                }
                Some(other) => self.deferred.push(other),
                None => {
                    *self.attached = false;
                    return None;
                }
            }
        }
    }
    fn push(&mut self, item: Item) {
        // Velka binarni data jen jako souhrn (zprava kanalu ma nejvys 64 KiB).
        let item = match item {
            Item::Value(v) => Item::Value(crate::binary::for_window(v)),
            other => other,
        };
        if *self.attached {
            self.link.send(&ServerMsg::Item { session: self.session, block: self.block.id, item: item.clone() });
        }
        self.block.items.push(item);
    }

    fn pump_tasks(&mut self) {
        pump(self.tasks, self.next_prompt, *self.attached, self.link, &mut self.log);
        // Odpoved, ktera prisla driv nez se vyzva ulohy zaregistrovala.
        let tasks = self.tasks;
        self.deferred.retain(|m| match m {
            ClientMsg::Answer { prompt, yes } => !tasks::answer(tasks, *prompt, *yes),
            _ => true,
        });
    }

    /// Zpracuje zpravu, ktera prisla behem prikazu.
    fn incoming(&mut self, m: ClientMsg) {
        if let ClientMsg::Answer { prompt, yes } = &m
            && tasks::answer(self.tasks, *prompt, *yes)
        {
            return;
        }
        match m {
            ClientMsg::Interrupt { session } if session == self.session => self.interrupt = true,
            ClientMsg::Detach { session } if session == self.session => *self.attached = false,
            other => self.deferred.push(other),
        }
    }
}

impl Host for SessionHost<'_> {
    fn fs(&mut self) -> &mut dyn Fs {
        self.fs
    }

    fn emit(&mut self, value: Value) {
        self.push(Item::Value(value));
    }

    fn note(&mut self, text: &str) {
        self.push(Item::Note(text.into()));
    }

    fn now(&mut self) -> Option<i64> {
        self.env.now()
    }

    fn sleep(&mut self, ns: i64) {
        // Spi po kouscich, aby slo prerusit; delsi cekani s pruhem prubehu.
        let step = 50_000_000;
        let mut left = ns;
        let long = ns >= 1_000_000_000;
        let ms = |v: i64| (v.max(0) / 1_000_000) as u64;
        while left > 0 && !self.interrupted() {
            if long {
                self.bar(0, ms(ns - left), ms(ns), "sleep");
            }
            self.env.sleep(left.min(step));
            left -= step;
        }
        if long {
            self.bar(0, ms(ns), ms(ns), "sleep");
        }
    }

    fn progress(&mut self, id: u32, done: u64, total: u64, label: &str) {
        self.bar(id, done, total, label);
    }

    fn choose(&mut self, question: &str, choices: &[String]) -> Option<usize> {
        if !*self.attached || choices.is_empty() {
            return None;
        }
        match self.ask(question, Item::Note(String::new()), choices)? {
            ClientMsg::Choose { index, .. } if (index as usize) < choices.len() => {
                self.block.items.push(Item::Note(alloc::format!("{question} -> {}", choices[index as usize])));
                Some(index as usize)
            }
            _ => None,
        }
    }

    fn interrupted(&mut self) -> bool {
        if self.interrupt {
            return true;
        }
        self.polls = self.polls.wrapping_add(1);
        if self.polls % 8 == 0 && *self.attached {
            while let Some(m) = self.link.poll() {
                self.incoming(m);
            }
        }
        self.interrupt
    }

    fn ask(&mut self, question: &str, secret: bool, default: &str) -> Option<String> {
        if !*self.attached {
            return None;
        }
        let id = *self.next_prompt;
        *self.next_prompt += 1;
        self.link.send(&ServerMsg::Ask { prompt: id, question: question.into(), secret, default: default.into() });
        loop {
            match self.link.wait() {
                Some(ClientMsg::Text { prompt, text }) if prompt == id => {
                    // Heslo do vypisu bloku nepatri.
                    let shown = if secret { String::from("***") } else { text.clone() };
                    self.block.items.push(Item::Note(alloc::format!("{question} -> {shown}")));
                    return Some(text);
                }
                Some(m @ (ClientMsg::Interrupt { .. } | ClientMsg::Detach { .. })) => {
                    self.incoming(m);
                    return None;
                }
                Some(other) => self.deferred.push(other),
                None => {
                    *self.attached = false;
                    return None;
                }
            }
        }
    }

    fn confirm(&mut self, question: &str, detail: &Value) -> bool {
        if !*self.attached {
            self.push(Item::Note(alloc::format!("{question} - okno neni pripojene, odmitnuto")));
            return false;
        }
        let yes = match self.ask(question, Item::Value(detail.clone()), &[]) {
            Some(ClientMsg::Answer { yes, .. }) => yes,
            Some(ClientMsg::Choose { index, .. }) => index == 0,
            _ => false,
        };
        self.block.items.push(Item::Note(alloc::format!("{question} -> {}", if yes { "ano" } else { "ne" })));
        yes
    }

    fn exec(&mut self, name: &str, args: &[(Option<String>, Value)], input: Value, needs: &ExecNeeds)
        -> Option<Result<Value, ShError>> {
        self.env.exec(name, args, input, needs)
    }

    fn home(&self) -> String {
        self.home.into()
    }

    fn audio(&mut self, req: AudioReq) -> Option<Result<Value, ShError>> {
        self.env.audio(req)
    }

    fn power(&mut self, reboot: bool) -> Result<(), abi::Error> {
        self.env.power(reboot)
    }

    fn report(&mut self) -> Result<String, abi::Error> {
        self.env.report()
    }

    fn installer(&mut self, req: &[u8], progress: &mut dyn FnMut(&[u8])) -> Result<Vec<u8>, abi::Error> {
        self.env.installer_send(req)?;
        loop {
            match self.env.installer_recv(1000)? {
                Some(InstallerMsg::Progress(m)) => progress(&m),
                Some(InstallerMsg::Done(m)) => return Ok(m),
                None => {}
            }
        }
    }

    fn log_level(&mut self, target: &str, level: &str) -> Result<String, abi::Error> {
        self.env.log_level(target, level)
    }

    fn appearance(&mut self, system: bool, change: &str) -> Result<String, abi::Error> {
        self.env.appearance(system, change)
    }

    fn permissions(&mut self, app: &str, perm: &str) -> Result<String, abi::Error> {
        self.env.permissions(app, perm)
    }

    fn system_file(&mut self, op: u8, path: &str, data: &[u8]) -> Result<(), abi::Error> {
        self.env.system_file(op, path, data)
    }

    #[cfg(feature = "dev-mode")]
    fn dev_mode(&mut self, set: u8) -> Result<bool, abi::Error> {
        self.env.dev_mode(set)
    }

    fn accounts(&mut self) -> Result<Vec<shlang::acl::Account>, abi::Error> {
        self.env.accounts()
    }

    fn ssh(&mut self, op: &str, arg: &str, password: &str) -> Result<String, abi::Error> {
        self.env.ssh(op, arg, password)
    }

    fn open_path(&mut self, path: &str, app_only: bool) -> Result<Opened, abi::Error> {
        self.env.open_send(path, app_only)?;
        loop {
            if let Some(r) = self.env.open_recv(200)? {
                return Ok(r);
            }
            if self.interrupted() {
                return Err(abi::Error::Canceled);
            }
        }
    }

    fn task_spawn(&mut self, spec: TaskSpec) -> Option<Result<u64, ShError>> {
        let wire = match tasks::SpecWire::of(&spec) {
            Ok(w) => w,
            Err(e) => return Some(Err(e)),
        };
        let mut c = crate::fg::MainCtx {
            env: &mut *self.env,
            board: &mut *self.board,
            link: &mut *self.link,
            attached: *self.attached,
            tasks: self.tasks,
            next_block: &mut *self.next_block,
            log: &mut self.log,
            session: self.session,
            home: self.home,
            task_stack: self.task_stack,
        };
        spawn_task(&mut c, wire)
    }

    fn task_wait(&mut self, ids: &[u64], any: bool, deadline: Option<i64>) -> Result<Vec<TaskDone>, ShError> {
        let notify = self.env.notifier();
        loop {
            self.pump_tasks();
            // Zruseni z prehledu uloh (panel, GUI) i behem cekani.
            self.board.pump(&mut EnvLink(self.env));
            apply_controls(self.board, self.tasks);
            if let Some(bad) = ids.iter().find(|i| !tasks::exists(self.tasks, **i)) {
                return Err(ShError::new(code::ARGS, alloc::format!("uloha {bad} neexistuje")));
            }
            let done: Vec<u64> = ids.iter().copied().filter(|i| tasks::is_done(self.tasks, *i)).collect();
            let failed = !any && done.iter().any(|i| tasks::failed(self.tasks, *i));
            if (any && !done.is_empty()) || done.len() == ids.len() || failed {
                let out = done.into_iter().filter_map(|i| tasks::take(self.tasks, i)).collect();
                // Posledni zpravy vyzvednutych uloh (konec bloku) hned oknu.
                self.pump_tasks();
                return Ok(out);
            }
            // Termin nejdriv: vyzvednuti bez cekani (volna chvile, `reap`)
            // nesmi cist zpravy okna - radek nebo Ctrl-C by se odlozil
            // a zrusil ulohy na pozadi.
            let now = self.env.now();
            let mut step = WAIT_STEP;
            if let Some(d) = deadline {
                match now {
                    Some(n) if n < d => step = step.min(d - n),
                    _ => return Err(ShError::sys(abi::Error::TimedOut, "await: cas vyprsel (ulohy bezi dal)")),
                }
            }
            // Okno kazde kolo (Ctrl-C, odpovedi na vyzvy uloh).
            if *self.attached {
                while let Some(m) = self.link.poll() {
                    self.incoming(m);
                }
            }
            if self.interrupt {
                for i in ids {
                    tasks::cancel(self.tasks, *i);
                }
                return Err(ShError::new(code::INTERRUPTED, "await prerusen (Ctrl-C), ulohy zruseny"));
            }
            match &notify {
                Some(n) => n.wait(step),
                None => self.env.sleep(step),
            }
        }
    }

    fn tasks(&mut self, op: shlang::host::TasksOp) -> Option<Result<Value, ShError>> {
        tasks_op(self.board, self.env, op)
    }

    fn task_cancel(&mut self, id: u64) -> Option<TaskDone> {
        let d = tasks::cancel(self.tasks, id);
        self.pump_tasks();
        d
    }
}

pub(crate) fn tasks_op(board: &mut Board, env: &mut dyn Env, op: shlang::host::TasksOp) -> Option<Result<Value, ShError>> {
    use shlang::host::TasksOp;
    board.pump(&mut EnvLink(&mut *env));
    if !board.available() {
        return None;
    }
    let (req, what) = match op {
        TasksOp::List => {
            // Vlastni prave nahlasene ulohy maji byt videt: chvili pockat
            // na potvrzeni registrace a udalost (nejvys 1 s).
            let end = env.now().map(|n| n + 1_000_000_000);
            while !board.settled() {
                match (end, env.now()) {
                    (Some(e), Some(n)) if n < e => {}
                    _ => break,
                }
                env.sleep(5_000_000);
                board.pump(&mut EnvLink(&mut *env));
            }
            let rows = board.list.tasks.iter().map(taskboard::record).collect();
            return Some(Ok(Value::List(rows)));
        }
        TasksOp::Cancel(id) => (board.action(&mut EnvLink(&mut *env), id, taskboard::Action::Cancel), "zruseni"),
        TasksOp::Pause(id) => (board.action(&mut EnvLink(&mut *env), id, taskboard::Action::Pause), "pozastaveni"),
        TasksOp::Resume(id) => (board.action(&mut EnvLink(&mut *env), id, taskboard::Action::Resume), "pokracovani"),
        TasksOp::Log(id, n) => (board.log(&mut EnvLink(&mut *env), id, n), "vystup"),
    };
    let req = req?;
    // Odpoved taskd (kratke cekani; o pravech rozhoduje taskd).
    let end = env.now().map(|n| n + 3_000_000_000);
    loop {
        board.pump(&mut EnvLink(&mut *env));
        if let Some(r) = board.result(req) {
            return Some(match r {
                Ok(()) => Ok(Value::Nothing),
                Err(e) => Err(ShError::sys(e, alloc::format!("tasks: {what} ulohy"))),
            });
        }
        if let Some(lines) = board.log_lines(req) {
            return Some(Ok(Value::List(lines.into_iter().map(Value::Str).collect())));
        }
        match (end, env.now()) {
            (Some(e), Some(n)) if n >= e => {
                return Some(Err(ShError::sys(abi::Error::TimedOut, "tasks: taskd neodpovedel")));
            }
            (None, _) | (_, None) => return Some(Err(ShError::sys(abi::Error::TimedOut, "tasks: bez hodin"))),
            _ => {}
        }
        env.sleep(10_000_000);
    }
}

/// Spusti ulohu `async` ve vlakne (z radku v hlavnim vlakne i z radku ve
/// vlakne pres pozadavek na hlavni vlakno). `None` = vlakno nejde spustit
/// (uloha probehne na miste).
pub(crate) fn spawn_task(c: &mut crate::fg::MainCtx<'_>, wire: tasks::SpecWire) -> Option<Result<u64, ShError>> {
    let notify = c.env.notifier()?;
    let clock = c.env.clock()?;
    let id = *c.next_block;
    let fs = c.env.task_fs(id)?;
    *c.next_block += 1;
    let cancel = tasks::register(c.tasks, id, c.env.new_notifier());
    let label = wire.label.clone();
    let (table, n, session, home, budget) = (c.tasks.clone(), notify.clone(), c.session, String::from(c.home), c.task_stack);
    let job = Box::new(move || tasks::run(table, n, clock, cancel, fs, session, id, id, home, wire, budget));
    let Some(join) = c.env.spawn(job) else {
        tasks::remove(c.tasks, id);
        return None;
    };
    tasks::set_join(c.tasks, id, join);
    let mut line = String::from("async ");
    line.push_str(&label);
    c.board.register(&mut EnvLink(&mut *c.env), Local::Async(id), &line, Some((session, id)), true);
    let m = ServerMsg::BlockStart { session, block: id, line };
    if c.attached {
        c.link.send(&m);
    }
    c.log.push(m);
    Some(Ok(id))
}

impl Service {
    pub fn new(home: &str) -> Self {
        Self {
            sessions: Vec::new(),
            next_session: 1,
            next_block: 1,
            next_prompt: 1,
            home: home.into(),
            keep_blocks: 200,
            deferred: Vec::new(),
            stack_budget: 512 * 1024,
            task_stack: 160 * 1024,
            fixed: None,
            board: Board::default(),
            line_threads: false,
        }
    }

    /// Sluzba jedne relace s cislem `session` (proces relace). Cisla bloku
    /// a vyzev lezi v rozsahu relace ([`crate::broker::id_base`]), takze je
    /// prostrednik smeruje bez tabulky.
    pub fn single(home: &str, session: u64) -> Self {
        let mut s = Self::new(home);
        s.fixed = Some(session);
        s.next_block = crate::broker::id_base(session);
        s.next_prompt = crate::broker::id_base(session);
        s
    }

    /// Volna chvile: klony k zahozeni (po prevzeti radku) opravdu zahodit.
    fn flush(&mut self, fs: &mut dyn Fs, env: &mut dyn Env) {
        let task_stack = self.task_stack;
        let mut next_block = 0;
        for s in &mut self.sessions {
            if !s.interp.has_flush() {
                continue;
            }
            let mut block = Block { id: 0, line: String::new(), items: Vec::new(), ok: None, duration: 0 };
            let mut attached = false;
            let mut link = NullLink;
            let mut next_prompt = 0;
            let mut deferred = Vec::new();
            let mut host = SessionHost {
                fs: &mut *fs,
                link: &mut link,
                env: &mut *env,
                session: s.id,
                block: &mut block,
                attached: &mut attached,
                interrupt: false,
                next_prompt: &mut next_prompt,
                deferred: &mut deferred,
                home: &self.home,
                polls: 0,
                bars: Vec::new(),
                tasks: &s.tasks,
                next_block: &mut next_block,
                task_stack,
                log: Vec::new(),
                board: &mut self.board,
            };
            s.interp.flush(&mut host);
        }
    }

    /// Ceka nejaka uloha na pozadi (`bg` nebo `async`)?
    pub fn has_jobs(&self) -> bool {
        self.sessions.iter().any(|s| {
            s.interp.jobs.iter().any(|j| j.state == shlang::eval::JobState::Pending) || s.interp.tasks_running() > 0
        }) || self.lines_running()
    }

    /// Zpravy uloh do okna a bloku; dobehle ulohy prevezme interpret
    /// (vysledek pro pozdejsi `await`, undo).
    fn pump_tasks(&mut self, link: &mut dyn Link, fs: &mut dyn Fs, env: &mut dyn Env) {
        let keep = self.keep_blocks;
        let task_stack = self.task_stack;
        let Service { sessions, next_prompt, next_block, deferred, home, board, .. } = self;
        board.pump(&mut EnvLink(&mut *env));
        // Zmeneny seznam uloh pripojenym oknum (panel, stavovy radek).
        if core::mem::take(&mut board.changed) && sessions.iter().any(|s| s.attached) {
            link.send(&ServerMsg::Tasks { tasks: board.list.tasks.clone() });
        }
        for s in sessions.iter_mut() {
            apply_controls(board, &s.tasks);
            let mut log = Vec::new();
            pump(&s.tasks, next_prompt, s.attached, link, &mut log);
            end_tasks(board, &mut *env, &log);
            for m in log {
                apply(&mut s.blocks, keep, m);
            }
            for undo in tasks::take_late(&s.tasks) {
                s.interp.undo.adopt(&mut *fs, undo);
            }
            if s.interp.tasks_running() == 0 {
                continue;
            }
            let Session { id, interp, attached, tasks, blocks, .. } = s;
            let mut block = Block { id: 0, line: String::new(), items: Vec::new(), ok: None, duration: 0 };
            let mut host = SessionHost {
                fs: &mut *fs,
                link: &mut *link,
                env: &mut *env,
                session: *id,
                block: &mut block,
                attached,
                interrupt: false,
                next_prompt: &mut *next_prompt,
                deferred: &mut *deferred,
                home,
                polls: 0,
                bars: Vec::new(),
                tasks: &*tasks,
                next_block: &mut *next_block,
                task_stack,
                log: Vec::new(),
                board: &mut *board,
            };
            interp.reap_tasks(&mut host);
            let log = core::mem::take(&mut host.log);
            end_tasks(host.board, host.env, &log);
            for m in log {
                apply(blocks, keep, m);
            }
        }
    }

    fn session_index(&self, id: u64) -> Option<usize> {
        self.sessions.iter().position(|s| s.id == id)
    }

    fn new_session(&mut self) -> u64 {
        let id = self.fixed.unwrap_or(self.next_session);
        self.next_session += 1;
        let mut interp = Interp::new(Box::new(Builtins), &self.home);
        interp.stack_budget = self.stack_budget;
        self.sessions.push(Session {
            id,
            interp,
            blocks: Vec::new(),
            attached: false,
            tasks: tasks::shared(),
            fg: None,
            bg: Vec::new(),
            queue: Vec::new(),
            next_bg: 1,
        });
        id
    }

    /// Zpracuje zpravu okna (a zpravy, ktere mezitim prisly).
    pub fn handle(&mut self, msg: ClientMsg, link: &mut dyn Link, fs: &mut dyn Fs, env: &mut dyn Env) {
        self.pump_lines(link, env);
        self.handle_one(msg, link, fs, env);
        while !self.deferred.is_empty() {
            let m = self.deferred.remove(0);
            self.handle_one(m, link, fs, env);
        }
    }

    fn handle_one(&mut self, msg: ClientMsg, link: &mut dyn Link, fs: &mut dyn Fs, env: &mut dyn Env) {
        match msg {
            ClientMsg::Hello { .. } => link.send(&ServerMsg::Welcome { version: VERSION }),
            ClientMsg::Attach { session } | ClientMsg::AttachRecent { session } => {
                let recent = matches!(msg, ClientMsg::AttachRecent { .. });
                let first = self.fixed.is_some_and(|f| f == session) && self.sessions.is_empty();
                let id = if session == 0 || first { self.new_session() } else { session };
                let Some(i) = self.session_index(id) else {
                    link.send(&ServerMsg::Fail { message: alloc::format!("relace {id} neexistuje") });
                    return;
                };
                let s = &mut self.sessions[i];
                s.attached = true;
                // Prehrat bloky (i ty, ktere dobehly bez okna). `Attach`:
                // vse v poradi pred `Attached`. `AttachRecent` (okno GUI):
                // nejdriv posledni (viditelna cast), po `Attached` starsi po
                // davkach od nejnovejsich; okno je vlozi pred novejsi podle
                // cisla bloku (`ui::sessions`), takze se nezasekne ani
                // u dlouhe historie.
                let split = if recent { s.blocks.len().saturating_sub(REPLAY_VISIBLE) } else { 0 };
                s.blocks[split..].iter().for_each(|b| replay_block(link, id, b));
                // Seznam uloh (jen s prehledem uloh) pred `Attached`.
                if self.board.available() {
                    link.send(&ServerMsg::Tasks { tasks: self.board.list.tasks.clone() });
                }
                link.send(&ServerMsg::Attached { session: id, cwd: s.interp.cwd.clone(), blocks: s.blocks.len() as u32 });
                for chunk in s.blocks[..split].rchunks(REPLAY_BATCH) {
                    chunk.iter().for_each(|b| replay_block(link, id, b));
                }
            }
            ClientMsg::Detach { session } => {
                if let Some(i) = self.session_index(session) {
                    self.sessions[i].attached = false;
                }
            }
            ClientMsg::Run { session, line } => self.run(session, &line, link, fs, env),
            ClientMsg::Answer { .. } | ClientMsg::Choose { .. } | ClientMsg::Text { .. } if self.answer_line(&msg) => {}
            ClientMsg::Interrupt { session } => self.interrupt_line(session),
            ClientMsg::Background { session } => self.background(session, link, env),
            ClientMsg::Answer { prompt, yes } => {
                // Odpoved na vyzvu ulohy (radek uz dobehl, uloha bezi).
                for s in &self.sessions {
                    if tasks::answer(&s.tasks, prompt, yes) {
                        break;
                    }
                }
            }
            ClientMsg::TaskAction { task, action, .. } => {
                // Vysledek (napr. ACCESS_DENIED) okno uvidi jako zmenu stavu
                // ulohy, nebo ji nevidi; taskd o pravech rozhoduje sam.
                if let Some(a) = taskboard::Action::from_u8(action) {
                    self.board.action(&mut EnvLink(env), task, a);
                }
            }
            ClientMsg::Choose { .. } | ClientMsg::Text { .. } => {}
            ClientMsg::Complete { session, line, cursor, seq } => {
                let Some(i) = self.session_index(session) else {
                    link.send(&ServerMsg::Fail { message: alloc::format!("relace {session} neexistuje") });
                    return;
                };
                let s = &self.sessions[i];
                let mut ctx = shlang::complete::Ctx {
                    sigs: &*s.interp.sigs,
                    vars: s.interp.var_names(),
                    fns: s.interp.fn_names(),
                    fs,
                    cwd: s.interp.cwd.clone(),
                    home: s.interp.home.clone(),
                };
                let (start, c) = shlang::complete::complete(&line, cursor as usize, &mut ctx);
                let items = c.into_iter().map(|c| (c.text, c.kind as u8, c.help)).collect();
                link.send(&ServerMsg::Completions { seq, start: start as u32, items });
            }
            ClientMsg::Format { session, line, style, seq } => {
                let vars = self.session_index(session).map(|i| self.sessions[i].interp.var_names()).unwrap_or_default();
                let to = if style == 1 { Style::C } else { Style::B };
                let parsed = shlang::parse(&line, &shlang::Options { style: None, sigs: &Builtins, vars: vars.clone(), script: false });
                match parsed {
                    Ok(p) => {
                        let text = shlang::fmt::format_line(&p.script, &shlang::fmt::FmtOptions { style: to, sigs: &Builtins, vars });
                        link.send(&ServerMsg::Formatted { seq, text });
                    }
                    Err(e) => link.send(&ServerMsg::Fail { message: e.report(&line) }),
                }
            }
            ClientMsg::Highlight { session, line, seq } => {
                let Some(i) = self.session_index(session) else {
                    link.send(&ServerMsg::Fail { message: alloc::format!("relace {session} neexistuje") });
                    return;
                };
                let s = &self.sessions[i];
                let vars = s.interp.var_names();
                let commands = s.interp.fn_names();
                let ctx = shlang::highlight::Ctx { sigs: &*s.interp.sigs, vars: &vars, commands: &commands };
                let h = shlang::highlight::highlight(&line, Some(&ctx));
                link.send(&highlighted(seq, &h));
            }
            ClientMsg::Sessions => {
                let sessions =
                    self.sessions.iter().map(|s| (s.id, s.blocks.len() as u32, s.interp.cwd.clone())).collect();
                link.send(&ServerMsg::SessionList { sessions });
            }
        }
    }

    fn start_block(&mut self, i: usize, line: &str, link: &mut dyn Link) -> u64 {
        let id = self.next_block;
        self.next_block += 1;
        let s = &mut self.sessions[i];
        s.blocks.push(Block { id, line: line.into(), items: Vec::new(), ok: None, duration: 0 });
        if s.blocks.len() > self.keep_blocks {
            s.blocks.remove(0);
        }
        if s.attached {
            link.send(&ServerMsg::BlockStart { session: s.id, block: id, line: line.into() });
        }
        id
    }

    fn end_block(&mut self, i: usize, block: u64, ok: bool, duration: i64, link: &mut dyn Link) {
        let s = &mut self.sessions[i];
        let Some(b) = s.blocks.iter_mut().rev().find(|b| b.id == block) else { return };
        b.ok = Some(ok);
        b.duration = duration;
        if s.attached {
            link.send(&ServerMsg::BlockEnd { session: s.id, block: b.id, ok, duration });
            // `cd` v bloku: vyzva v okne hned s novym adresarem.
            link.send(&ServerMsg::Cwd { session: s.id, cwd: s.interp.cwd.clone() });
        }
    }

    /// Spusti radek v relaci jako novy blok.
    pub fn run(&mut self, session: u64, line: &str, link: &mut dyn Link, fs: &mut dyn Fs, env: &mut dyn Env) {
        let Some(i) = self.session_index(session) else {
            link.send(&ServerMsg::Fail { message: alloc::format!("relace {session} neexistuje") });
            return;
        };
        // Ve vlakne (Ctrl+Z, `&`, fronta radku), kdyz to prostredi umi.
        if self.run_threaded(i, line, link, env) {
            return;
        }
        self.start_block(i, line, link);
        let start = env.now();
        let (keep, task_stack) = (self.keep_blocks, self.task_stack);
        let Service { sessions, next_prompt, next_block, deferred, home, board, .. } = self;
        let s = &mut sessions[i];
        let Session { id, interp, blocks, attached, tasks, .. } = s;
        let mut line_block = blocks.pop().unwrap();
        let line_id = line_block.id;
        let mut host = SessionHost {
            fs,
            link: &mut *link,
            env,
            session: *id,
            block: &mut line_block,
            attached,
            interrupt: false,
            next_prompt,
            deferred,
            home,
            polls: 0,
            bars: Vec::new(),
            tasks: &*tasks,
            next_block,
            task_stack,
            log: Vec::new(),
            board: &mut *board,
        };
        let out = interp.run_line(&mut host, line);
        for w in out.warnings {
            host.push(Item::Warning(w.message));
        }
        if !out.diff.is_empty() {
            host.push(Item::Diff(shlang::vfs::diff::diff_value(&out.diff)));
        }
        let ok = out.result.is_ok();
        if let Err(e) = out.result {
            host.push(Item::Error { code: e.code.clone(), message: e.message.clone(), report: e.report(line) });
        }
        let duration = match (start, host.env.now()) {
            (Some(a), Some(b)) => b - a,
            _ => 0,
        };
        // Blok radku zustava pred bloky uloh, ktere spustil.
        let log = core::mem::take(&mut host.log);
        end_tasks(host.board, host.env, &log);
        blocks.push(line_block);
        for m in log {
            apply(blocks, keep, m);
        }
        self.end_block(i, line_id, ok, duration, link);
    }

    /// Shell je volny: dobehnou ulohy na pozadi (kazda jako blok).
    pub fn idle(&mut self, link: &mut dyn Link, fs: &mut dyn Fs, env: &mut dyn Env) {
        self.pump_lines(link, env);
        self.pump_tasks(link, fs, env);
        self.flush(fs, env);
        for i in 0..self.sessions.len() {
            let pending = self.sessions[i]
                .interp
                .jobs
                .iter()
                .filter(|j| j.state == shlang::eval::JobState::Pending)
                .map(|j| (j.id, j.line.clone()))
                .collect::<Vec<_>>();
            if pending.is_empty() {
                continue;
            }
            // Vsechny cekajici ulohy relace dobehnou v jednom bloku.
            let label: Vec<String> = pending.iter().map(|(jid, line)| alloc::format!("[{jid}] {line}")).collect();
            {
                let bid = self.start_block(i, &label.join(", "), link);
                let sid = self.sessions[i].id;
                for (jid, line) in &pending {
                    let name = alloc::format!("bg [{jid}] {line}");
                    self.board.register(&mut EnvLink(&mut *env), Local::Bg(*jid), &name, Some((sid, bid)), false);
                }
                let (keep, task_stack) = (self.keep_blocks, self.task_stack);
                let Service { sessions, next_prompt, next_block, deferred, home, board, .. } = self;
                let Session { id, interp, blocks, attached, tasks, .. } = &mut sessions[i];
                let mut job_block = blocks.pop().unwrap();
                let mut host = SessionHost {
                    fs: &mut *fs,
                    link: &mut *link,
                    env: &mut *env,
                    session: *id,
                    block: &mut job_block,
                    attached,
                    interrupt: false,
                    next_prompt,
                    deferred,
                    home,
                    polls: 0,
                    bars: Vec::new(),
                    tasks: &*tasks,
                    next_block,
                    task_stack,
                    log: Vec::new(),
                    board: &mut *board,
                };
                let results = interp.run_pending_jobs(&mut host);
                let mut ok = true;
                for (jid, r) in &results {
                    let st = match r {
                        Ok(_) => taskproto::State::Done(String::new()),
                        Err(e) if e.code == code::INTERRUPTED => taskproto::State::Cancelled,
                        Err(e) => taskproto::State::Failed { code: 0, message: e.message.clone() },
                    };
                    host.board.finish(&mut EnvLink(&mut *host.env), Local::Bg(*jid), st);
                }
                for (_, r) in results {
                    match r {
                        Ok(v) => {
                            if !matches!(v, Value::Nothing) {
                                host.push(Item::Value(v));
                            }
                        }
                        Err(e) => {
                            ok = false;
                            host.push(Item::Error { code: e.code.clone(), message: e.message.clone(), report: e.report("") });
                        }
                    }
                }
                let log = core::mem::take(&mut host.log);
                blocks.push(job_block);
                for m in log {
                    apply(blocks, keep, m);
                }
                self.end_block(i, bid, ok, 0, link);
            }
        }
        // Klony, ktere ulohy prave odlozily, zahodit hned (ne az pri dalsi
        // volne chvili): mezitim muze relaci prevzit novy termd a hned
        // poslat prikazy - dalsi volna chvile by pak neprisla a klon by
        // zustal ve slozce klonu (`/tmp/<uid>/.aside`).
        self.flush(fs, env);
    }
}
