//! Protokol sluzby uloh `taskd` (verze 1, NAVRH k odsouhlaseni vlaknem
//! shellu 6b, ktere taskd implementuje).
//!
//! Vsechno, co bezi na pozadi, je v jednom seznamu (viz
//! `docs/cs/architektura.md`, kapitola "Prehled uloh"). Dve strany:
//!
//! - **klient** (prikaz `tasks`, spravce uloh v GUI): [`Request`] ->
//!   [`Reply`]. Odber zmen zacne snimkem ([`Reply::Snapshot`]) a pokracuje
//!   udalostmi s poradovym cislem ([`Reply::Event`]); po vypadku jde navazat
//!   od posledniho cisla (`Subscribe { since }`).
//! - **producent** (shell, supervisor, sluzby, API akci): [`Produce`] -
//!   registrace ulohy, prubeh, stav, konec. Taskd je prijme jen na kanalu
//!   producenta (jina capability nez kanal klienta GUI nebo terminalu).
//!
//! Prava kontroluje taskd podle identity relace kanalu: klient vidi a ridi
//! jen sve ulohy, cizi a systemove jen s pravem.
//!
//! Zprava = kod operace `u32` + pole (`libs/wire`, little-endian). Cteni
//! neplatne zpravy vrati `INVALID_ARGS`, nikdy paniku.

#![cfg_attr(not(test), no_std)]

extern crate alloc;

#[cfg(test)]
mod tests;

use alloc::string::String;
use alloc::vec::Vec;

pub use abi::{Error, Result};
use wire::{Reader, Writer};

pub const VERSION: u32 = 1;

/// Nejvic uloh v jednom snimku a radku logu v odpovedi (zbytek dalsimi
/// zpravami - limit zpravy kanalu je 64 KiB).
pub const MAX_TASKS_PER_MSG: usize = 128;
pub const MAX_LOG_LINES: u32 = 1000;

/// Kdo ulohu spustil.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    User,
    Script,
    App,
    Service,
    /// Agent AI (ulohy agenta jsou v rozhrani vyrazne oznacene).
    Agent,
    /// Proces v jobu, ktery se neregistroval ("neznamy proces v jobu X").
    Unknown,
}

impl Origin {
    const fn to_u8(self) -> u8 {
        match self {
            Origin::User => 0,
            Origin::Script => 1,
            Origin::App => 2,
            Origin::Service => 3,
            Origin::Agent => 4,
            Origin::Unknown => 5,
        }
    }

    fn from_u8(v: u8) -> Result<Self> {
        Ok(match v {
            0 => Origin::User,
            1 => Origin::Script,
            2 => Origin::App,
            3 => Origin::Service,
            4 => Origin::Agent,
            5 => Origin::Unknown,
            _ => return Err(Error::InvalidArgs),
        })
    }

    pub const fn name(self) -> &'static str {
        match self {
            Origin::User => "user",
            Origin::Script => "script",
            Origin::App => "app",
            Origin::Service => "service",
            Origin::Agent => "agent",
            Origin::Unknown => "unknown",
        }
    }
}

/// Stav ulohy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum State {
    Running,
    Paused,
    /// Ceka na vyzvu nebo vstup (text vyzvy). Odpoved na vyzvu se dava
    /// pres `attach` v terminalu, ne pres tento protokol.
    Waiting(String),
    /// Zruseni pozadano, uloha jeste konci.
    Cancelling,
    /// Hotovo s vysledkem (text pro cloveka).
    Done(String),
    /// Chyba: kod (`abi::Error` nebo kod programu) a popis.
    Failed { code: i32, message: String },
    Cancelled,
}

impl State {
    /// Uloha uz skoncila (zustane chvili v seznamu i s vysledkem).
    pub const fn finished(&self) -> bool {
        matches!(self, State::Done(_) | State::Failed { .. } | State::Cancelled)
    }

