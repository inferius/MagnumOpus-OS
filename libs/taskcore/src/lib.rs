//! Jadro sluzby uloh `taskd` (bez systemovych volani - testuje se na
//! hostiteli; proces `user/taskd` jen predava zpravy kanalu).
//!
//! - **Ulohy** registruji producenti ([`taskproto::Produce`]): shell
//!   (ulohy `async`, `bg`, relace), supervisor (sluzby), fsd (scrub,
//!   deduplikace, kontrola). Kazdy producent ma vlastni kanal s identitou,
//!   kterou mu prirazuje init ([`Core::add_producer`]); uloha patri
//!   identite producenta.
//! - **Klienti** ([`taskproto::Request`]): prikaz `tasks`, panel uloh
//!   v terminalu, spravce uloh v GUI, ikona v liste kompozitoru. Kanal
//!   klienta ma rozsah ([`Scope`]): vlastni ulohy, vsechny, a zda smi
//!   ridit (zrusit, pozastavit).
//! - **Odber:** snimek a pak udalosti s globalnim poradovym cislem. Zmenu
//!   ulohy, kterou klient nevidi, dostane jako `Removed(id)` (bez obsahu),
//!   takze poradova cisla jdou u kazdeho klienta bez der a navazani od
//!   `since` funguje (posledni [`HISTORY`] zmen, jinak novy snimek).
//! - **Hotove ulohy** zustanou chvili i s vysledkem: nejvys
//!   [`KEEP_FINISHED`] a nejdel [`KEEP_FINISHED_NS`].
//! - **Neznamy proces:** procesy ve sledovanych jobech (aplikace), ktere
//!   zadna uloha nehlasi ([`Core::processes`]), se objevi jako uloha puvodu
//!   `Unknown` - nic nezustane skryte.

#![cfg_attr(not(test), no_std)]

extern crate alloc;

#[cfg(test)]
mod tests;

use alloc::collections::{BTreeMap, VecDeque};
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use abi::job::{ProcEntry, proc_flags};
use taskproto::{Change, Control, Error, Origin, Produce, Reply, Request, State, Task, actions};

/// Kolik hotovych uloh si taskd pamatuje.
pub const KEEP_FINISHED: usize = 100;
/// Jak dlouho drzi hotovou ulohu (30 min).
pub const KEEP_FINISHED_NS: u64 = 30 * 60 * 1_000_000_000;
/// Kolik poslednich zmen jde dohnat pres `Subscribe { since }`.
pub const HISTORY: usize = 512;
/// Kolik radku vystupu drzi taskd u ulohy.
pub const KEEP_LOG: usize = 200;

pub type ClientId = u64;
pub type ProducerId = u64;

/// Rozsah kanalu klienta (razi ho init).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Scope {
    /// Identita (vlastni ulohy = ulohy producentu teto identity).
    pub identity: String,
    /// Vidi i cizi a systemove ulohy.
    pub all: bool,
    /// Smi ridit (zrusit, pozastavit, pokracovat) ulohy, ktere vidi.
    pub control: bool,
}

/// Zprava k odeslani.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Out {
    Client(ClientId, Vec<u8>),
    Producer(ProducerId, Vec<u8>),
}

struct Entry {
    task: Task,
    /// Identita vlastnika (producenta).
    owner: String,
    /// Producent (`None` = taskd sam: neznamy proces).
    producer: Option<ProducerId>,
    /// Job ulohy (procesy v nem patri k uloze).
    job: Option<u64>,
    /// Neznamy proces: cislo procesu.
    pid: Option<u64>,
    log: VecDeque<String>,
}

struct Client {
    scope: Scope,
    subscribed: bool,
}

struct Producer {
    identity: String,
}

#[derive(Default)]
pub struct Core {
    tasks: BTreeMap<u64, Entry>,
    next_id: u64,
    seq: u64,
    history: VecDeque<(u64, u64)>,
    clients: BTreeMap<ClientId, Client>,
    producers: BTreeMap<ProducerId, Producer>,
    next_conn: u64,
}

