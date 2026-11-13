//! files-copy: kopirovani a presun souboru jako ulohy relace uzivatele
//! (pomocny proces spravce souboru `gui-files`).
//!
//! Spousti ho init pri otevreni relace (job relace). Startovni handly:
//! 1. relace FS uzivatele, 2. ridici kanal od initu, 3. volitelne kanal
//! producenta na taskd; pracovni adresar v bloku parametru = domov
//! uzivatele. Init po ridicim kanalu posila pro kazde okno spravce
//! `client` s jednim handlem (kanal protokolu `fileops::proto`). Stroj kopirovani je `fileops::job`; stav uloh se
//! uklada do domova a po pristim startu relace ulohy pokracuji.

#![no_std]
#![no_main]

extern crate alloc;

mod store;
mod tasks;
mod vfs;

use alloc::string::String;
use alloc::vec::Vec;

use fileops::job::{Job, Phase, Step};
use fileops::proto::{ClientMsg, ServerMsg, Status};
use fsproto::FsClient;
use libos::{Channel, Error, Handle, Signals, Startup, println, time};

use store::Store;
use tasks::Tasks;
use vfs::FsVfs;

libos::entry!(main);

/// Cas prace mezi kontrolami zprav (ns).
const SLICE_NS: u64 = 8_000_000;
/// Jak casto nejvys posilat prubeh (ns).
const REPORT_NS: u64 = 250_000_000;

struct Service {
    fs: FsClient,
    store: Option<Store>,
    jobs: Vec<Job>,
    clients: Vec<Channel>,
    tasks: Tasks,
    next_id: u64,
    last_report: u64,
}

impl Service {
    fn send_all(&mut self, m: &ServerMsg) {
        let data = m.encode();
        self.clients.retain(|c| c.write(&data, Vec::new()).is_ok());
    }

    /// Stav ulohy klientum a do prehledu uloh.
    fn report(&mut self, i: usize) {
        let st = Status::of(&self.jobs[i]);
        self.tasks.update(&self.jobs[i]);
        self.send_all(&ServerMsg::Status(st));
    }

    fn save(&mut self, i: usize) {
        let Some(store) = &self.store else { return };
        let j = &self.jobs[i];
        let r = if matches!(j.phase, Phase::Done | Phase::Canceled) {
            store.remove(&self.fs, j.id);
            Ok(())
        } else {
            store.save(&self.fs, j)
        };
        if let Err(e) = r {
            println!("files-copy: stav ulohy {} nejde ulozit: {e}", j.id);
        }
        self.jobs[i].saved();
    }

    fn client_msg(&mut self, from: usize, data: &[u8]) {
        let m = match ClientMsg::decode(data) {
            Ok(m) => m,
            Err(e) => {
                println!("files-copy: neplatna zprava: {e}");
                return;
            }
        };
        match m {
            ClientMsg::Hello { .. } => {}
            ClientMsg::Subscribe => {
                let all: Vec<ServerMsg> = self.jobs.iter().map(|j| ServerMsg::Status(Status::of(j))).collect();
                for s in all {
                    let _ = self.clients[from].write(&s.encode(), Vec::new());
                }
            }
            ClientMsg::Start { req, op, sources, dest } => {
                if self.store.is_none() {
                    let _ = self.clients[from].write(&ServerMsg::Rejected { req, error: Error::BadState }.encode(), Vec::new());
                    return;
                }
                let id = self.next_id;
                self.next_id += 1;
                self.jobs.push(Job::new(id, op, sources, dest));
                let i = self.jobs.len() - 1;
                let _ = self.clients[from].write(&ServerMsg::Started { req, job: id }.encode(), Vec::new());
                self.tasks.register(&self.jobs[i]);
                self.save(i);
                self.report(i);
            }
            ClientMsg::Decide { job, resolution, all } => self.with_job(job, |j, fs| j.decide(&mut FsVfs(fs), resolution, all)),
            ClientMsg::Cancel { job } => self.with_job(job, |j, fs| j.cancel(&mut FsVfs(fs))),
            ClientMsg::Pause { job } => self.with_job(job, |j, _| j.pause()),
            ClientMsg::Resume { job } => self.with_job(job, |j, _| j.resume()),
        }
    }

    fn with_job(&mut self, id: u64, f: impl FnOnce(&mut Job, &FsClient)) {
        let Some(i) = self.jobs.iter().position(|j| j.id == id) else { return };
        f(&mut self.jobs[i], &self.fs);
        self.save(i);
        self.report(i);
        self.reap();
    }

    /// Hotove a zrusene ulohy pryc (posledni stav uz klienti dostali).
    fn reap(&mut self) {
        let before = self.jobs.len();
        self.jobs.retain(|j| !matches!(j.phase, Phase::Done | Phase::Canceled));
        if self.jobs.len() != before {
            self.tasks.forget(&self.jobs);
        }
    }

    fn busy(&self) -> bool {
        self.jobs.iter().any(|j| j.phase == Phase::Running)
    }

