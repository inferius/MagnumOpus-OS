//! notifyd: sluzba oznameni (`docs/cs/oznameni.md`). Logika je v knihovne
//! `notifycore` (testy na hostiteli); tady jsou kanaly a cas.
//!
//! Bootstrap kanal = ridici kanal od initu. Jen init razi kanaly
//! s identitou (text, odpoved `ok` / `err <KOD>` jako u taskd):
//!
//! | Prikaz | Odpoved |
//! |---|---|
//! | `sender <uid> <system 0/1> <app_id> <jmeno...>` | `ok` + kanal odesilatele (`notifyproto::Send`) |
//! | `viewer <uid>` | `ok` + kanal prohlizece (`notifyproto::View`) |
//! | `status` | `ok count=N` |
//!
//! Stav je jen v pameti: pad sluzby oznameni ztrati (aplikace dostanou
//! zavreny kanal a pri pristim oznameni novy pres init).

#![no_std]
#![no_main]

extern crate alloc;

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use libos::{Channel, Error, Handle, Signals, Startup, println, time};
use notifycore::{Core, Identity, Out, SenderId, ViewerId};

libos::entry!(main);

enum Conn {
    Sender(SenderId),
    Viewer(ViewerId),
}

struct Notifyd {
    core: Core,
    conns: Vec<(Conn, Channel)>,
    dead: Vec<usize>,
}

/// Realny cas (ns od 1970).
fn wall() -> u64 {
    time::wall().map(|t| (t.secs.max(0) as u64).saturating_mul(1_000_000_000).saturating_add(t.nanos as u64)).unwrap_or(0)
}

impl Notifyd {
    fn send(&mut self, out: Vec<Out>) {
        for o in out {
            let (sender, id, data) = match o {
                Out::Sender(s, d) => (true, s, d),
                Out::Viewer(v, d) => (false, v, d),
            };
            let Some(i) = self.conns.iter().position(|(c, _)| match c {
                Conn::Sender(x) => sender && *x == id,
                Conn::Viewer(x) => !sender && *x == id,
            }) else {
                continue;
            };
            // Plna fronta prohlizece (zaseknuty kompozitor): zprava se zahodi,
            // sluzba necheka; mrtvy kanal pryc.
            if let Err(e) = self.conns[i].1.write(&data, Vec::new())
                && e != Error::ShouldWait
            {
                self.dead.push(i);
            }
        }
    }

    fn reap(&mut self) {
        let mut dead = core::mem::take(&mut self.dead);
        dead.sort_unstable();
        dead.dedup();
        for i in dead.into_iter().rev() {
            match self.conns.remove(i).0 {
                Conn::Sender(s) => self.core.remove_sender(s),
                Conn::Viewer(v) => self.core.remove_viewer(v),
            }
        }
    }

    fn control(&mut self, text: &str) -> (String, Vec<Handle>) {
        let mut w = text.splitn(5, ' ');
        let cmd = w.next().unwrap_or("");
        let new = |d: &mut Self, c: Conn, ours: Channel| d.conns.push((c, ours));
        match cmd {
            "sender" => {
                let (Some(uid), Some(sys), Some(app_id)) = (w.next().and_then(|u| u.parse().ok()), w.next(), w.next()) else {
                    return ("err INVALID_ARGS".into(), Vec::new());
                };
                let name = w.next().unwrap_or(app_id);
                let Ok((ours, theirs)) = Channel::create() else { return ("err NO_RESOURCES".into(), Vec::new()) };
                let id = self.core.add_sender(Identity { app_id: app_id.into(), app_name: name.into(), uid, system: sys == "1" });
                new(self, Conn::Sender(id), ours);
                ("ok".into(), alloc::vec![theirs.into_handle()])
            }
            "viewer" => {
                let Some(uid) = w.next().and_then(|u| u.parse().ok()) else { return ("err INVALID_ARGS".into(), Vec::new()) };
                let Ok((ours, theirs)) = Channel::create() else { return ("err NO_RESOURCES".into(), Vec::new()) };
                let id = self.core.add_viewer(uid);
                new(self, Conn::Viewer(id), ours);
                ("ok".into(), alloc::vec![theirs.into_handle()])
            }
            "status" => (format!("ok count={}", self.core.len()), Vec::new()),
            _ => ("err INVALID_ARGS".into(), Vec::new()),
        }
    }
}

fn main(startup: Startup) -> i64 {
    let control = startup.bootstrap;
    let mut d = Notifyd { core: Core::new(), conns: Vec::new(), dead: Vec::new() };
    println!("notifyd: bezim");
    // Promenny pocet kanalu: cekani pres port (`WaitSet`).
    let mut waits = match libos::WaitSet::new() {
        Ok(w) => w,
        Err(e) => {
            println!("notifyd: port nejde vytvorit: {e}");
            return 1;
        }
    };
    let mut next_tick: Option<u64> = None;
    loop {
        let rd = Signals::READABLE | Signals::PEER_CLOSED;
        let mut items: Vec<(&Handle, Signals)> = alloc::vec![(control.handle(), rd)];
        for (_, ch) in &d.conns {
            items.push((ch.handle(), rd));
        }
        // Konec Nerusit (realny cas) -> termin monotonnich hodin.
        let deadline = next_tick.map(|t| time::deadline_after(core::time::Duration::from_nanos(t.saturating_sub(wall()))));
        let observed = waits.wait(&items, deadline).map(|(_, o)| o).unwrap_or_default();
        drop(items);
        if observed.first().is_some_and(|s| s.intersects(rd)) {
            loop {
                match control.try_read() {
                    Ok(m) => {
                        let text = String::from_utf8_lossy(&m.data).into_owned();
                        let (reply, hs) = d.control(&text);
                        let _ = control.write(reply.as_bytes(), hs);
                    }
                    Err(Error::ShouldWait) => break,
                    Err(_) => {
                        println!("notifyd: init zavrel ridici kanal, koncim");
                        return 0;
                    }
                }
            }
        }
        let n = d.conns.len().min(observed.len().saturating_sub(1));
        for i in 0..n {
            if !observed[i + 1].intersects(rd) {
                continue;
            }
            loop {
                match d.conns[i].1.try_read() {
                    Ok(m) => {
                        let out = match d.conns[i].0 {
                            Conn::Sender(s) => d.core.sender_msg(s, &m.data, wall(), time::now()),
                            Conn::Viewer(v) => d.core.viewer_msg(v, &m.data, wall()),
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
        let (out, next) = d.core.tick(wall());
        d.send(out);
        d.reap();
        next_tick = next;
    }
}