fn reply(c: ClientId, r: &Reply) -> Out {
    Out::Client(c, r.encode())
}

impl Core {
    pub fn new() -> Self {
        Self { next_id: 1, next_conn: 1, ..Self::default() }
    }

    pub fn add_client(&mut self, scope: Scope) -> ClientId {
        let id = self.next_conn;
        self.next_conn += 1;
        self.clients.insert(id, Client { scope, subscribed: false });
        id
    }

    pub fn remove_client(&mut self, id: ClientId) {
        self.clients.remove(&id);
    }

    pub fn add_producer(&mut self, identity: &str) -> ProducerId {
        let id = self.next_conn;
        self.next_conn += 1;
        self.producers.insert(id, Producer { identity: identity.into() });
        id
    }

    /// Producent skoncil (kanal zavren): jeho bezici ulohy skonci chybou.
    pub fn remove_producer(&mut self, id: ProducerId, now: u64) -> Vec<Out> {
        self.producers.remove(&id);
        let ids: Vec<u64> = self
            .tasks
            .iter()
            .filter(|(_, e)| e.producer == Some(id) && !e.task.state.finished())
            .map(|(i, _)| *i)
            .collect();
        let mut out = Vec::new();
        for t in ids {
            let st = State::Failed { code: Error::PeerClosed as i32, message: "producent ulohy skoncil".into() };
            out.extend(self.set_state(t, st, now));
        }
        out
    }

    /// Bezici ulohy (vsechny) a z toho ulohy agenta - ikona v liste.
    pub fn running(&self) -> (usize, usize) {
        let run = self.tasks.values().filter(|e| !e.task.state.finished());
        let (mut n, mut a) = (0, 0);
        for e in run {
            n += 1;
            a += e.task.agent as usize;
        }
        (n, a)
    }

    fn visible(scope: &Scope, e: &Entry) -> bool {
        scope.all || e.owner == scope.identity
    }

    /// Pohled klienta na ulohu: akce jen ty, ktere smi.
    fn view(scope: &Scope, e: &Entry) -> Task {
        let mut t = e.task.clone();
        let mut allowed = actions::LOG | actions::ATTACH;
        if scope.control {
            allowed |= actions::CANCEL | actions::PAUSE | actions::RESUME;
        }
        t.actions &= allowed;
        if e.owner != scope.identity && !scope.all {
            t.actions = 0;
        }
        t
    }

    /// Zaznam zmeny ulohy `id` a udalost odebiratelum.
    fn changed(&mut self, id: u64, added: bool) -> Vec<Out> {
        self.seq += 1;
        let seq = self.seq;
        self.history.push_back((seq, id));
        if self.history.len() > HISTORY {
            self.history.pop_front();
        }
        let mut out = Vec::new();
        for (cid, c) in &self.clients {
            if !c.subscribed {
                continue;
            }
            let change = self.change_for(&c.scope, id, added);
            out.push(reply(*cid, &Reply::Event { seq, change }));
        }
        out
    }

    fn change_for(&self, scope: &Scope, id: u64, added: bool) -> Change {
        match self.tasks.get(&id) {
            Some(e) if Self::visible(scope, e) => {
                let t = Self::view(scope, e);
                if added { Change::Added(t) } else { Change::Updated(t) }
            }
            _ => Change::Removed(id),
        }
    }

