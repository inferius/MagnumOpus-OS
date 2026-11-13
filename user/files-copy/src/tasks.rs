//! Ulohy kopirovani v prehledu uloh (`taskd`, kanal producenta od initu):
//! registrace, prubeh, konec; zruseni, pozastaveni a pokracovani ze
//! Spravce uloh nebo prikazu `tasks`.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use fileops::job::{Job, Op, Phase};
use libos::Channel;
use taskproto::{Control, Origin, Produce, State, actions};

struct Slot {
    job: u64,
    req: u32,
    task: Option<u64>,
    /// Konec driv, nez taskd potvrdil registraci.
    early: Option<State>,
}

pub struct Tasks {
    chan: Option<Channel>,
    slots: Vec<Slot>,
    next_req: u32,
}

fn state_of(j: &Job) -> State {
    match j.phase {
        Phase::Running => State::Running,
        Phase::Paused => State::Paused,
        Phase::Conflict => State::Waiting(format!("Cíl už existuje: {}", j.conflict().map_or("", |c| fileops::path::name(&c.target)))),
        Phase::Done if j.error_count > 0 => State::Failed { code: libos::Error::IoError.code(), message: format!("Hotovo, chyby: {}", j.error_count) },
        Phase::Done => State::Done(format!("Hotovo: {}", j.done_items)),
        Phase::Canceled => State::Cancelled,
    }
}

impl Tasks {
    pub fn new(chan: Option<Channel>) -> Self {
        Self { chan, slots: Vec::new(), next_req: 0 }
    }

    pub fn chan(&self) -> Option<&Channel> {
        self.chan.as_ref()
    }

    fn send(&mut self, m: &Produce) {
        if let Some(c) = &self.chan
            && c.write(&m.encode(), Vec::new()).is_err()
        {
            self.chan = None;
        }
    }

    pub fn register(&mut self, j: &Job) {
        if self.chan.is_none() {
            return;
        }
        self.next_req = self.next_req.wrapping_add(1).max(1);
        let req = self.next_req;
        let what = if j.op == Op::Copy { "Kopírování" } else { "Přesun" };
        let n = j.total_items();
        let name = format!("{what} {n} {} do {}", if n == 1 { "položky" } else { "položek" }, fileops::path::name(&j.dest));
        self.send(&Produce::Register {
            req,
            parent: 0,
            name,
            origin: Origin::App,
            who: String::from("Soubory"),
            agent: false,
            job: None,
            attach: None,
            actions: actions::CANCEL | actions::PAUSE | actions::RESUME,
        });
        self.slots.push(Slot { job: j.id, req, task: None, early: None });
    }

    pub fn update(&mut self, j: &Job) {
        let Some(i) = self.slots.iter().position(|s| s.job == j.id) else { return };
        let state = state_of(j);
        let done = matches!(j.phase, Phase::Done | Phase::Canceled);
        match self.slots[i].task {
            None if done => self.slots[i].early = Some(state),
            None => {}
            Some(id) if done => self.send(&Produce::Finish { id, state }),
            Some(id) => self.send(&Produce::Update { id, state, progress: Some(j.percent()), eta_ms: None }),
        }
    }

    /// Ulohy, ktere uz nejsou (hotove po odeslani konce).
    pub fn forget(&mut self, jobs: &[Job]) {
        self.slots.retain(|s| s.early.is_some() || jobs.iter().any(|j| j.id == s.job));
    }

    pub fn job_of(&self, task: u64) -> Option<u64> {
        self.slots.iter().find(|s| s.task == Some(task)).map(|s| s.job)
    }

    /// Precte zpravy taskd: potvrzeni registrace a pozadavky na rizeni.
    pub fn controls(&mut self) -> Vec<Control> {
        let mut out = Vec::new();
        loop {
            let Some(c) = &self.chan else { break };
            match c.try_read() {
                Ok(m) => {
                    if let Ok((req, id)) = taskproto::decode_registered(&m.data) {
                        if let Some(i) = self.slots.iter().position(|s| s.req == req) {
                            self.slots[i].task = Some(id);
                            if let Some(state) = self.slots[i].early.take() {
                                self.send(&Produce::Finish { id, state });
                                self.slots.remove(i);
                            }
                        }
                    } else if let Ok(c) = Control::decode(&m.data) {
                        out.push(c);
                    }
                }
                Err(libos::Error::ShouldWait) => break,
                Err(_) => {
                    self.chan = None;
                    break;
                }
            }
        }
        out
    }
}