    pub const fn name(&self) -> &'static str {
        match self {
            State::Running => "running",
            State::Paused => "paused",
            State::Waiting(_) => "waiting",
            State::Cancelling => "cancelling",
            State::Done(_) => "done",
            State::Failed { .. } => "failed",
            State::Cancelled => "cancelled",
        }
    }

    fn encode(&self, w: &mut Writer) {
        match self {
            State::Running => {
                w.u8(0);
            }
            State::Paused => {
                w.u8(1);
            }
            State::Waiting(p) => {
                w.u8(2).str(p);
            }
            State::Cancelling => {
                w.u8(3);
            }
            State::Done(r) => {
                w.u8(4).str(r);
            }
            State::Failed { code, message } => {
                w.u8(5).i32(*code).str(message);
            }
            State::Cancelled => {
                w.u8(6);
            }
        }
    }

    fn decode(r: &mut Reader<'_>) -> Result<Self> {
        Ok(match r.u8()? {
            0 => State::Running,
            1 => State::Paused,
            2 => State::Waiting(r.str()?),
            3 => State::Cancelling,
            4 => State::Done(r.str()?),
            5 => State::Failed { code: r.i32()?, message: r.str()? },
            6 => State::Cancelled,
            _ => return Err(Error::InvalidArgs),
        })
    }
}

/// Bity akci, ktere uloha podporuje (a smi je volajici pouzit).
pub mod actions {
    pub const CANCEL: u32 = 1 << 0;
    pub const PAUSE: u32 = 1 << 1;
    pub const RESUME: u32 = 1 << 2;
    pub const LOG: u32 = 1 << 3;
    /// Vystup jde otevrit v terminalu (`attach`).
    pub const ATTACH: u32 = 1 << 4;
}

/// Uloha v seznamu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Task {
    pub id: u64,
    /// Nadrazena uloha (podulohy async), 0 = zadna.
    pub parent: u64,
    /// Lidsky citelny nazev ("Kopirovani 3 souboru do /zaloha").
    pub name: String,
    pub origin: Origin,
    /// Identita spoustejiciho (aplikace, skript, uzivatel, agent).
    pub who: String,
    /// Spustil agent AI (vyrazne oznaceni v rozhrani).
    pub agent: bool,
    pub state: State,
    /// Prubeh 0-100; `None` = neznamy.
    pub progress: Option<u8>,
    /// Odhad zbyvajiciho casu (ms).
    pub eta_ms: Option<u64>,
    /// Spotreba z jobu jadra (ne kazda uloha ma vlastni job).
    pub memory: Option<u64>,
    pub cpu_ns: Option<u64>,
    /// Realny cas startu a konce (ns od epochy UTC; 0 = neznamy).
    pub started: u64,
    pub ended: Option<u64>,
    /// Vystup v terminalu: (relace termd, cislo bloku).
    pub attach: Option<(u64, u64)>,
    /// Akce, ktere smi volajici pouzit ([`actions`]).
    pub actions: u32,
}

fn opt_u64(w: &mut Writer, v: Option<u64>) {
    match v {
        Some(x) => w.u8(1).u64(x),
        None => w.u8(0),
    };
}

fn read_opt_u64(r: &mut Reader<'_>) -> Result<Option<u64>> {
    Ok(match r.u8()? {
        0 => None,
        1 => Some(r.u64()?),
        _ => return Err(Error::InvalidArgs),
    })
}

impl Task {
    /// Nejmensi delka zakodovane ulohy.
    pub const MIN_SIZE: usize = 8 + 8 + 2 + 1 + 2 + 1 + 1 + 1 + 1 + 1 + 1 + 8 + 1 + 1 + 4;

    pub fn encode(&self, w: &mut Writer) {
        w.u64(self.id).u64(self.parent).str(&self.name).u8(self.origin.to_u8()).str(&self.who).u8(self.agent as u8);
        self.state.encode(w);
        match self.progress {
            Some(p) => w.u8(1).u8(p.min(100)),
            None => w.u8(0),
        };
        opt_u64(w, self.eta_ms);
        opt_u64(w, self.memory);
        opt_u64(w, self.cpu_ns);
        w.u64(self.started);
        opt_u64(w, self.ended);
        match self.attach {
            Some((s, b)) => w.u8(1).u64(s).u64(b),
            None => w.u8(0),
        };
        w.u32(self.actions);
    }

