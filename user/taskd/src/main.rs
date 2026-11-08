//! taskd: sluzba uloh - jeden seznam vseho, co bezi na pozadi (viz
//! `docs/cs/architektura.md`, "Prehled uloh"). Logika je v knihovne
//! `taskcore` (testy na hostiteli); tady jsou kanaly a vycet procesu.
//!
//! Bootstrap kanal = ridici kanal od initu. Jen init razi kanaly s
//! identitou a rozsahem (text, odpoved `ok` / `err <KOD>` jako u inputd):
//!
//! | Prikaz | Handly | Odpoved |
//! |---|---|---|
//! | `client <identita> <all 0/1> <control 0/1>` | - | `ok` + kanal klienta (`taskproto::Request`) |
//! | `producer <identita>` | - | `ok` + kanal producenta (`taskproto::Produce`) |
//! | `watch` | job (pravo `READ`) | `ok`: procesy jobu se sleduji (neznamy proces) |
//! | `status` | - | `ok running=N agent=M` |
//!
//! Kazdych [`POLL`] projde procesy sledovanych jobu (`JOB_PROCESSES`) a
//! pamet jobu (`JOB_INFO`) a uklidi stare hotove ulohy.

#![no_std]
#![no_main]

extern crate alloc;

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::time::Duration;

use libos::job::Job;
use libos::{Channel, Error, Handle, Signals, Startup, println, time};
use taskcore::{ClientId, Core, Out, ProducerId, Scope};

libos::entry!(main);

/// Vycet procesu a uklid hotovych uloh.
const POLL: Duration = Duration::from_secs(2);

enum Conn {
    Client(ClientId),
    Producer(ProducerId),
}

struct Taskd {
    core: Core,
    conns: Vec<(Conn, Channel)>,
    watched: Vec<Job>,
    /// Spojeni k odebrani po kole smycky (indexy se behem kola nemeni).
    dead: Vec<usize>,
}

fn now() -> u64 {
    time::wall().map(|t| (t.secs.max(0) as u64).saturating_mul(1_000_000_000).saturating_add(t.nanos as u64)).unwrap_or(0)
}

impl Taskd {
    fn send(&mut self, out: Vec<Out>) {
        let mut dead = Vec::new();
        for o in out {
            let (is_client, id, data) = match o {
                Out::Client(c, d) => (true, c, d),
                Out::Producer(p, d) => (false, p, d),
            };
            let Some(i) = self.conns.iter().position(|(c, _)| match c {
                Conn::Client(x) => is_client && *x == id,
                Conn::Producer(x) => !is_client && *x == id,
            }) else {
                continue;
            };
            if let Err(e) = self.conns[i].1.write(&data, Vec::new())
                && e != Error::ShouldWait
            {
                dead.push(i);
            }
        }
        self.dead.extend(dead);
    }

    /// Odebere mrtva spojeni (konec kola). Konec producenta ukonci jeho
    /// ulohy - to muze odhalit dalsi mrtva spojeni, proto opakovane.
    fn reap(&mut self) {
        while !self.dead.is_empty() {
            let mut dead = core::mem::take(&mut self.dead);
            dead.sort_unstable();
            dead.dedup();
            let mut out = Vec::new();
            for i in dead.into_iter().rev() {
                let (c, _) = self.conns.remove(i);
                match c {
                    Conn::Client(id) => self.core.remove_client(id),
                    Conn::Producer(id) => out.extend(self.core.remove_producer(id, now())),
                }
            }
            self.send(out);
        }
    }

