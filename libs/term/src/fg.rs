//! Radek relace ve vlakne: hlavni vlakno sluzby relaci mezitim obsluhuje
//! okno (Ctrl+C, Ctrl+Z, vyzvy, dalsi radky do fronty).
//!
//! Radek bezi nad interpretem relace (`cd`, `let`, `fn` a undo plati jako
//! drive) a nad vlastni relaci FS (stejna identita; koncept a pohledy undo
//! jsou objekty FS, ne spojeni). Hlavni vlakno:
//!
//! - preposila zpravy radku oknu a uklada je do bloku (`outbox`),
//! - posila vyzvy radku oknu a vraci odpovedi (`prompt`),
//! - provadi pozadavky na sluzby prostredi (zvuk, agent, prehled uloh,
//!   napajeni, log): radek je ve vlakne nevola sam (`call`),
//! - Ctrl+C = priznak `cancel`; Ctrl+Z = `detached`: radek bezi dal jako
//!   uloha na pozadi a relace pokracuje kopii stavu ze zacatku radku,
//! - po konci vrati interpret relaci (radek v poprede) nebo ho zahodi
//!   (uloha na pozadi - jako subshell).

use alloc::boxed::Box;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::any::Any;
use core::sync::atomic::{AtomicBool, Ordering};

use shlang::eval::Interp;
use shlang::host::{AudioReq, ExecNeeds, Fs, Host, Opened, TaskDone, TaskSpec, TasksOp};
use shlang::value::Value;
use shlang::{ShError, error::code};
use spin::Mutex;

use crate::proto::{ClientMsg, Item, ServerMsg};
use crate::session::{Env, InstallerMsg, Link};
use crate::taskboard::Board;
use crate::tasks::{self, Clock, Notify};

/// Pozadavek na hlavni vlakno (sluzby prostredi). Vysledek jako `Any`.
pub type MainCall = Box<dyn FnOnce(&mut MainCtx<'_>) -> Box<dyn Any + Send> + Send>;

/// Co hlavni vlakno da pozadavku radku.
pub struct MainCtx<'a> {
    pub env: &'a mut dyn Env,
    pub board: &'a mut Board,
    pub link: &'a mut dyn Link,
    pub attached: bool,
    pub tasks: &'a tasks::Shared,
    pub next_block: &'a mut u64,
    pub log: &'a mut Vec<ServerMsg>,
    pub session: u64,
    pub home: &'a str,
    pub task_stack: usize,
}

/// Odpoved okna na vyzvu radku.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Answer {
    Yes(bool),
    Choice(u32),
    /// Text (dotaz `Host::ask`, napr. heslo v pruvodci `install`).
    Text(String),
    /// Okno neni pripojene nebo radek je na pozadi bez okna.
    Refused,
}

pub struct LinePrompt {
    pub question: String,
    pub detail: Item,
    pub choices: Vec<String>,
    /// Dotaz na text (`ServerMsg::Ask`): skryty vstup a vychozi text.
    /// `None` = vyzva ano/ne nebo vyber (`ServerMsg::Prompt`).
    pub text: Option<(bool, String)>,
    /// Cislo vyzvy v protokolu (az ji hlavni vlakno posle oknu).
    pub sent: Option<u64>,
    pub answer: Option<Answer>,
}

/// Konec radku: interpret (vrati se relaci, nebo se zahodi) a vysledek.
pub struct LineDone {
    pub interp: Box<Interp>,
    pub ok: bool,
    pub interrupted: bool,
    pub duration: i64,
}

#[derive(Default)]
pub struct LineState {
    pub outbox: Vec<ServerMsg>,
    pub prompt: Option<LinePrompt>,
    pub call: Option<MainCall>,
    reply: Option<Box<dyn Any + Send>>,
    pub done: Option<LineDone>,
}