    pub fn decode(r: &mut Reader<'_>) -> Result<Self> {
        let id = r.u64()?;
        let parent = r.u64()?;
        let name = r.str()?;
        let origin = Origin::from_u8(r.u8()?)?;
        let who = r.str()?;
        let agent = match r.u8()? {
            0 => false,
            1 => true,
            _ => return Err(Error::InvalidArgs),
        };
        let state = State::decode(r)?;
        let progress = match r.u8()? {
            0 => None,
            1 => Some(r.u8()?.min(100)),
            _ => return Err(Error::InvalidArgs),
        };
        let eta_ms = read_opt_u64(r)?;
        let memory = read_opt_u64(r)?;
        let cpu_ns = read_opt_u64(r)?;
        let started = r.u64()?;
        let ended = read_opt_u64(r)?;
        let attach = match r.u8()? {
            0 => None,
            1 => Some((r.u64()?, r.u64()?)),
            _ => return Err(Error::InvalidArgs),
        };
        Ok(Self {
            id,
            parent,
            name,
            origin,
            who,
            agent,
            state,
            progress,
            eta_ms,
            memory,
            cpu_ns,
            started,
            ended,
            attach,
            actions: r.u32()?,
        })
    }
}

/// Kody operaci.
pub mod op {
    // Klient -> taskd.
    pub const HELLO: u32 = 1;
    pub const SUBSCRIBE: u32 = 2;
    pub const UNSUBSCRIBE: u32 = 3;
    pub const CANCEL: u32 = 4;
    pub const PAUSE: u32 = 5;
    pub const RESUME: u32 = 6;
    pub const LOG: u32 = 7;
    // Producent -> taskd.
    pub const REGISTER: u32 = 20;
    pub const UPDATE: u32 = 21;
    pub const FINISH: u32 = 22;
    // taskd -> klient / producent.
    pub const SNAPSHOT: u32 = 100;
    pub const EVENT: u32 = 101;
    pub const RESULT: u32 = 102;
    pub const LOG_LINES: u32 = 103;
    pub const REGISTERED: u32 = 104;
    /// Taskd zada producenta o akci (zrusit, pozastavit, pokracovat).
    pub const CONTROL: u32 = 105;
}

/// Zprava klienta. `req` = cislo pozadavku, ktere se vrati v odpovedi.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Request {
    Hello { version: u32 },
    /// Prihlasit odber: snimek a pak udalosti. `since` = posledni prijate
    /// poradove cislo (navazani po vypadku: taskd posle chybejici udalosti,
    /// nebo novy snimek, kdyz uz je nema).
    Subscribe { req: u32, since: Option<u64> },
    Unsubscribe { req: u32 },
    Cancel { req: u32, id: u64 },
    Pause { req: u32, id: u64 },
    Resume { req: u32, id: u64 },
    /// Poslednich `lines` radku nebo polozek vystupu.
    Log { req: u32, id: u64, lines: u32 },
}

/// Zmena seznamu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Change {
    Added(Task),
    Updated(Task),
    Removed(u64),
}

/// Zprava taskd klientovi.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reply {
    /// Cast snimku: `seq` = cislo posledni zahrnute zmeny, `last` = posledni
    /// cast (snimek muze mit vic zprav).
    Snapshot { req: u32, seq: u64, tasks: Vec<Task>, last: bool },
    Event { seq: u64, change: Change },
    /// Vysledek akce: `Ok(())` nebo kod chyby (`ACCESS_DENIED`, `NOT_FOUND`,
    /// `NOT_SUPPORTED` ...).
    Result { req: u32, result: Result<()> },
    LogLines { req: u32, lines: Vec<String> },
}