    /// Prace na bezicich ulohach po dobu `SLICE_NS` (stridave).
    fn work(&mut self) {
        let end = time::now() + SLICE_NS;
        let mut changed: Vec<usize> = Vec::new();
        while time::now() < end && self.busy() {
            for i in 0..self.jobs.len() {
                if self.jobs[i].phase != Phase::Running {
                    continue;
                }
                let step = self.jobs[i].step(&mut FsVfs(&self.fs));
                if !matches!(step, Step::Worked) && !changed.contains(&i) {
                    changed.push(i);
                }
                if self.jobs[i].should_save() {
                    self.save(i);
                }
            }
        }
        let due = time::now().saturating_sub(self.last_report) >= REPORT_NS;
        for i in 0..self.jobs.len() {
            if changed.contains(&i) || (due && self.jobs[i].phase == Phase::Running) {
                self.save(i);
                self.report(i);
            }
        }
        if due {
            self.last_report = time::now();
        }
        self.reap();
    }

    /// Nacte ulozene ulohy z domova `home` a pokracuje v nich.
    fn start(&mut self, home: &str) {
        {
            let s = Store::new(home);
            if let Err(e) = s.ensure(&self.fs) {
                println!("files-copy: slozka stavu {}: {e} (ulohy se neulozi)", s.dir);
            }
            let mut loaded = s.load(&self.fs);
            for j in &mut loaded {
                j.resumed(&mut FsVfs(&self.fs));
                self.next_id = self.next_id.max(j.id + 1);
            }
            if !loaded.is_empty() {
                println!("files-copy: pokracuje {} nedokoncenych uloh", loaded.len());
            }
            self.store = Some(s);
            for j in loaded {
                self.jobs.push(j);
                let i = self.jobs.len() - 1;
                self.tasks.register(&self.jobs[i]);
            }
        }
    }

    fn control(&mut self, text: &str, handles: Vec<Handle>) {
        if text == "client" {
            for h in handles {
                self.clients.push(Channel::from_handle(h));
            }
        }
    }
}

fn main(startup: Startup) -> i64 {
    let home = startup.params.as_ref().and_then(|v| libos::params::Params::read(v).ok()).map(|p| String::from_utf8_lossy(&p.cwd).into_owned()).unwrap_or_default();
    let mut hs = startup.handles.into_iter();
    let (Some(fs), Some(control)) = (hs.next(), hs.next()) else {
        println!("files-copy: chybi relace FS nebo ridici kanal");
        return 1;
    };
    let control = Channel::from_handle(control);
    let producer = hs.next().map(Channel::from_handle);
    let mut s = Service {
        fs: FsClient::new(Channel::from_handle(fs)),
        store: None,
        jobs: Vec::new(),
        clients: Vec::new(),
        tasks: Tasks::new(producer),
        next_id: 1,
        last_report: 0,
    };
    if home.starts_with("/home/") {
        s.start(&home);
    } else {
        println!("files-copy: chybi domov uzivatele, ulohy se neulozi");
    }
    let rd = Signals::READABLE | Signals::PEER_CLOSED;
    loop {
        let deadline = if s.busy() { Some(0) } else { None };
        let observed = {
            let mut items: Vec<(&Handle, Signals)> = alloc::vec![(control.handle(), rd)];
            for c in &s.clients {
                items.push((c.handle(), rd));
            }
            if let Some(p) = s.tasks.chan() {
                items.push((p.handle(), rd));
            }
            match libos::wait_many(&items, deadline) {
                Ok((_, o)) => o,
                Err(Error::TimedOut) => Vec::new(),
                Err(e) => {
                    println!("files-copy: cekani: {e}");
                    Vec::new()
                }
            }
        };
        if observed.first().is_some_and(|o| o.intersects(rd)) {
            loop {
                match control.try_read() {
                    Ok(m) => {
                        let text = String::from_utf8_lossy(&m.data).into_owned();
                        s.control(&text, m.handles);
                    }
                    Err(Error::ShouldWait) => break,
                    Err(_) => {
                        // Init zavrel kanal (konec relace): stav uloh je ulozeny.
                        for i in 0..s.jobs.len() {
                            s.save(i);
                        }
                        println!("files-copy: konec relace, koncim");
                        return 0;
                    }
                }
            }
        }
        let n = s.clients.len();
        let mut dead = Vec::new();
        for i in 0..n {
            if !observed.get(i + 1).is_some_and(|o| o.intersects(rd)) {
                continue;
            }
            loop {
                match s.clients[i].try_read() {
                    Ok(m) => s.client_msg(i, &m.data),
                    Err(Error::ShouldWait) => break,
                    Err(_) => {
                        dead.push(i);
                        break;
                    }
                }
            }
        }
        for i in dead.into_iter().rev() {
            s.clients.remove(i);
        }
        if observed.get(n + 1).is_some_and(|o| o.intersects(rd)) {
            for c in s.tasks.controls() {
                let (id, f): (u64, fn(&mut Job, &FsClient)) = match c {
                    taskproto::Control::Cancel(t) => (t, |j, fs| j.cancel(&mut FsVfs(fs))),
                    taskproto::Control::Pause(t) => (t, |j, _| j.pause()),
                    taskproto::Control::Resume(t) => (t, |j, _| j.resume()),
                };
                if let Some(job) = s.tasks.job_of(id) {
                    s.with_job(job, f);
                }
            }
        }
        if s.busy() {
            s.work();
        }
    }
}