/// Sdileny stav radku (hlavni vlakno a vlakno radku).
pub struct LineShared {
    pub cancel: AtomicBool,
    pub detached: AtomicBool,
    /// Probouzeni vlakna radku (odpoved, zruseni).
    wake: Option<Arc<dyn Notify>>,
    /// Probouzeni hlavniho vlakna (zprava, pozadavek, konec).
    main: Arc<dyn Notify>,
    pub st: Mutex<LineState>,
}

impl LineShared {
    pub fn new(main: Arc<dyn Notify>, wake: Option<Arc<dyn Notify>>) -> Arc<Self> {
        Arc::new(Self { cancel: AtomicBool::new(false), detached: AtomicBool::new(false), wake, main, st: Mutex::new(LineState::default()) })
    }

    /// Ctrl+C (nebo `kill`, konec relace).
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Release);
        self.wake();
    }

    pub fn wake(&self) {
        if let Some(w) = &self.wake {
            w.notify();
        }
    }

    /// Hlavni vlakno: vysledek pozadavku `call`.
    pub fn reply(&self, r: Box<dyn Any + Send>) {
        self.st.lock().reply = Some(r);
        self.wake();
    }

    /// Hlavni vlakno: odpoved okna na vyzvu `prompt`; `false` = neni nase.
    pub fn answer(&self, prompt: u64, a: Answer) -> bool {
        let mut st = self.st.lock();
        match st.prompt.as_mut() {
            Some(p) if p.sent == Some(prompt) => {
                p.answer = Some(a);
                drop(st);
                self.wake();
                true
            }
            _ => false,
        }
    }
}

/// Hostitel interpretu ve vlakne radku.
struct LineHost {
    fs: Box<dyn Fs + Send>,
    sh: Arc<LineShared>,
    clock: Clock,
    session: u64,
    block: u64,
    home: String,
    tasks: tasks::Shared,
    /// Polozky bloku (pro aktualizace pruhu) a pruhy (id, index).
    items: usize,
    bars: Vec<(u32, usize)>,
}

impl LineHost {
    fn send(&mut self, m: ServerMsg) {
        self.sh.st.lock().outbox.push(m);
        self.sh.main.notify();
    }

    fn push(&mut self, item: Item) {
        let item = match item {
            Item::Value(v) => Item::Value(crate::binary::for_window(v)),
            other => other,
        };
        self.items += 1;
        self.send(ServerMsg::Item { session: self.session, block: self.block, item });
    }

    fn bar(&mut self, id: u32, done: u64, total: u64, label: &str) {
        let item = Item::Progress { done, total, label: label.into() };
        match self.bars.iter().find(|(b, _)| *b == id).map(|(_, i)| *i) {
            Some(index) => self.send(ServerMsg::ItemUpdate { session: self.session, block: self.block, index: index as u32, item }),
            None => {
                self.bars.push((id, self.items));
                self.push(item);
            }
        }
    }

    /// Pocka nejvys `ns` (drive pri probuzeni).
    fn nap(&self, ns: i64) {
        match &self.sh.wake {
            Some(w) => w.wait(ns),
            None => (self.clock.sleep)(ns.min(20_000_000)),
        }
    }

