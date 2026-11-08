//! Relace a prehled uloh (`taskd`): relace je producent (hlasi ulohy
//! `async` a `bg` s blokem v terminalu) i klient (prikaz `tasks`, panel
//! uloh v okne).
//!
//! Kanaly razi init (pres termd prijdou relaci zpravou `SessMsg::Tasks`);
//! posila a prijima pres ne [`crate::session::Env`]. Bez nich (testy bez
//! taskd, taskd nebezi) relace funguje dal, jen ulohy nikam nehlasi.

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;

use taskproto::{Control, Origin, Produce, Reply, Request, State, Task, TaskList, actions};

/// Kanal prehledu uloh.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Chan {
    Producer,
    Client,
}

/// Uloha relace (mistni cislo).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Local {
    /// Uloha `async` (cislo bloku = cislo ulohy).
    Async(u64),
    /// Uloha `bg` (cislo ulohy interpretu).
    Bg(u64),
}

/// Stav prehledu uloh relace.
#[derive(Default)]
pub struct Board {
    /// Ulohy, ktere relace vidi (vlastni ulohy uzivatele).
    pub list: TaskList,
    subscribed: bool,
    next_req: u32,
    pending: BTreeMap<u32, Local>,
    ids: BTreeMap<Local, u64>,
    /// Konec ulohy driv, nez prislo REGISTERED.
    early: BTreeMap<Local, State>,
    results: BTreeMap<u32, taskproto::Result<()>>,
    logs: BTreeMap<u32, Vec<String>>,
    /// Pozadavky taskd na ulohy relace (zrusit ...), vyzvedne relace.
    pub controls: Vec<(Local, Control)>,
    /// Seznam se zmenil (poslat oknu).
    pub changed: bool,
}

/// Odeslani a prijem pres kanaly relace (implementuje `Env`).
pub trait Link {
    /// `false` = kanal neni (taskd nebezi).
    fn send(&mut self, to: Chan, data: Vec<u8>) -> bool;
    fn recv(&mut self) -> Vec<(Chan, Vec<u8>)>;
}

impl Board {
    fn req(&mut self) -> u32 {
        self.next_req = self.next_req.wrapping_add(1).max(1);
        self.next_req
    }

    /// Hlasit ulohu relace.
    pub fn register(&mut self, link: &mut dyn Link, local: Local, name: &str, attach: Option<(u64, u64)>, cancel: bool) {
        let req = self.req();
        let m = Produce::Register {
            req,
            parent: 0,
            name: name.into(),
            origin: Origin::User,
            who: String::from("shell"),
            agent: false,
            job: None,
            attach,
            actions: actions::ATTACH | if cancel { actions::CANCEL } else { 0 },
        };
        if link.send(Chan::Producer, m.encode()) {
            self.pending.insert(req, local);
        }
    }

    /// Konec ulohy relace.
    pub fn finish(&mut self, link: &mut dyn Link, local: Local, state: State) {
        match self.ids.remove(&local) {
            Some(id) => {
                link.send(Chan::Producer, Produce::Finish { id, state }.encode());
            }
            None if self.pending.values().any(|l| *l == local) => {
                self.early.insert(local, state);
            }
            None => {}
        }
    }

    /// Prichozi zpravy a prvni odber.
    pub fn pump(&mut self, link: &mut dyn Link) {
        if !self.subscribed {
            self.subscribed = link.send(Chan::Client, Request::Subscribe { req: 0, since: None }.encode());
        }
        for (chan, data) in link.recv() {
            match chan {
                Chan::Producer => {
                    if let Ok((req, id)) = taskproto::decode_registered(&data) {
                        let Some(local) = self.pending.remove(&req) else { continue };
                        match self.early.remove(&local) {
                            Some(state) => {
                                link.send(Chan::Producer, Produce::Finish { id, state }.encode());
                            }
                            None => {
                                self.ids.insert(local, id);
                            }
                        }
                    } else if let Ok(c) = Control::decode(&data) {
                        let id = match c {
                            Control::Cancel(i) | Control::Pause(i) | Control::Resume(i) => i,
                        };
                        if let Some((l, _)) = self.ids.iter().find(|(_, v)| **v == id) {
                            self.controls.push((*l, c));
                        }
                    }
                }
                Chan::Client => {
                    let Ok(r) = Reply::decode(&data) else { continue };
                    match &r {
                        Reply::Result { req, result } if *req != 0 => {
                            self.results.insert(*req, *result);
                        }
                        Reply::LogLines { req, lines } => {
                            self.logs.insert(*req, lines.clone());
                        }
                        _ => {
                            if matches!(self.list.apply(&r), taskproto::Applied::Gap) {
                                let since = self.list.seq;
                                link.send(Chan::Client, Request::Subscribe { req: 0, since: Some(since) }.encode());
                            }
                            self.changed = true;
                        }
                    }
                }
            }
        }
    }

