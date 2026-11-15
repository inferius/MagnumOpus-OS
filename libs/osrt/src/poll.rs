//! Cekani na vice cisel fd (`poll`, `select`, `epoll` v cstd).
//!
//! Soubory jsou vzdy pripravene; stdin a roury cekaji na signal kanalu
//! (`READABLE`, `PEER_CLOSED`), sockety na event pripravenosti netd
//! (`netproto::ready`). Zapis do kanalu nema signal "lze zapisovat"
//! (zapis pri plne fronte kratce pocka), proto je stdout a zapisovaci konec
//! roury hlasen jako pripraveny vzdy. Az 64 objektu = `OBJECT_WAIT_MANY`,
//! vic = port s jednorazovou registraci.

use alloc::sync::Arc;
use alloc::vec::Vec;
use core::time::Duration;

use abi::Signals;
use libos::{Handle, Port};
use netproto::{Readiness, ready};

use crate::Result;
use crate::fd::{self, Description, Object};
use crate::stdio::{self, Stream};

/// Udalosti (hodnoty jako Linux `POLL*`).
pub mod ev {
    pub const IN: u16 = 0x001;
    pub const PRI: u16 = 0x002;
    pub const OUT: u16 = 0x004;
    pub const ERR: u16 = 0x008;
    pub const HUP: u16 = 0x010;
    pub const NVAL: u16 = 0x020;
}

/// Polozka `poll` (jako `struct pollfd`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PollFd {
    pub fd: i32,
    pub events: u16,
    pub revents: u16,
}

/// Na co se u jednoho fd ceka.
enum Source {
    /// Pripraveno hned (`revents` znamy).
    Now(u16),
    /// Kanal (stdin, cteci konec roury); `extra` = vzdy hlasene bity.
    Channel(Held, u16),
    /// Event socketu.
    Socket(Arc<Readiness>),
    /// Kanal socketu `AF_UNIX` (duplikat handlu na dobu cekani).
    Unix(Handle, u16),
}

/// Popis drzeny po dobu cekani (handle kanalu zije s nim).
struct Held(Arc<Description>);

impl Held {
    fn handle(&self) -> Option<&Handle> {
        match &self.0.object {
            Object::Pipe(p) => Some(p.handle()),
            Object::Stdio(s) => stdio::handle(*s),
            _ => None,
        }
    }
}

fn source(p: &PollFd) -> Result<Source> {
    let Ok(d) = fd::get(p.fd) else { return Ok(Source::Now(ev::NVAL)) };
    let want = p.events | ev::ERR | ev::HUP;
    let st = d.status() & fd::status::ACCMODE;
    let can_read = st != fd::status::WRONLY;
    let can_write = st != fd::status::RDONLY;
    Ok(match &d.object {
        Object::File(_) | Object::Memory(_) => Source::Now(want & (ev::IN | ev::OUT)),
        Object::User(_) | Object::Process(_) => Source::Now(0),
        // /dev/null je vzdy pripraveny (cteni hned konec).
        Object::Null => Source::Now(want & (ev::IN | ev::OUT)),
        Object::Stdio(Stream::In) => {
            if stdio::stdin_buffered() || stdio::handle(Stream::In).is_none() {
                // Data ve zbytku zpravy, nebo stdin neni (hned konec).
                Source::Now(want & (ev::IN | ev::HUP))
            } else {
                Source::Channel(Held(d.clone()), 0)
            }
        }
        Object::Stdio(_) => Source::Now(want & ev::OUT),
        Object::Pipe(pipe) => {
            let out = if can_write { want & ev::OUT } else { 0 };
            if can_read && pipe.has_buffered() {
                Source::Now(out | (want & ev::IN))
            } else {
                Source::Channel(Held(d.clone()), out)
            }
        }
        Object::Unix(u) => match u.wait_handle() {
            Some((_, true)) => Source::Now(want & (ev::IN | ev::OUT)),
            Some((h, false)) => Source::Unix(h, if u.is_listening() { 0 } else { want & ev::OUT }),
            // Bez spojeni (po socket/bind): zapis by selhal.
            None => Source::Now(want & ev::OUT),
        },
        Object::Socket(s) => match s.readiness()? {
            Some(r) => Source::Socket(r),
            // Socket bez spojeni: jako Linux (zapis by selhal, konec).
            None => Source::Now(want & (ev::OUT | ev::HUP)),
        },
    })
}