/// Zprava producenta (shell, supervisor, sluzba, API akci).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Produce {
    /// Nova uloha; `actions` = co producent umi (taskd je zpristupni podle
    /// prav volajicich). Odpoved `REGISTERED { req, id }`.
    Register { req: u32, parent: u64, name: String, origin: Origin, who: String, agent: bool, job: Option<u64>, attach: Option<(u64, u64)>, actions: u32 },
    Update { id: u64, state: State, progress: Option<u8>, eta_ms: Option<u64> },
    Finish { id: u64, state: State },
}

/// Pozadavek taskd na producenta.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Control {
    Cancel(u64),
    Pause(u64),
    Resume(u64),
}

fn finish(r: &Reader<'_>) -> Result<()> {
    if r.is_empty() { Ok(()) } else { Err(Error::InvalidArgs) }
}

fn opt_seq(r: &mut Reader<'_>) -> Result<Option<u64>> {
    read_opt_u64(r)
}

impl Request {
    pub fn encode(&self) -> Vec<u8> {
        let mut w;
        match self {
            Request::Hello { version } => {
                w = Writer::op(op::HELLO);
                w.u32(*version);
            }
            Request::Subscribe { req, since } => {
                w = Writer::op(op::SUBSCRIBE);
                w.u32(*req);
                opt_u64(&mut w, *since);
            }
            Request::Unsubscribe { req } => {
                w = Writer::op(op::UNSUBSCRIBE);
                w.u32(*req);
            }
            Request::Cancel { req, id } => {
                w = Writer::op(op::CANCEL);
                w.u32(*req).u64(*id);
            }
            Request::Pause { req, id } => {
                w = Writer::op(op::PAUSE);
                w.u32(*req).u64(*id);
            }
            Request::Resume { req, id } => {
                w = Writer::op(op::RESUME);
                w.u32(*req).u64(*id);
            }
            Request::Log { req, id, lines } => {
                w = Writer::op(op::LOG);
                w.u32(*req).u64(*id).u32(*lines);
            }
        }
        w.finish()
    }

    pub fn decode(data: &[u8]) -> Result<Self> {
        let mut r = Reader::new(data);
        let m = match r.u32()? {
            op::HELLO => Request::Hello { version: r.u32()? },
            op::SUBSCRIBE => Request::Subscribe { req: r.u32()?, since: opt_seq(&mut r)? },
            op::UNSUBSCRIBE => Request::Unsubscribe { req: r.u32()? },
            op::CANCEL => Request::Cancel { req: r.u32()?, id: r.u64()? },
            op::PAUSE => Request::Pause { req: r.u32()?, id: r.u64()? },
            op::RESUME => Request::Resume { req: r.u32()?, id: r.u64()? },
            op::LOG => Request::Log { req: r.u32()?, id: r.u64()?, lines: r.u32()?.min(MAX_LOG_LINES) },
            _ => return Err(Error::NotSupported),
        };
        finish(&r)?;
        Ok(m)
    }
}

impl Reply {
    pub fn encode(&self) -> Vec<u8> {
        let mut w;
        match self {
            Reply::Snapshot { req, seq, tasks, last } => {
                w = Writer::op(op::SNAPSHOT);
                w.u32(*req).u64(*seq).u8(*last as u8).u32(tasks.len() as u32);
                for t in tasks {
                    t.encode(&mut w);
                }
            }
            Reply::Event { seq, change } => {
                w = Writer::op(op::EVENT);
                w.u64(*seq);
                match change {
                    Change::Added(t) => {
                        w.u8(0);
                        t.encode(&mut w);
                    }
                    Change::Updated(t) => {
                        w.u8(1);
                        t.encode(&mut w);
                    }
                    Change::Removed(id) => {
                        w.u8(2).u64(*id);
                    }
                }
            }
            Reply::Result { req, result } => {
                w = Writer::op(op::RESULT);
                w.u32(*req).i32(match result {
                    Ok(()) => 0,
                    Err(e) => e.code(),
                });
            }
            Reply::LogLines { req, lines } => {
                w = Writer::op(op::LOG_LINES);
                w.u32(*req).u32(lines.len() as u32);
                for l in lines {
                    w.str(l);
                }
            }
        }
        w.finish()
    }