    fn snapshot(&self, c: ClientId, req: u32) -> Vec<Out> {
        let Some(cl) = self.clients.get(&c) else { return Vec::new() };
        let tasks: Vec<Task> =
            self.tasks.values().filter(|e| Self::visible(&cl.scope, e)).map(|e| Self::view(&cl.scope, e)).collect();
        let mut out = Vec::new();
        let chunks: Vec<&[Task]> = tasks.chunks(taskproto::MAX_TASKS_PER_MSG).collect();
        if chunks.is_empty() {
            out.push(reply(c, &Reply::Snapshot { req, seq: self.seq, tasks: Vec::new(), last: true }));
        }
        for (i, ch) in chunks.iter().enumerate() {
            let last = i + 1 == chunks.len();
            out.push(reply(c, &Reply::Snapshot { req, seq: self.seq, tasks: ch.to_vec(), last }));
        }
        out
    }

    /// Zprava od klienta (neplatna = `INVALID_ARGS`, nikdy panika).
    pub fn client_msg(&mut self, c: ClientId, data: &[u8], _now: u64) -> Vec<Out> {
        let Ok(req) = Request::decode(data) else {
            return alloc::vec![reply(c, &Reply::Result { req: 0, result: Err(Error::InvalidArgs) })];
        };
        match req {
            Request::Hello { .. } => Vec::new(),
            Request::Subscribe { req, since } => {
                let Some(cl) = self.clients.get_mut(&c) else { return Vec::new() };
                cl.subscribed = true;
                // Dohnat z historie, kdyz ji jeste mame, jinak snimek.
                if let Some(since) = since
                    && since <= self.seq
                    && self.history.front().is_some_and(|(s, _)| *s <= since + 1)
                {
                    let scope = cl.scope.clone();
                    let mut out = Vec::new();
                    for (seq, id) in self.history.iter().filter(|(s, _)| *s > since) {
                        let change = self.change_for(&scope, *id, false);
                        out.push(reply(c, &Reply::Event { seq: *seq, change }));
                    }
                    out.push(reply(c, &Reply::Result { req, result: Ok(()) }));
                    return out;
                }
                self.snapshot(c, req)
            }
            Request::Unsubscribe { req } => {
                if let Some(cl) = self.clients.get_mut(&c) {
                    cl.subscribed = false;
                }
                alloc::vec![reply(c, &Reply::Result { req, result: Ok(()) })]
            }
            Request::Cancel { req, id } => self.control(c, req, id, actions::CANCEL, Control::Cancel(id)),
            Request::Pause { req, id } => self.control(c, req, id, actions::PAUSE, Control::Pause(id)),
            Request::Resume { req, id } => self.control(c, req, id, actions::RESUME, Control::Resume(id)),
            Request::Log { req, id, lines } => {
                let r = match (self.clients.get(&c), self.tasks.get(&id)) {
                    (Some(cl), Some(e)) if Self::visible(&cl.scope, e) => {
                        let skip = e.log.len().saturating_sub(lines as usize);
                        Reply::LogLines { req, lines: e.log.iter().skip(skip).cloned().collect() }
                    }
                    _ => Reply::Result { req, result: Err(Error::NotFound) },
                };
                alloc::vec![reply(c, &r)]
            }
        }
    }

    /// Akce nad ulohou: smi ji klient (rozsah, vlastnik) a umi ji
    /// producent? Pak ji preda producentovi.
    fn control(&mut self, c: ClientId, req: u32, id: u64, bit: u32, ctl: Control) -> Vec<Out> {
        let Some(cl) = self.clients.get(&c) else { return Vec::new() };
        let result = match self.tasks.get(&id) {
            None => Err(Error::NotFound),
            Some(e) if !Self::visible(&cl.scope, e) => Err(Error::NotFound),
            Some(e) if Self::view(&cl.scope, e).actions & bit == 0 => {
                if !cl.scope.control || (e.owner != cl.scope.identity && !cl.scope.all) {
                    Err(Error::AccessDenied)
                } else {
                    Err(Error::NotSupported)
                }
            }
            Some(e) if e.task.state.finished() => Err(Error::BadState),
            Some(e) => match e.producer {
                Some(p) if self.producers.contains_key(&p) => {
                    let mut out = alloc::vec![Out::Producer(p, ctl.encode())];
                    out.push(reply(c, &Reply::Result { req, result: Ok(()) }));
                    return out;
                }
                _ => Err(Error::NotSupported),
            },
        };
        alloc::vec![reply(c, &Reply::Result { req, result })]
    }