    /// Pozadavek na hlavni vlakno; `None` = zadna odpoved (relace konci).
    fn call<T: Send + 'static>(&mut self, f: impl FnOnce(&mut MainCtx<'_>) -> T + Send + 'static) -> Option<T> {
        {
            let mut st = self.sh.st.lock();
            st.reply = None;
            st.call = Some(Box::new(move |c: &mut MainCtx<'_>| Box::new(f(c)) as Box<dyn Any + Send>));
        }
        self.sh.main.notify();
        let mut waited = 0i64;
        loop {
            if let Some(r) = self.sh.st.lock().reply.take() {
                return r.downcast::<T>().ok().map(|b| *b);
            }
            // Relace skoncila (zruseni bez odpovedi): nevisi navzdy.
            if self.sh.cancel.load(Ordering::Acquire) {
                waited += 1;
                if waited > 20 {
                    return None;
                }
            }
            self.nap(100_000_000);
        }
    }

    /// Vyzva v okne a cekani na odpoved (nebo zruseni).
    fn ask(&mut self, question: &str, detail: Item, choices: &[String]) -> Answer {
        self.wait_answer(LinePrompt { question: question.into(), detail, choices: choices.to_vec(), text: None, sent: None, answer: None })
    }

    fn wait_answer(&mut self, prompt: LinePrompt) -> Answer {
        self.sh.st.lock().prompt = Some(prompt);
        self.sh.main.notify();
        loop {
            if self.sh.cancel.load(Ordering::Acquire) {
                self.sh.st.lock().prompt = None;
                return Answer::Refused;
            }
            let a = {
                let mut st = self.sh.st.lock();
                match st.prompt.as_mut().and_then(|p| p.answer.take()) {
                    Some(a) => {
                        st.prompt = None;
                        Some(a)
                    }
                    None => None,
                }
            };
            if let Some(a) = a {
                return a;
            }
            self.nap(500_000_000);
        }
    }
}

impl Host for LineHost {
    fn fs(&mut self) -> &mut dyn Fs {
        &mut *self.fs
    }

    fn emit(&mut self, value: Value) {
        self.push(Item::Value(value));
    }

    fn note(&mut self, text: &str) {
        self.push(Item::Note(text.into()));
    }

    fn now(&mut self) -> Option<i64> {
        (self.clock.now)()
    }

    fn sleep(&mut self, ns: i64) {
        // Preruseni hned (udalost), delsi cekani s pruhem prubehu.
        let Some(start) = (self.clock.now)() else {
            (self.clock.sleep)(ns);
            return;
        };
        let long = ns >= 1_000_000_000;
        let ms = |v: i64| (v.max(0) / 1_000_000) as u64;
        let end = start.saturating_add(ns);
        while !self.interrupted() {
            let Some(now) = (self.clock.now)() else { return };
            if now >= end {
                break;
            }
            if long {
                self.bar(0, ms(now - start), ms(ns), "sleep");
            }
            self.nap((end - now).min(250_000_000));
        }
        if long && !self.interrupted() {
            self.bar(0, ms(ns), ms(ns), "sleep");
        }
    }

    fn progress(&mut self, id: u32, done: u64, total: u64, label: &str) {
        self.bar(id, done, total, label);
    }

    fn interrupted(&mut self) -> bool {
        self.sh.cancel.load(Ordering::Acquire)
    }

    fn confirm(&mut self, question: &str, detail: &Value) -> bool {
        let yes = matches!(self.ask(question, Item::Value(detail.clone()), &[]), Answer::Yes(true) | Answer::Choice(0));
        self.push(Item::Note(alloc::format!("{question} -> {}", if yes { "ano" } else { "ne" })));
        yes
    }

    fn choose(&mut self, question: &str, choices: &[String]) -> Option<usize> {
        if choices.is_empty() {
            return None;
        }
        match self.ask(question, Item::Note(String::new()), choices) {
            Answer::Choice(i) if (i as usize) < choices.len() => {
                self.push(Item::Note(alloc::format!("{question} -> {}", choices[i as usize])));
                Some(i as usize)
            }
            _ => None,
        }
    }

    fn ask(&mut self, question: &str, secret: bool, default: &str) -> Option<String> {
        let p = LinePrompt {
            question: question.into(),
            detail: Item::Note(String::new()),
            choices: Vec::new(),
            text: Some((secret, default.into())),
            sent: None,
            answer: None,
        };
        match self.wait_answer(p) {
            Answer::Text(text) => {
                // Heslo do vypisu bloku nepatri.
                let shown = if secret { String::from("***") } else { text.clone() };
                self.push(Item::Note(alloc::format!("{question} -> {shown}")));
                Some(text)
            }
            _ => None,
        }
    }