    pub fn decode(data: &[u8]) -> Result<Self> {
        let mut r = Reader::new(data);
        let len = data.len();
        let m = match r.u32()? {
            op::SNAPSHOT => {
                let req = r.u32()?;
                let seq = r.u64()?;
                let last = r.u8()? != 0;
                let n = r.u32()? as usize;
                if n > MAX_TASKS_PER_MSG.min(len / Task::MIN_SIZE) || n.saturating_mul(Task::MIN_SIZE) > len {
                    return Err(Error::InvalidArgs);
                }
                let mut tasks = Vec::with_capacity(n);
                for _ in 0..n {
                    tasks.push(Task::decode(&mut r)?);
                }
                Reply::Snapshot { req, seq, tasks, last }
            }
            op::EVENT => {
                let seq = r.u64()?;
                let change = match r.u8()? {
                    0 => Change::Added(Task::decode(&mut r)?),
                    1 => Change::Updated(Task::decode(&mut r)?),
                    2 => Change::Removed(r.u64()?),
                    _ => return Err(Error::InvalidArgs),
                };
                Reply::Event { seq, change }
            }
            op::RESULT => {
                let req = r.u32()?;
                let code = r.i32()?;
                let result = if code == 0 { Ok(()) } else { Err(Error::from_code(code as i64).ok_or(Error::InvalidArgs)?) };
                Reply::Result { req, result }
            }
            op::LOG_LINES => {
                let req = r.u32()?;
                let n = r.u32()? as usize;
                if n.saturating_mul(2) > len {
                    return Err(Error::InvalidArgs);
                }
                let mut lines = Vec::with_capacity(n);
                for _ in 0..n {
                    lines.push(r.str()?);
                }
                Reply::LogLines { req, lines }
            }
            _ => return Err(Error::NotSupported),
        };
        finish(&r)?;
        Ok(m)
    }
}

fn opt_pair(w: &mut Writer, v: Option<(u64, u64)>) {
    match v {
        Some((a, b)) => w.u8(1).u64(a).u64(b),
        None => w.u8(0),
    };
}

impl Produce {
    pub fn encode(&self) -> Vec<u8> {
        let mut w;
        match self {
            Produce::Register { req, parent, name, origin, who, agent, job, attach, actions } => {
                w = Writer::op(op::REGISTER);
                w.u32(*req).u64(*parent).str(name).u8(origin.to_u8()).str(who).u8(*agent as u8);
                opt_u64(&mut w, *job);
                opt_pair(&mut w, *attach);
                w.u32(*actions);
            }
            Produce::Update { id, state, progress, eta_ms } => {
                w = Writer::op(op::UPDATE);
                w.u64(*id);
                state.encode(&mut w);
                match progress {
                    Some(p) => w.u8(1).u8((*p).min(100)),
                    None => w.u8(0),
                };
                opt_u64(&mut w, *eta_ms);
            }
            Produce::Finish { id, state } => {
                w = Writer::op(op::FINISH);
                w.u64(*id);
                state.encode(&mut w);
            }
        }
        w.finish()
    }