    fn own(&self, p: ProducerId, id: u64) -> bool {
        self.tasks.get(&id).is_some_and(|e| e.producer == Some(p))
    }

    fn set_state(&mut self, id: u64, state: State, now: u64) -> Vec<Out> {
        let Some(e) = self.tasks.get_mut(&id) else { return Vec::new() };
        if state.finished() && e.task.ended.is_none() {
            e.task.ended = Some(now);
            e.task.progress = if matches!(state, State::Done(_)) { Some(100) } else { e.task.progress };
            e.task.eta_ms = None;
        }
        e.task.state = state;
        let mut out = self.changed(id, false);
        out.extend(self.retain(now));
        out
    }

    /// Zprava od producenta.
    pub fn producer_msg(&mut self, p: ProducerId, data: &[u8], now: u64) -> Vec<Out> {
        let Some(identity) = self.producers.get(&p).map(|x| x.identity.clone()) else { return Vec::new() };
        let Ok(m) = Produce::decode(data) else { return Vec::new() };
        match m {
            Produce::Register { req, parent, name, origin, who, agent, job, attach, actions: acts } => {
                // Producent nesmi vydavat ulohu za neznamy proces.
                let origin = if origin == Origin::Unknown { Origin::App } else { origin };
                let id = self.next_id;
                self.next_id += 1;
                let task = Task {
                    id,
                    parent: if self.tasks.contains_key(&parent) { parent } else { 0 },
                    name,
                    origin,
                    who,
                    agent,
                    state: State::Running,
                    progress: None,
                    eta_ms: None,
                    memory: None,
                    cpu_ns: None,
                    started: now,
                    ended: None,
                    attach,
                    actions: acts,
                };
                self.tasks.insert(id, Entry { task, owner: identity, producer: Some(p), job, pid: None, log: VecDeque::new() });
                let mut out = alloc::vec![Out::Producer(p, taskproto::encode_registered(req, id))];
                out.extend(self.changed(id, true));
                out
            }
            Produce::Update { id, state, progress, eta_ms } => {
                if !self.own(p, id) {
                    return Vec::new();
                }
                if state.finished() {
                    return self.set_state(id, state, now);
                }
                let e = self.tasks.get_mut(&id).unwrap();
                if e.task.state.finished() {
                    return Vec::new();
                }
                e.task.state = state;
                e.task.progress = progress;
                e.task.eta_ms = eta_ms;
                self.changed(id, false)
            }
            Produce::Finish { id, state } => {
                if !self.own(p, id) {
                    return Vec::new();
                }
                // Konec bez koncoveho stavu = hotovo.
                let state = if state.finished() { state } else { State::Done(String::new()) };
                self.set_state(id, state, now)
            }
            Produce::LogLine { id, line } => {
                if let Some(e) = self.tasks.get_mut(&id)
                    && e.producer == Some(p)
                {
                    e.log.push_back(line);
                    if e.log.len() > KEEP_LOG {
                        e.log.pop_front();
                    }
                }
                Vec::new()
            }
        }
    }

    /// Uklid hotovych uloh (pocet a stari).
    fn retain(&mut self, now: u64) -> Vec<Out> {
        let mut done: Vec<(u64, u64)> = self
            .tasks
            .iter()
            .filter_map(|(id, e)| e.task.ended.filter(|_| e.task.state.finished()).map(|t| (t, *id)))
            .collect();
        done.sort_unstable();
        let over = done.len().saturating_sub(KEEP_FINISHED);
        let old: Vec<u64> = done
            .iter()
            .enumerate()
            .filter(|(i, (t, _))| *i < over || now.saturating_sub(*t) > KEEP_FINISHED_NS)
            .map(|(_, (_, id))| *id)
            .collect();
        let mut out = Vec::new();
        for id in old {
            self.tasks.remove(&id);
            out.extend(self.changed(id, false));
        }
        out
    }

