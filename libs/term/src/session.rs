//! Sluzba relaci: relace (interpret, pracovni slozka, historie, undo,
//! ulohy) a bloky vystupu patri sluzbe, ne oknu terminalu.
//!
//! Okno se k relaci pripoji (`Attach`), posila radky (`Run`) a dostava
//! bloky vystupu. Po odpojeni (`Detach`, zavreni nebo pad okna) relace bezi
//! dal: rozbehnuty prikaz dobehne, vystup se ulozi do bloku a po novem
//! pripojeni se prehraje. Dotaz (nahled zmen, zamek) bez pripojeneho okna
//! se bezpecne odmitne.

use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;

use shlang::eval::Interp;
use shlang::host::{ExecNeeds, Fs, Host};
use shlang::sig::Builtins;
use shlang::value::Value;
use shlang::{ShError, Style};

use crate::proto::{ClientMsg, Item, ServerMsg, VERSION};

/// Spojeni s oknem terminalu (kanal v systemu, fronta v testech).
pub trait Link {
    fn send(&mut self, msg: &ServerMsg);
    /// Zprava, pokud nejaka ceka (bez cekani).
    fn poll(&mut self) -> Option<ClientMsg>;
    /// Pocka na zpravu; `None` = okno je pryc.
    fn wait(&mut self) -> Option<ClientMsg>;
}

/// Zbytek prostredi (cas, spousteni programu).
pub trait Env {
    fn now(&mut self) -> Option<i64>;
    fn sleep(&mut self, ns: i64);
    fn exec(&mut self, _name: &str, _args: &[(Option<String>, Value)], _input: Value, _needs: &ExecNeeds)
        -> Option<Result<Value, ShError>> {
        None
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
}

impl SessionHost<'_> {
    fn push(&mut self, item: Item) {
        if *self.attached {
            self.link.send(&ServerMsg::Item { session: self.session, block: self.block.id, item: item.clone() });
        }
        self.block.items.push(item);
    }

    /// Zpracuje zpravu, ktera prisla behem prikazu.
    fn incoming(&mut self, m: ClientMsg) {
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
        // Spi po kouscich, aby slo prerusit.
        let step = 50_000_000;
        let mut left = ns;
        while left > 0 && !self.interrupted() {
            self.env.sleep(left.min(step));
            left -= step;
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

    fn confirm(&mut self, question: &str, detail: &Value) -> bool {
        if !*self.attached {
            self.push(Item::Note(alloc::format!("{question} - okno neni pripojene, odmitnuto")));
            return false;
        }
        let id = *self.next_prompt;
        *self.next_prompt += 1;
        self.link.send(&ServerMsg::Prompt { prompt: id, question: question.into(), detail: Item::Value(detail.clone()) });
        loop {
            match self.link.wait() {
                Some(ClientMsg::Answer { prompt, yes }) if prompt == id => {
                    self.block.items.push(Item::Note(alloc::format!(
                        "{question} -> {}",
                        if yes { "ano" } else { "ne" }
                    )));
                    return yes;
                }
                Some(m @ (ClientMsg::Interrupt { .. } | ClientMsg::Detach { .. })) => {
                    self.incoming(m);
                    return false;
                }
                Some(other) => self.deferred.push(other),
                None => {
                    *self.attached = false;
                    return false;
                }
            }
        }
    }

    fn exec(&mut self, name: &str, args: &[(Option<String>, Value)], input: Value, needs: &ExecNeeds)
        -> Option<Result<Value, ShError>> {
        self.env.exec(name, args, input, needs)
    }

    fn home(&self) -> String {
        self.home.into()
    }
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
        }
    }

    fn session_index(&self, id: u64) -> Option<usize> {
        self.sessions.iter().position(|s| s.id == id)
    }

    fn new_session(&mut self) -> u64 {
        let id = self.next_session;
        self.next_session += 1;
        let mut interp = Interp::new(Box::new(Builtins), &self.home);
        interp.stack_budget = self.stack_budget;
        self.sessions.push(Session { id, interp, blocks: Vec::new(), attached: false });
        id
    }

    /// Zpracuje zpravu okna (a zpravy, ktere mezitim prisly).
    pub fn handle(&mut self, msg: ClientMsg, link: &mut dyn Link, fs: &mut dyn Fs, env: &mut dyn Env) {
        self.handle_one(msg, link, fs, env);
        while !self.deferred.is_empty() {
            let m = self.deferred.remove(0);
            self.handle_one(m, link, fs, env);
        }
    }