    fn installer(&mut self, req: &[u8], progress: &mut dyn FnMut(&[u8])) -> Result<Vec<u8>, abi::Error> {
        // Kratka volani hlavniho vlakna: mezi nimi obsluhuje okno (vyzvy,
        // Ctrl+C jinych radku) a prubeh chodi hned. Instalace se Ctrl+C
        // neprerusi (zapis na disk bezi v installd), cekani pokracuje.
        let req = req.to_vec();
        self.call(move |c| c.env.installer_send(&req)).unwrap_or(Err(abi::Error::Canceled))?;
        loop {
            match self.call(|c| c.env.installer_recv(200)).unwrap_or(Err(abi::Error::Canceled))? {
                Some(InstallerMsg::Progress(m)) => progress(&m),
                Some(InstallerMsg::Done(m)) => return Ok(m),
                None => {}
            }
        }
    }

    fn open_path(&mut self, path: &str, app_only: bool) -> Result<Opened, abi::Error> {
        // Kratka volani hlavniho vlakna jako u instalatoru (aplikace muze
        // startovat az 20 s); Ctrl+C cekani prerusi.
        let p = String::from(path);
        self.call(move |c| c.env.open_send(&p, app_only)).unwrap_or(Err(abi::Error::Canceled))?;
        loop {
            if let Some(r) = self.call(|c| c.env.open_recv(200)).unwrap_or(Err(abi::Error::Canceled))? {
                return Ok(r);
            }
            if self.interrupted() {
                return Err(abi::Error::Canceled);
            }
        }
    }

    fn exec(&mut self, name: &str, args: &[(Option<String>, Value)], input: Value, needs: &ExecNeeds) -> Option<Result<Value, ShError>> {
        let (name, args, needs) = (String::from(name), args.to_vec(), needs.clone());
        self.call(move |c| c.env.exec(&name, &args, input, &needs)).flatten()
    }

    fn home(&self) -> String {
        self.home.clone()
    }

    fn audio(&mut self, req: AudioReq) -> Option<Result<Value, ShError>> {
        self.call(move |c| c.env.audio(req)).flatten()
    }

    fn power(&mut self, reboot: bool) -> Result<(), abi::Error> {
        self.call(move |c| c.env.power(reboot)).unwrap_or(Err(abi::Error::Canceled))
    }

    fn report(&mut self) -> Result<String, abi::Error> {
        self.call(|c| c.env.report()).unwrap_or(Err(abi::Error::Canceled))
    }

    fn log_level(&mut self, target: &str, level: &str) -> Result<String, abi::Error> {
        let (t, l) = (String::from(target), String::from(level));
        self.call(move |c| c.env.log_level(&t, &l)).unwrap_or(Err(abi::Error::Canceled))
    }

    fn appearance(&mut self, system: bool, change: &str) -> Result<String, abi::Error> {
        let c = String::from(change);
        self.call(move |s| s.env.appearance(system, &c)).unwrap_or(Err(abi::Error::Canceled))
    }

    fn permissions(&mut self, app: &str, perm: &str) -> Result<String, abi::Error> {
        let (a, p) = (String::from(app), String::from(perm));
        self.call(move |c| c.env.permissions(&a, &p)).unwrap_or(Err(abi::Error::Canceled))
    }

    fn accounts(&mut self) -> Result<Vec<shlang::acl::Account>, abi::Error> {
        self.call(|c| c.env.accounts()).unwrap_or(Err(abi::Error::Canceled))
    }

    fn task_spawn(&mut self, spec: TaskSpec) -> Option<Result<u64, ShError>> {
        let wire = match tasks::SpecWire::of(&spec) {
            Ok(w) => w,
            Err(e) => return Some(Err(e)),
        };
        self.call(move |c| crate::session::spawn_task(c, wire)).flatten()
    }

