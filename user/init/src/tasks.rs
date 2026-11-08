//! Prehled uloh: sluzba `taskd` (viz `user/taskd`, `libs/taskcore`).
//!
//! Init taskd spusti, drzi jeho ridici kanal a jako jediny razi kanaly
//! s identitou a rozsahem:
//! - **klient** ([`client`]): prikaz `tasks` a panel uloh v terminalu
//!   (identita uzivatele, vlastni ulohy), spravce uloh v GUI,
//! - **producent** ([`producer`]): shell (relace), fsd (scrub, deduplikace),
//!   init sam (systemove sluzby ze supervisoru),
//! - **sledovany job** ([`watch`]): procesy aplikaci; co se v jobu
//!   neregistruje, ukaze taskd jako neznamy proces.
//!
//! [`sync`] vola supervisor po kazdem kole: stav sluzeb do taskd a pocet
//! bezicich uloh do ikony v liste kompozitoru (`tasks N A`).

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::time::Duration;

use libos::initfs::Initfs;
use libos::job::Job;
use libos::{Channel, Error, Handle, Process, Result, Rights, Startup, println, time};
use taskproto::{Origin, Produce, Reply, Request, State as TaskState, TaskList};

use crate::supervisor::{State, Supervisor};

const REPLY_TIMEOUT: Duration = Duration::from_secs(5);

pub struct Tasks {
    pub process: Process,
    ctl: Channel,
    /// Producent "system" (sluzby).
    producer: Channel,
    /// Klient se shrnutim (ikona v liste): vidi vse, neridi.
    summary: Channel,
    list: TaskList,
    /// Sluzba -> (cislo ulohy, posledni stav); `None` = ceka na REGISTERED.
    services: BTreeMap<&'static str, (Option<u64>, State)>,
    /// Cislo pozadavku registrace -> sluzba.
    pending: BTreeMap<u32, &'static str>,
    next_req: u32,
    /// Posledni pocet poslany kompozitoru.
    icon: Option<(usize, usize)>,
}

/// Bezici taskd (bezny i testovaci beh).
pub static TASKS: spin::Mutex<Option<Tasks>> = spin::Mutex::new(None);

fn command(ctl: &Channel, text: &str, handles: Vec<Handle>) -> Result<Vec<Handle>> {
    ctl.write(text.as_bytes(), handles)?;
    let m = ctl.read(Some(time::deadline_after(REPLY_TIMEOUT)))?;
    let reply = core::str::from_utf8(&m.data).map_err(|_| Error::Corrupted)?;
    if let Some(code) = reply.strip_prefix("err ") {
        return Err(libos::abi::error::ALL.iter().copied().find(|e| e.name() == code).unwrap_or(Error::Corrupted));
    }
    Ok(m.handles)
}

fn one(hs: Vec<Handle>) -> Result<Channel> {
    hs.into_iter().next().map(Channel::from_handle).ok_or(Error::Corrupted)
}

/// Spusti taskd (z initfs).
pub fn start(startup: &Startup) -> Result<()> {
    let fs = Initfs::open(startup)?;
    let image = fs.find("taskd").ok_or(Error::NotFound)?;
    let (process, ctl) = libos::spawn("taskd", image, Vec::new())?;
    let producer = one(command(&ctl, "producer system", Vec::new())?)?;
    let summary = one(command(&ctl, "client compd 1 0", Vec::new())?)?;
    summary.write(&Request::Subscribe { req: 1, since: None }.encode(), Vec::new())?;
    *TASKS.lock() = Some(Tasks {
        process,
        ctl,
        producer,
        summary,
        list: TaskList::default(),
        services: BTreeMap::new(),
        pending: BTreeMap::new(),
        next_req: 1,
        icon: None,
    });
    println!("init: sluzba uloh (taskd) bezi");
    Ok(())
}

/// Kanal klienta s identitou `identity`; `all` = i cizi a systemove ulohy,
/// `control` = smi ridit.
pub fn client(identity: &str, all: bool, control: bool) -> Result<Channel> {
    let g = TASKS.lock();
    let t = g.as_ref().ok_or(Error::NotFound)?;
    one(command(&t.ctl, &format!("client {identity} {} {}", all as u8, control as u8), Vec::new())?)
}

/// Kanal producenta s identitou `identity`.
pub fn producer(identity: &str) -> Result<Channel> {
    let g = TASKS.lock();
    let t = g.as_ref().ok_or(Error::NotFound)?;
    one(command(&t.ctl, &format!("producer {identity}"), Vec::new())?)
}

/// Sledovat procesy jobu (aplikace: neznamy proces).
pub fn watch(job: &Job) -> Result<()> {
    let g = TASKS.lock();
    let t = g.as_ref().ok_or(Error::NotFound)?;
    let h = job.duplicate(Some(Rights::BASIC | Rights::READ))?;
    command(&t.ctl, "watch", alloc::vec![h.into_handle()]).map(|_| ())
}

impl Tasks {
    fn produce(&self, m: &Produce) {
        let _ = self.producer.write(&m.encode(), Vec::new());
    }