/// Maska signalu objektu pro pozadovane udalosti.
fn mask(src: &Source, events: u16) -> Signals {
    match src {
        Source::Channel(..) | Source::Unix(..) => Signals(Signals::READABLE.0 | Signals::PEER_CLOSED.0),
        Source::Socket(_) => {
            let mut m = ready::HUP.0 | ready::ERROR.0;
            if events & ev::IN != 0 {
                m |= ready::READABLE.0;
            }
            if events & ev::OUT != 0 {
                m |= ready::WRITABLE.0;
            }
            Signals(m)
        }
        Source::Now(_) => Signals::NONE,
    }
}

/// Udalosti z pozorovanych signalu.
fn revents(src: &Source, events: u16, s: Signals) -> u16 {
    let want = events | ev::ERR | ev::HUP;
    match src {
        Source::Now(r) => *r,
        Source::Channel(_, extra) | Source::Unix(_, extra) => {
            let mut r = *extra;
            if s.0 & Signals::READABLE.0 != 0 {
                r |= ev::IN;
            }
            if s.0 & Signals::PEER_CLOSED.0 != 0 {
                // Konec proudu: cteni vrati 0 (POSIX hlasi IN i HUP).
                r |= ev::IN | ev::HUP;
            }
            r & want
        }
        Source::Socket(_) => {
            let mut r = 0;
            if s.0 & ready::READABLE.0 != 0 {
                r |= ev::IN;
            }
            if s.0 & ready::WRITABLE.0 != 0 {
                r |= ev::OUT;
            }
            if s.0 & ready::HUP.0 != 0 {
                r |= ev::HUP;
            }
            if s.0 & ready::ERROR.0 != 0 {
                r |= ev::ERR;
            }
            r & want
        }
    }
}

fn handle_of(src: &Source) -> Option<&Handle> {
    match src {
        Source::Channel(d, _) => d.handle(),
        Source::Socket(r) => Some(r.handle()),
        Source::Unix(h, _) => Some(h),
        Source::Now(_) => None,
    }
}

/// Signaly objektu prave ted.
fn current(h: &Handle, m: Signals) -> Signals {
    let (_, observed) = libos::sys::object_wait_one(h.raw(), m.0, 0);
    Signals(observed)
}

/// `poll`: vyplni `revents`, vraci pocet pripravenych; `timeout` `None`
/// = bez limitu, nula = jen zjistit.
pub fn poll(fds: &mut [PollFd], timeout: Option<Duration>) -> Result<usize> {
    let deadline = timeout.map(|t| libos::time::deadline_after(t));
    loop {
        let sources: Vec<Source> = fds.iter().map(source).collect::<Result<_>>()?;
        let mut ready_count = 0;
        for (p, src) in fds.iter_mut().zip(&sources) {
            let r = match handle_of(src) {
                Some(h) => revents(src, p.events, current(h, mask(src, p.events))),
                None => revents(src, p.events, Signals::NONE),
            };
            p.revents = r;
            if r != 0 {
                ready_count += 1;
            }
        }
        let expired = deadline.is_some_and(|d| libos::time::now() >= d);
        if ready_count > 0 || expired {
            return Ok(ready_count);
        }
        let waits: Vec<(&Handle, Signals)> = fds
            .iter()
            .zip(&sources)
            .filter_map(|(p, src)| handle_of(src).map(|h| (h, mask(src, p.events))))
            .collect();
        if waits.is_empty() {
            // Neni na co cekat: jen termin.
            match deadline {
                Some(d) => {
                    let _ = libos::sys::object_wait_many(&mut [], d);
                }
                None => return Ok(0),
            }
            continue;
        }
        if waits.len() <= abi::syscall::WAIT_MANY_MAX {
            // Vysledek je jedno: po probuzeni (nebo terminu) se vse prepocita.
            let _ = libos::wait_many(&waits, deadline);
        } else {
            let port = Port::create()?;
            for (i, (h, m)) in waits.iter().enumerate() {
                port.wait_async(h, i as u64, *m)?;
            }
            let _ = port.wait(deadline);
        }
    }
}