    /// Akce nad ulohou (zrusit, pozastavit, pokracovat); vrati cislo
    /// pozadavku pro [`Board::result`] (`None` = taskd nebezi).
    pub fn action(&mut self, link: &mut dyn Link, id: u64, what: Action) -> Option<u32> {
        let req = self.req();
        let m = match what {
            Action::Cancel => Request::Cancel { req, id },
            Action::Pause => Request::Pause { req, id },
            Action::Resume => Request::Resume { req, id },
        };
        link.send(Chan::Client, m.encode()).then_some(req)
    }

    /// Vystup ulohy (poslednich `lines` radku).
    pub fn log(&mut self, link: &mut dyn Link, id: u64, lines: u32) -> Option<u32> {
        let req = self.req();
        link.send(Chan::Client, Request::Log { req, id, lines }.encode()).then_some(req)
    }

    /// Vysledek pozadavku `req`, kdyz uz prisel.
    pub fn result(&mut self, req: u32) -> Option<taskproto::Result<()>> {
        self.results.remove(&req)
    }

    pub fn log_lines(&mut self, req: u32) -> Option<Vec<String>> {
        self.logs.remove(&req)
    }

    /// Je prehled uloh dostupny (kanal klienta funguje)?
    pub fn available(&self) -> bool {
        self.subscribed
    }

    /// Bezici ulohy (stavovy radek okna).
    pub fn running(&self) -> usize {
        self.list.running().0
    }

    /// Je uloha relace hlasena (ma cislo v taskd)?
    pub fn known(&self, local: Local) -> bool {
        self.ids.contains_key(&local) || self.pending.values().any(|l| *l == local)
    }
}

/// Akce nad ulohou z okna nebo prikazu `tasks`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Cancel,
    Pause,
    Resume,
}

impl Action {
    pub fn from_u8(v: u8) -> Option<Self> {
        Some(match v {
            0 => Action::Cancel,
            1 => Action::Pause,
            2 => Action::Resume,
            _ => return None,
        })
    }

    pub fn to_u8(self) -> u8 {
        match self {
            Action::Cancel => 0,
            Action::Pause => 1,
            Action::Resume => 2,
        }
    }
}

/// Uloha jako zaznam shellu (`tasks`, tabulka).
pub fn record(t: &Task) -> shlang::value::Value {
    use shlang::value::Value;
    let state = match &t.state {
        State::Waiting(p) => alloc::format!("waiting: {p}"),
        State::Done(r) if !r.is_empty() => alloc::format!("done: {r}"),
        State::Failed { message, .. } => alloc::format!("failed: {message}"),
        s => String::from(s.name()),
    };
    let opt = |v: Option<u64>, f: fn(i64) -> Value| v.map_or(Value::Null, |x| f(x as i64));
    Value::record(&[
        ("id", Value::Int(t.id as i64)),
        ("name", Value::str(&t.name)),
        ("state", Value::str(&state)),
        ("owner", Value::str(t.origin.name())),
        ("who", Value::str(&t.who)),
        ("agent", Value::Bool(t.agent)),
        ("progress", t.progress.map_or(Value::Null, |p| Value::Int(p as i64))),
        ("eta", opt(t.eta_ms.map(|m| m * 1_000_000), Value::Duration)),
        ("cpu", opt(t.cpu_ns, Value::Duration)),
        ("memory", opt(t.memory, Value::Size)),
        ("started", if t.started == 0 { Value::Null } else { Value::Time(t.started as i64) }),
        ("ended", opt(t.ended, Value::Time)),
    ])
}