    /// Stav sluzby do ulohy.
    fn service(&mut self, name: &'static str, state: State, reason: Option<&'static str>, last_exit: Option<i64>) {
        let known = self.services.get(&name).copied();
        let fresh = match known {
            None => true,
            // Uloha uz skoncila (vzdano, zastaveno) a sluzba zase bezi
            // (revive): nova uloha.
            Some((_, old)) => matches!(old, State::GaveUp | State::Stopped) && state == State::Running,
        };
        if fresh {
            let req = self.next_req;
            self.next_req += 1;
            self.pending.insert(req, name);
            self.services.insert(name, (None, state));
            self.produce(&Produce::Register {
                req,
                parent: 0,
                name: format!("Služba {name}"),
                origin: Origin::Service,
                who: String::from("init"),
                agent: false,
                job: None,
                attach: None,
                actions: 0,
            });
            return;
        }
        let Some((Some(id), old)) = known else {
            // Stav si zapamatovat; uloha se aktualizuje po REGISTERED.
            if let Some(e) = self.services.get_mut(&name) {
                e.1 = state;
            }
            return;
        };
        if old == state {
            return;
        }
        self.services.insert(name, (Some(id), state));
        self.send_state(id, state, reason, last_exit);
    }

    fn send_state(&self, id: u64, state: State, reason: Option<&'static str>, last_exit: Option<i64>) {
        let why = String::from(reason.unwrap_or(""));
        let m = match state {
            State::Running => Produce::Update { id, state: TaskState::Running, progress: None, eta_ms: None },
            State::Restarting => {
                Produce::Update { id, state: TaskState::Waiting(format!("restart {why}")), progress: None, eta_ms: None }
            }
            State::GaveUp => Produce::Finish {
                id,
                state: TaskState::Failed { code: last_exit.unwrap_or(0) as i32, message: format!("supervisor to vzdal {why}") },
            },
            State::Stopped => Produce::Finish {
                id,
                state: TaskState::Failed {
                    code: last_exit.unwrap_or(0) as i32,
                    message: format!("sluzba skoncila: {}", libos::abi::process::describe_exit(last_exit.unwrap_or(0))),
                },
            },
        };
        self.produce(&m);
    }

    /// Odpovedi taskd producentovi (REGISTERED) a zmeny pro ikonu.
    fn pump(&mut self) {
        while let Ok(m) = self.producer.try_read() {
            if let Ok((req, id)) = taskproto::decode_registered(&m.data)
                && let Some(name) = self.pending.remove(&req)
                && let Some(e) = self.services.get_mut(name)
            {
                e.0 = Some(id);
                let st = e.1;
                if st != State::Running {
                    self.send_state(id, st, None, None);
                }
            }
        }
        while let Ok(m) = self.summary.try_read() {
            if let Ok(r) = Reply::decode(&m.data)
                && matches!(self.list.apply(&r), taskproto::Applied::Gap)
            {
                let since = self.list.seq;
                let _ = self.summary.write(&Request::Subscribe { req: 2, since: Some(since) }.encode(), Vec::new());
            }
        }
    }
}

/// Po kole supervisoru: stav sluzeb do taskd, pocet uloh do ikony.
pub fn sync(sup: &Supervisor) {
    let mut g = TASKS.lock();
    let Some(t) = g.as_mut() else { return };
    t.pump();
    for s in sup.status() {
        t.service(s.name, s.state, s.reason, s.last_exit);
    }
    let now = t.list.running();
    if t.icon != Some(now) {
        t.icon = Some(now);
        if let Some(gui) = crate::gui::GUI.lock().as_ref() {
            let _ = gui.command(format!("tasks {} {}", now.0, now.1).as_bytes(), Vec::new());
        }
    }
}

/// Aplikace spustena initem (okno): uloha s jejim jobem (procesy v nem
/// k ni patri, cas CPU a pamet) a job sledovat (co v nem pribude bez
/// registrace, je neznamy proces).
pub fn app(name: &str, who: &str, job: &Job) {
    let job_id = job.info().map(|i| i.id).ok();
    {
        let mut g = TASKS.lock();
        let Some(t) = g.as_mut() else { return };
        let req = t.next_req;
        t.next_req += 1;
        t.produce(&Produce::Register {
            req,
            parent: 0,
            name: format!("Aplikace {name}"),
            origin: Origin::App,
            who: String::from(who),
            agent: false,
            job: job_id,
            attach: None,
            actions: 0,
        });
    }
    if let Err(e) = watch(job) {
        println!("init: job aplikace {name} pro prehled uloh: {e}");
    }
}

/// Preda sluzbe kanal producenta zpravou `tasks` na jejim ridicim kanalu
/// (odpoved `ok`).
pub fn give_producer(ctl: &Channel, service: &str) -> Result<()> {
    let p = producer("system")?;
    ctl.write(b"tasks", alloc::vec![p.into_handle()])?;
    let m = ctl.read(Some(time::deadline_after(REPLY_TIMEOUT)))?;
    if m.data.starts_with(b"ok") {
        Ok(())
    } else {
        println!("init: {service} odmitl kanal prehledu uloh");
        Err(Error::NotSupported)
    }
}