    fn task_wait(&mut self, ids: &[u64], any: bool, deadline: Option<i64>) -> Result<Vec<TaskDone>, ShError> {
        // Zpravy uloh oknu preposila hlavni vlakno (volna chvile).
        loop {
            if let Some(bad) = ids.iter().find(|i| !tasks::exists(&self.tasks, **i)) {
                return Err(ShError::new(code::ARGS, alloc::format!("úloha {bad} neexistuje")));
            }
            let done: Vec<u64> = ids.iter().copied().filter(|i| tasks::is_done(&self.tasks, *i)).collect();
            let failed = !any && done.iter().any(|i| tasks::failed(&self.tasks, *i));
            if (any && !done.is_empty()) || done.len() == ids.len() || failed {
                let out = done.into_iter().filter_map(|i| tasks::take(&self.tasks, i)).collect();
                self.sh.main.notify();
                return Ok(out);
            }
            if self.interrupted() {
                for i in ids {
                    tasks::cancel(&self.tasks, *i);
                }
                return Err(ShError::new(code::INTERRUPTED, "await přerušen (Ctrl-C), úlohy zrušeny"));
            }
            let mut step = 100_000_000;
            if let Some(d) = deadline {
                match (self.clock.now)() {
                    Some(n) if n < d => step = step.min(d - n),
                    _ => return Err(ShError::sys(abi::Error::TimedOut, "await: čas vypršel (úlohy běží dál)")),
                }
            }
            self.nap(step);
        }
    }

    fn tasks(&mut self, op: TasksOp) -> Option<Result<Value, ShError>> {
        self.call(move |c| crate::session::tasks_op(c.board, c.env, op)).flatten()
    }

    fn task_cancel(&mut self, id: u64) -> Option<TaskDone> {
        let d = tasks::cancel(&self.tasks, id);
        self.sh.main.notify();
        d
    }
}

/// Zadani radku pro vlakno.
pub struct LineJob {
    pub sh: Arc<LineShared>,
    pub interp: Box<Interp>,
    pub fs: Box<dyn Fs + Send>,
    pub clock: Clock,
    pub session: u64,
    pub block: u64,
    pub home: String,
    pub line: String,
    pub tasks: tasks::Shared,
}

/// Telo vlakna radku: provede radek, zpravy do schranky, na konci vrati
/// interpret (`done`).
pub fn run(job: LineJob) {
    let LineJob { sh, mut interp, fs, clock, session, block, home, line, tasks } = job;
    let start = (clock.now)();
    let mut host = LineHost { fs, sh: sh.clone(), clock, session, block, home, tasks, items: 0, bars: Vec::new() };
    let out = interp.run_line(&mut host, &line);
    for w in out.warnings {
        host.push(Item::Warning(w.message));
    }
    if !out.diff.is_empty() {
        host.push(Item::Diff(shlang::vfs::diff::diff_value(&out.diff)));
    }
    let ok = out.result.is_ok();
    let interrupted = matches!(&out.result, Err(e) if e.code == code::INTERRUPTED);
    if let Err(e) = out.result {
        host.push(Item::Error { code: e.code.clone(), message: e.message.clone(), report: e.report(&line) });
    }
    // Klony k zahozeni: radek na pozadi uz zadnou volnou chvili relace nema.
    if sh.detached.load(Ordering::Acquire) {
        interp.flush(&mut host);
    }
    let duration = match (start, (clock.now)()) {
        (Some(a), Some(b)) => b - a,
        _ => 0,
    };
    sh.st.lock().done = Some(LineDone { interp, ok, interrupted, duration });
    sh.main.notify();
}

/// Zprava okna patri radku (odpoved na jeho vyzvu)?
pub fn answer_of(m: &ClientMsg) -> Option<(u64, Answer)> {
    match m {
        ClientMsg::Answer { prompt, yes } => Some((*prompt, Answer::Yes(*yes))),
        ClientMsg::Choose { prompt, index } => Some((*prompt, Answer::Choice(*index))),
        ClientMsg::Text { prompt, text } => Some((*prompt, Answer::Text(text.clone()))),
        _ => None,
    }
}
