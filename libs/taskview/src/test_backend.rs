//! Testovaci zdroj uloh (dokud sluzba `taskd` neni v systemu): nekolik
//! uloh vcetne ulohy agenta AI a systemove ulohy bez opravneni. Akce se
//! chovaji jako u taskd: kontrola prav, zmena stavu udalosti s poradim.

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use taskproto::{Change, Error, Origin, Reply, Request, State, Task, actions};

use crate::Backend;

pub struct TestBackend {
    pub tasks: Vec<Task>,
    seq: u64,
}

fn task(id: u64, name: &str, origin: Origin, who: &str, state: State, progress: Option<u8>, acts: u32) -> Task {
    Task {
        id,
        parent: 0,
        name: name.into(),
        origin,
        who: who.into(),
        agent: origin == Origin::Agent,
        state,
        progress,
        eta_ms: None,
        memory: Some((id * 7 + 3) << 20),
        cpu_ns: Some(id * 120_000_000),
        started: 0,
        ended: None,
        attach: None,
        actions: acts,
    }
}

impl Default for TestBackend {
    fn default() -> Self {
        let all = actions::CANCEL | actions::PAUSE | actions::RESUME | actions::LOG;
        Self {
            tasks: vec![
                task(1, "Kopírování 3 souborů do /zaloha", Origin::User, "michal", State::Running, Some(40), all),
                task(2, "Refaktorace projektu (klon)", Origin::Agent, "asistent", State::Running, Some(15), all),
                task(3, "Scrub svazku data", Origin::Service, "fsd", State::Running, Some(12), 0),
                task(4, "build.sh", Origin::Script, "michal", State::Waiting(String::from("Přepsat výstup? [a/n]")), None, all),
                task(5, "Stažení aktualizací", Origin::App, "aktualizace", State::Done(String::from("3 balíky")), None, actions::LOG),
            ],
            seq: 1,
        }
    }
}

impl TestBackend {
    fn change(&mut self, id: u64, state: State) -> Reply {
        let t = self.tasks.iter_mut().find(|t| t.id == id).unwrap();
        t.state = state;
        self.seq += 1;
        Reply::Event { seq: self.seq, change: Change::Updated(t.clone()) }
    }
}

impl Backend for TestBackend {
    fn send(&mut self, req: Request) -> Vec<Reply> {
        let (req_id, id, bit) = match req {
            Request::Hello { .. } | Request::Unsubscribe { .. } => return Vec::new(),
            Request::Subscribe { req, .. } => {
                return vec![Reply::Snapshot { req, seq: self.seq, tasks: self.tasks.clone(), last: true }];
            }
            Request::Cancel { req, id } => (req, id, actions::CANCEL),
            Request::Pause { req, id } => (req, id, actions::PAUSE),
            Request::Resume { req, id } => (req, id, actions::RESUME),
            Request::Log { req, id, .. } => (req, id, actions::LOG),
        };
        let res = |r| Reply::Result { req: req_id, result: r };
        let Some(t) = self.tasks.iter().find(|t| t.id == id) else {
            return vec![res(Err(Error::NotFound))];
        };
        // Taskd rozhoduje sam: bez prava ACCESS_DENIED, ve spatnem stavu
        // INVALID_ARGS.
        if t.actions & bit == 0 {
            return vec![res(Err(Error::AccessDenied))];
        }
        if bit == actions::LOG {
            let lines = vec![alloc::format!("{}: start", t.name), alloc::format!("{}: pracuji", t.name)];
            return vec![Reply::LogLines { req: req_id, lines }, res(Ok(()))];
        }
        let next = match (bit, &t.state) {
            (actions::CANCEL, s) if !s.finished() => State::Cancelled,
            (actions::PAUSE, State::Running | State::Waiting(_)) => State::Paused,
            (actions::RESUME, State::Paused) => State::Running,
            _ => return vec![res(Err(Error::InvalidArgs))],
        };
        let ev = self.change(id, next);
        vec![ev, res(Ok(()))]
    }
}