    pub fn decode(data: &[u8]) -> Result<Self> {
        let mut r = Reader::new(data);
        let m = match r.u32()? {
            op::REGISTER => {
                let req = r.u32()?;
                let parent = r.u64()?;
                let name = r.str()?;
                let origin = Origin::from_u8(r.u8()?)?;
                let who = r.str()?;
                let agent = r.u8()? == 1;
                let job = read_opt_u64(&mut r)?;
                let attach = match r.u8()? {
                    0 => None,
                    1 => Some((r.u64()?, r.u64()?)),
                    _ => return Err(Error::InvalidArgs),
                };
                Produce::Register { req, parent, name, origin, who, agent, job, attach, actions: r.u32()? }
            }
            op::UPDATE => {
                let id = r.u64()?;
                let state = State::decode(&mut r)?;
                let progress = match r.u8()? {
                    0 => None,
                    1 => Some(r.u8()?.min(100)),
                    _ => return Err(Error::InvalidArgs),
                };
                Produce::Update { id, state, progress, eta_ms: read_opt_u64(&mut r)? }
            }
            op::FINISH => Produce::Finish { id: r.u64()?, state: State::decode(&mut r)? },
            _ => return Err(Error::NotSupported),
        };
        finish(&r)?;
        Ok(m)
    }
}

/// `REGISTERED { req, id }` (taskd -> producent).
pub fn encode_registered(req: u32, id: u64) -> Vec<u8> {
    let mut w = Writer::op(op::REGISTERED);
    w.u32(req).u64(id);
    w.finish()
}

pub fn decode_registered(data: &[u8]) -> Result<(u32, u64)> {
    let mut r = Reader::new(data);
    if r.u32()? != op::REGISTERED {
        return Err(Error::InvalidArgs);
    }
    let v = (r.u32()?, r.u64()?);
    finish(&r)?;
    Ok(v)
}

impl Control {
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::op(op::CONTROL);
        match self {
            Control::Cancel(id) => w.u8(0).u64(*id),
            Control::Pause(id) => w.u8(1).u64(*id),
            Control::Resume(id) => w.u8(2).u64(*id),
        };
        w.finish()
    }

    pub fn decode(data: &[u8]) -> Result<Self> {
        let mut r = Reader::new(data);
        if r.u32()? != op::CONTROL {
            return Err(Error::InvalidArgs);
        }
        let m = match r.u8()? {
            0 => Control::Cancel(r.u64()?),
            1 => Control::Pause(r.u64()?),
            2 => Control::Resume(r.u64()?),
            _ => return Err(Error::InvalidArgs),
        };
        finish(&r)?;
        Ok(m)
    }
}

/// Seznam uloh u klienta: uplatnuje snimek a udalosti, hlida poradi.
#[derive(Debug, Default, Clone)]
pub struct TaskList {
    pub tasks: Vec<Task>,
    /// Posledni uplatnene poradove cislo.
    pub seq: u64,
    /// Snimek se skladal z vic zprav a jeste neni cely.
    building: bool,
}

/// Vysledek uplatneni zpravy na seznam.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Applied {
    Ok,
    /// Udalost preskocila cislo: klient ma poslat `Subscribe { since: seq }`.
    Gap,
    Ignored,
}

impl TaskList {
    pub fn apply(&mut self, m: &Reply) -> Applied {
        match m {
            Reply::Snapshot { seq, tasks, last, .. } => {
                if !self.building {
                    self.tasks.clear();
                }
                self.tasks.extend(tasks.iter().cloned());
                self.building = !last;
                self.seq = *seq;
                Applied::Ok
            }
            Reply::Event { seq, change } => {
                if *seq <= self.seq {
                    return Applied::Ignored;
                }
                if *seq != self.seq + 1 {
                    return Applied::Gap;
                }
                self.seq = *seq;
                match change {
                    Change::Added(t) | Change::Updated(t) => match self.tasks.iter_mut().find(|x| x.id == t.id) {
                        Some(x) => *x = t.clone(),
                        None => self.tasks.push(t.clone()),
                    },
                    Change::Removed(id) => self.tasks.retain(|x| x.id != *id),
                }
                Applied::Ok
            }
            _ => Applied::Ignored,
        }
    }

    /// Pocet bezicich uloh a z toho uloh agenta (ikona v liste).
    pub fn running(&self) -> (usize, usize) {
        let run: Vec<&Task> = self.tasks.iter().filter(|t| !t.state.finished()).collect();
        (run.len(), run.iter().filter(|t| t.agent).count())
    }
}