    fn handle_one(&mut self, msg: ClientMsg, link: &mut dyn Link, fs: &mut dyn Fs, env: &mut dyn Env) {
        match msg {
            ClientMsg::Hello { .. } => link.send(&ServerMsg::Welcome { version: VERSION }),
            ClientMsg::Attach { session } => {
                let id = if session == 0 { self.new_session() } else { session };
                let Some(i) = self.session_index(id) else {
                    link.send(&ServerMsg::Fail { message: alloc::format!("relace {id} neexistuje") });
                    return;
                };
                let s = &mut self.sessions[i];
                s.attached = true;
                // Prehrat bloky (i ty, ktere dobehly bez okna).
                for b in &s.blocks {
                    link.send(&ServerMsg::BlockStart { session: id, block: b.id, line: b.line.clone() });
                    for it in &b.items {
                        link.send(&ServerMsg::Item { session: id, block: b.id, item: it.clone() });
                    }
                    if let Some(ok) = b.ok {
                        link.send(&ServerMsg::BlockEnd { session: id, block: b.id, ok, duration: b.duration });
                    }
                }
                link.send(&ServerMsg::Attached { session: id, cwd: s.interp.cwd.clone(), blocks: s.blocks.len() as u32 });
            }
            ClientMsg::Detach { session } => {
                if let Some(i) = self.session_index(session) {
                    self.sessions[i].attached = false;
                }
            }
            ClientMsg::Run { session, line } => self.run(session, &line, link, fs, env),
            ClientMsg::Interrupt { .. } | ClientMsg::Answer { .. } => {}
            ClientMsg::Complete { session, line, cursor } => {
                let Some(i) = self.session_index(session) else { return };
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
                link.send(&ServerMsg::Completions { start: start as u32, items });
            }
            ClientMsg::Format { session, line, style } => {
                let vars = self.session_index(session).map(|i| self.sessions[i].interp.var_names()).unwrap_or_default();
                let to = if style == 1 { Style::C } else { Style::B };
                let parsed = shlang::parse(&line, &shlang::Options { style: None, sigs: &Builtins, vars: vars.clone(), script: false });
                match parsed {
                    Ok(p) => {
                        let text = shlang::fmt::format_line(&p.script, &shlang::fmt::FmtOptions { style: to, sigs: &Builtins, vars });
                        link.send(&ServerMsg::Formatted { text });
                    }
                    Err(e) => link.send(&ServerMsg::Fail { message: e.report(&line) }),
                }
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

    fn end_block(&mut self, i: usize, ok: bool, duration: i64, link: &mut dyn Link) {
        let s = &mut self.sessions[i];
        let b = s.blocks.last_mut().unwrap();
        b.ok = Some(ok);
        b.duration = duration;
        if s.attached {
            link.send(&ServerMsg::BlockEnd { session: s.id, block: b.id, ok, duration });
        }
    }

    /// Spusti radek v relaci jako novy blok.
    pub fn run(&mut self, session: u64, line: &str, link: &mut dyn Link, fs: &mut dyn Fs, env: &mut dyn Env) {
        let Some(i) = self.session_index(session) else {
            link.send(&ServerMsg::Fail { message: alloc::format!("relace {session} neexistuje") });
            return;
        };
        self.start_block(i, line, link);
        let start = env.now();
        let Service { sessions, next_prompt, deferred, home, .. } = self;
        let s = &mut sessions[i];
        let Session { id, interp, blocks, attached } = s;
        let block = blocks.last_mut().unwrap();
        let mut host = SessionHost {
            fs,
            link: &mut *link,
            env,
            session: *id,
            block,
            attached,
            interrupt: false,
            next_prompt,
            deferred,
            home,
            polls: 0,
        };
        let out = interp.run_line(&mut host, line);
        for w in out.warnings {
            host.push(Item::Warning(w.message));
        }
        if !out.diff.is_empty() {
            host.push(Item::Diff(shlang::vfs::overlay::diff_value(&out.diff)));
        }
        let ok = out.result.is_ok();
        if let Err(e) = out.result {
            host.push(Item::Error { code: e.code.clone(), message: e.message.clone(), report: e.report(line) });
        }
        let duration = match (start, host.env.now()) {
            (Some(a), Some(b)) => b - a,
            _ => 0,
        };

        self.end_block(i, ok, duration, link);
    }

    /// Shell je volny: dobehnou ulohy na pozadi (kazda jako blok).
    pub fn idle(&mut self, link: &mut dyn Link, fs: &mut dyn Fs, env: &mut dyn Env) {
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
                self.start_block(i, &label.join(", "), link);
                let Service { sessions, next_prompt, deferred, home, .. } = self;
                let Session { id, interp, blocks, attached } = &mut sessions[i];
                let block = blocks.last_mut().unwrap();
                let mut host = SessionHost {
                    fs: &mut *fs,
                    link: &mut *link,
                    env: &mut *env,
                    session: *id,
                    block,
                    attached,
                    interrupt: false,
                    next_prompt,
                    deferred,
                    home,
                    polls: 0,
                };
                let results = interp.run_pending_jobs(&mut host);
                let mut ok = true;
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
                self.end_block(i, ok, 0, link);
            }
        }
    }
}