    /// Ridici prikaz od initu.
    fn control(&mut self, text: &str, mut handles: Vec<Handle>) -> (String, Vec<Handle>) {
        let mut w = text.split_whitespace();
        match w.next().unwrap_or("") {
            "client" => {
                let (Some(identity), all, control) = (w.next(), w.next() == Some("1"), w.next() == Some("1")) else {
                    return ("err INVALID_ARGS".into(), Vec::new());
                };
                let Ok((ours, theirs)) = Channel::create() else { return ("err NO_RESOURCES".into(), Vec::new()) };
                let id = self.core.add_client(Scope { identity: identity.into(), all, control });
                self.conns.push((Conn::Client(id), ours));
                ("ok".into(), alloc::vec![theirs.into_handle()])
            }
            "producer" => {
                let Some(identity) = w.next() else { return ("err INVALID_ARGS".into(), Vec::new()) };
                let Ok((ours, theirs)) = Channel::create() else { return ("err NO_RESOURCES".into(), Vec::new()) };
                let id = self.core.add_producer(identity);
                self.conns.push((Conn::Producer(id), ours));
                ("ok".into(), alloc::vec![theirs.into_handle()])
            }
            "watch" => match handles.pop() {
                Some(h) => {
                    self.watched.push(Job::from_handle(h));
                    self.poll();
                    ("ok".into(), Vec::new())
                }
                None => ("err INVALID_ARGS".into(), Vec::new()),
            },
            "status" => {
                let (n, a) = self.core.running();
                (format!("ok running={n} agent={a}"), Vec::new())
            }
            _ => ("err INVALID_ARGS".into(), Vec::new()),
        }
    }

    /// Procesy a pamet sledovanych jobu, uklid starych hotovych uloh.
    fn poll(&mut self) {
        let mut procs = Vec::new();
        let mut memory = Vec::new();
        self.watched.retain(|j| match (j.processes(), j.info()) {
            (Ok(p), Ok(info)) => {
                procs.extend(p);
                memory.push((info.id, info.user.used));
                true
            }
            // Job zanikl nebo handle neplati: prestat sledovat.
            _ => false,
        });
        let t = now();
        let mut out = self.core.processes(&procs, &memory, t);
        out.extend(self.core.tick(t));
        self.send(out);
        self.reap();
    }
}

fn main(startup: Startup) -> i64 {
    let control = startup.bootstrap;
    let mut d = Taskd { core: Core::new(), conns: Vec::new(), watched: Vec::new(), dead: Vec::new() };
    println!("taskd: bezim");
    let mut next_poll = time::deadline_after(POLL);
    loop {
        let rd = Signals::READABLE | Signals::PEER_CLOSED;
        let mut items: Vec<(&Handle, Signals)> = alloc::vec![(control.handle(), rd)];
        for (_, ch) in &d.conns {
            items.push((ch.handle(), rd));
        }
        let observed = match libos::wait_many(&items, Some(next_poll)) {
            Ok((_, o)) => o,
            Err(_) => Vec::new(),
        };
        drop(items);
        // Ridici kanal.
        if observed.first().is_some_and(|s| s.intersects(rd)) {
            loop {
                match control.try_read() {
                    Ok(m) => {
                        let text = String::from_utf8_lossy(&m.data).into_owned();
                        let (reply, hs) = d.control(&text, m.handles);
                        let _ = control.write(reply.as_bytes(), hs);
                    }
                    Err(Error::ShouldWait) => break,
                    Err(_) => {
                        println!("taskd: init zavrel ridici kanal, koncim");
                        return 0;
                    }
                }
            }
        }
        // Klienti a producenti (indexy plati cele kolo, mrtve pryc na konci).
        let n = d.conns.len().min(observed.len().saturating_sub(1));
        for i in 0..n {
            if !observed[i + 1].intersects(rd) || d.dead.contains(&i) {
                continue;
            }
            loop {
                match d.conns[i].1.try_read() {
                    Ok(m) => {
                        let t = now();
                        let out = match d.conns[i].0 {
                            Conn::Client(c) => d.core.client_msg(c, &m.data, t),
                            Conn::Producer(p) => d.core.producer_msg(p, &m.data, t),
                        };
                        d.send(out);
                    }
                    Err(Error::ShouldWait) => break,
                    Err(_) => {
                        d.dead.push(i);
                        break;
                    }
                }
            }
        }
        d.reap();
        if time::now() >= next_poll {
            next_poll = time::deadline_after(POLL);
            d.poll();
        }
    }
}