    /// Volat pravidelne: uklid starych hotovych uloh.
    pub fn tick(&mut self, now: u64) -> Vec<Out> {
        self.retain(now)
    }

    /// Procesy sledovanych jobu (vycet jadra `JOB_PROCESSES`) a pamet jobu:
    /// cas CPU a pamet uloh s jobem, neznamy proces (v jobu, ktery zadna
    /// bezici uloha nehlasi) jako uloha puvodu `Unknown`.
    pub fn processes(&mut self, procs: &[ProcEntry], memory: &[(u64, u64)], now: u64) -> Vec<Out> {
        let mut out = Vec::new();
        let claimed: BTreeMap<u64, u64> = self
            .tasks
            .iter()
            .filter(|(_, e)| e.pid.is_none() && !e.task.state.finished())
            .filter_map(|(id, e)| e.job.map(|j| (j, *id)))
            .collect();
        // Cas CPU a pamet uloh s jobem.
        let mut cpu: BTreeMap<u64, u64> = BTreeMap::new();
        for p in procs {
            if let Some(t) = claimed.get(&p.job) {
                *cpu.entry(*t).or_default() += p.cpu_ns;
            }
        }
        for (job, tid) in &claimed {
            let mem = memory.iter().find(|(j, _)| j == job).map(|(_, m)| *m);
            let c = cpu.get(tid).copied();
            let e = self.tasks.get_mut(tid).unwrap();
            if e.task.cpu_ns != c || e.task.memory != mem {
                e.task.cpu_ns = c;
                e.task.memory = mem;
                out.extend(self.changed(*tid, false));
            }
        }
        // Neznamy proces: nove se objevi, skonceny skonci.
        let live: Vec<&ProcEntry> =
            procs.iter().filter(|p| p.flags & proc_flags::FINISHED == 0 && !claimed.contains_key(&p.job)).collect();
        let known: BTreeMap<u64, u64> = self
            .tasks
            .iter()
            .filter(|(_, e)| !e.task.state.finished())
            .filter_map(|(id, e)| e.pid.map(|p| (p, *id)))
            .collect();
        for p in &live {
            match known.get(&p.pid) {
                Some(tid) => {
                    let e = self.tasks.get_mut(tid).unwrap();
                    if e.task.cpu_ns != Some(p.cpu_ns) {
                        e.task.cpu_ns = Some(p.cpu_ns);
                        out.extend(self.changed(*tid, false));
                    }
                }
                None => {
                    let id = self.next_id;
                    self.next_id += 1;
                    let task = Task {
                        id,
                        parent: 0,
                        name: format!("neznámý proces {} v jobu {}", p.name(), p.job),
                        origin: Origin::Unknown,
                        who: format!("job {}", p.job),
                        agent: false,
                        state: State::Running,
                        progress: None,
                        eta_ms: None,
                        memory: memory.iter().find(|(j, _)| *j == p.job).map(|(_, m)| *m),
                        cpu_ns: Some(p.cpu_ns),
                        started: now,
                        ended: None,
                        attach: None,
                        actions: 0,
                    };
                    self.tasks.insert(
                        id,
                        Entry { task, owner: String::from("system"), producer: None, job: Some(p.job), pid: Some(p.pid), log: VecDeque::new() },
                    );
                    out.extend(self.changed(id, true));
                }
            }
        }
        let gone: Vec<u64> = known
            .iter()
            .filter(|(pid, _)| !live.iter().any(|p| p.pid == **pid))
            .map(|(_, tid)| *tid)
            .collect();
        for tid in gone {
            out.extend(self.set_state(tid, State::Done(String::from("proces skoncil")), now));
        }
        out
    }
}
