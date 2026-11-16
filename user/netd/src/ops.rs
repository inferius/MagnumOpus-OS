//! Dotazy na kanalu objektu (netproto v2): cislo dotazu, termin, zruseni
//! a fronty podle smeru.
//!
//! - Kazda zprava je novy dotaz (`Op`) s cislem z hlavicky; odpoved nese
//!   stejne cislo. Kanal se cte porad, rozpracovany dotaz dalsi neblokuje.
//! - Socket: v kazdem smeru ([`Dir`]) je rozpracovany nejvys jeden dotaz,
//!   dalsi cekaji ve fronte FIFO (nejvys [`netproto::DIRECTION_QUEUE`]
//!   vcetne rozpracovaneho), jinak `NO_RESOURCES`. Fronta drzi poradi dat
//!   (dva zapisy se neprolozi).
//! - Relace: dotazy bez fronty, nejvys `Quota::ops` najednou (`OpTicket`).
//!   Dalsi se odlozi ([`SESSION_BACKLOG`] zprav, FIFO) a spusti, az se
//!   misto uvolni; s plnym odkladem se kanal prestane cist (protitlak,
//!   zpravy cekaji v jadre na ucet odesilatele). `CANCEL` se dostane
//!   i k odlozenym dotazum.
//! - `CANCEL`: zrusi dotaz podle cisla (rozpracovany, ve fronte i odlozeny).

use alloc::vec::Vec;

use libos::{Channel, Error, Result};
use netproto::auth::OpTicket;
use netproto::msg::{self, Header};
use netproto::{DIRECTION_QUEUE, Writer, op};

use crate::obj::{self, Dir, Kind, Obj, Op, OpState, Step};
use crate::stack::{Stack, now_ns};

/// Kolik zprav precist z jednoho kanalu najednou (ferovost mezi klienty).
pub const READ_BATCH: usize = 32;
/// Kolik zprav relace s plnym limitem dotazu si sluzba odlozi, nez kanal
/// prestane cist.
pub const SESSION_BACKLOG: usize = 32;

/// Smi se cist dalsi zprava z kanalu objektu?
pub fn can_read(o: &Obj) -> bool {
    o.backlog.len() < SESSION_BACKLOG
}

/// Hlavicka odlozene zpravy.
fn header_of(data: &[u8]) -> Option<Header> {
    msg::parse_request(data).ok().map(|(h, _)| h)
}

/// Spusti odlozene dotazy relace, dokud je misto v limitu.
fn drain_backlog(stack: &mut Stack, o: &mut Obj, created: &mut Vec<Obj>) {
    while let Some(front) = o.backlog.front() {
        let Some(h) = header_of(front) else {
            o.backlog.pop_front();
            continue;
        };
        let ticket = match &*o.kind.borrow() {
            Kind::Session(auth) => match auth.borrow().begin_op() {
                Ok(t) => t,
                Err(_) => return,
            },
            _ => return,
        };
        let data = o.backlog.pop_front().unwrap();
        let m = Meta {
            id: h.id,
            dir: Dir::Free,
            ticket: Some(ticket),
        };
        start(stack, o, h, &data, m, created);
    }
}

/// Odpoved na dotaz `id` s pripadnym novym objektem (jeho kanal jde
/// klientovi v handlech).
fn reply(stack: &mut Stack, o: &Obj, id: u32, mut result: Result<Writer>, new: Option<Kind>, created: &mut Vec<Obj>) {
    let mut handles = Vec::new();
    if let Some(kind) = new {
        if result.is_err() {
            obj::close(stack, kind, Vec::new());
        } else {
            match Channel::create() {
                Ok((client, server)) => {
                    handles.push(client.into_handle());
                    created.push(Obj::new(server, kind));
                }
                Err(e) => {
                    obj::close(stack, kind, Vec::new());
                    result = Err(e);
                }
            }
        }
    }
    // Klient, ktery mezitim kanal zavrel, se uklidi po paketu "zavreno".
    let _ = o.ch.write(&msg::reply(id, result), handles);
}

/// Cislo, smer a misto v limitu relace jednoho dotazu.
struct Meta {
    id: u32,
    dir: Dir,
    ticket: Option<OpTicket>,
}

/// Krok dotazu: hotovo = odpoved, jinak se ulozi jako rozpracovany.
/// Vraci `true`, kdyz dotaz skoncil.
fn settle(stack: &mut Stack, o: &mut Obj, m: Meta, step: Step, created: &mut Vec<Obj>) -> bool {
    match step {
        Step::Wait(p) => {
            o.ops.push(Op {
                id: m.id,
                dir: m.dir,
                state: OpState::Active(p),
                _ticket: m.ticket,
            });
            false
        }
        Step::Done(r, new) => {
            reply(stack, o, m.id, r, new, created);
            true
        }
    }
}

/// Zacne dotaz ze zpravy `data` (hlavicka `h` uz overena).
fn start(stack: &mut Stack, o: &mut Obj, h: Header, data: &[u8], m: Meta, created: &mut Vec<Obj>) -> bool {
    let step = match msg::parse_request(data) {
        Ok((_, mut r)) => obj::request(stack, &mut o.kind.borrow_mut(), h.op, h.deadline, &mut r),
        Err(e) => Step::err(e),
    };
    settle(stack, o, m, step, created)
}

/// Spusti dalsi dotazy ve fronte smeru `dir`, dokud nektery nezustane
/// rozpracovany (nebo fronta nedojde).
fn start_queued(stack: &mut Stack, o: &mut Obj, dir: Dir, created: &mut Vec<Obj>) {
    if dir == Dir::Free {
        return;
    }
    loop {
        if o.ops.iter().any(|x| x.dir == dir && matches!(x.state, OpState::Active(_))) {
            return;
        }
        let Some(i) = o.ops.iter().position(|x| x.dir == dir) else {
            return;
        };
        let op = o.ops.remove(i);
        let OpState::Queued { msg: data, .. } = op.state else {
            unreachable!("rozpracovany dotaz vyse vyloucen");
        };
        let Ok((h, _)) = msg::parse_request(&data) else {
            continue;
        };
        let m = Meta {
            id: op.id,
            dir,
            ticket: op._ticket,
        };
        start(stack, o, h, &data, m, created);
    }
}

/// Zrusi dotaz `id` (rozpracovany i ve fronte); nezname cislo se ignoruje.
fn cancel(stack: &mut Stack, o: &mut Obj, id: u32, created: &mut Vec<Obj>) {
    if let Some(i) = o.backlog.iter().position(|d| header_of(d).is_some_and(|h| h.id == id)) {
        o.backlog.remove(i);
        reply(stack, o, id, Err(Error::Canceled), None, created);
        return;
    }
    let Some(i) = o.ops.iter().position(|x| x.id == id) else {
        return;
    };
    let op = o.ops.remove(i);
    let result = match op.state {
        OpState::Active(p) => obj::cancel_pending(stack, p),
        OpState::Queued { .. } => Err(Error::Canceled),
    };
    reply(stack, o, id, result, None, created);
    start_queued(stack, o, op.dir, created);
    drop(op._ticket);
    drain_backlog(stack, o, created);
}

/// `READINESS`: event pripravenosti socketu (zalozi se pri prvnim dotazu,
/// dalsi dotaz vrati dalsi duplikat stejneho eventu).
fn readiness(stack: &mut Stack, o: &mut Obj, id: u32) {
    if matches!(*o.kind.borrow(), Kind::Session(_)) {
        let _ = o.ch.write(&msg::reply(id, Err(Error::NotSupported)), Vec::new());
        return;
    }
    if o.ready.is_none() {
        match crate::ready::Readiness::new() {
            Ok(r) => o.ready = Some(r),
            Err(e) => {
                let _ = o.ch.write(&msg::reply(id, Err(e)), Vec::new());
                return;
            }
        }
    }
    crate::ready::update(stack, o);
    let r = o.ready.as_ref().map(|r| r.client_handle());
    match r {
        Some(Ok(h)) => {
            let _ = o.ch.write(&msg::reply(id, Ok(Writer::new())), alloc::vec![h]);
        }
        Some(Err(e)) => {
            let _ = o.ch.write(&msg::reply(id, Err(e)), Vec::new());
        }
        None => {}
    }
}

/// `DUP`: dalsi kanal ke stejnemu socketu (relace ne: uzsi relaci dava
/// `RESTRICT`).
fn dup(o: &mut Obj, id: u32, created: &mut Vec<Obj>) {
    if matches!(*o.kind.borrow(), Kind::Session(_)) {
        let _ = o.ch.write(&msg::reply(id, Err(Error::NotSupported)), Vec::new());
        return;
    }
    match Channel::create() {
        Ok((client, server)) => {
            created.push(Obj::shared(server, o.kind.clone()));
            let _ = o.ch.write(&msg::reply(id, Ok(Writer::new())), alloc::vec![client.into_handle()]);
        }
        Err(e) => {
            let _ = o.ch.write(&msg::reply(id, Err(e)), Vec::new());
        }
    }
}

/// Nova zprava od klienta.
pub fn on_message(stack: &mut Stack, o: &mut Obj, data: Vec<u8>, created: &mut Vec<Obj>) {
    // Zprava bez platne hlavicky nema cislo, na ktere by slo odpovedet:
    // zahodit (klient ji nikdy neposle spravne).
    let Ok((h, _)) = msg::parse_request(&data) else {
        return;
    };
    if h.op == op::CANCEL {
        cancel(stack, o, h.id, created);
        return;
    }
    if h.op == op::READINESS {
        readiness(stack, o, h.id);
        return;
    }
    if h.op == op::DUP {
        dup(o, h.id, created);
        return;
    }
    let backlogged = o.backlog.iter().any(|d| header_of(d).is_some_and(|x| x.id == h.id));
    if backlogged || o.ops.iter().any(|x| x.id == h.id) {
        // Cislo uz patri rozpracovanemu dotazu: odpoved by se nedala
        // rozlisit.
        reply(stack, o, h.id, Err(Error::AlreadyExists), None, created);
        return;
    }
    let dir = obj::dir_of(&o.kind.borrow(), h.op);
    let ticket = match &*o.kind.borrow() {
        // Za odlozenymi dotazy (poradi), nebo odlozit pri plnem limitu.
        Kind::Session(_) if !o.backlog.is_empty() => None,
        Kind::Session(auth) => auth.borrow().begin_op().ok(),
        _ => None,
    };
    if matches!(*o.kind.borrow(), Kind::Session(_)) && ticket.is_none() {
        if o.backlog.len() < SESSION_BACKLOG {
            o.backlog.push_back(data);
        } else {
            // Sem se nedojde: s plnym odkladem se kanal necte.
            reply(stack, o, h.id, Err(Error::NoResources), None, created);
        }
        return;
    }
    if dir != Dir::Free {
        let same = o.ops.iter().filter(|x| x.dir == dir).count();
        if same >= DIRECTION_QUEUE {
            reply(stack, o, h.id, Err(Error::NoResources), None, created);
            return;
        }
        if same > 0 {
            o.ops.push(Op {
                id: h.id,
                dir,
                state: OpState::Queued {
                    until: h.deadline,
                    msg: data,
                },
                _ticket: ticket,
            });
            return;
        }
    }
    let m = Meta { id: h.id, dir, ticket };
    start(stack, o, h, &data, m, created);
}

/// Posune vsechny dotazy objektu (po zpracovani site nebo vyprseni):
/// hotove odpovi, za skoncene se spusti dalsi ve fronte smeru, ve fronte
/// propadle odpovi `TIMED_OUT`.
pub fn progress(stack: &mut Stack, o: &mut Obj, created: &mut Vec<Obj>) {
    let now = now_ns();
    let old = core::mem::take(&mut o.ops);
    let mut freed: Vec<Dir> = Vec::new();
    for op in old {
        match op.state {
            OpState::Active(p) => {
                let step = obj::progress(stack, &mut o.kind.borrow_mut(), p);
                let dir = op.dir;
                let m = Meta {
                    id: op.id,
                    dir,
                    ticket: op._ticket,
                };
                if settle(stack, o, m, step, created) && !freed.contains(&dir) {
                    freed.push(dir);
                }
            }
            OpState::Queued { until, .. } if now >= until => {
                reply(stack, o, op.id, Err(crate::stack::expired(until)), None, created);
            }
            OpState::Queued { .. } => o.ops.push(op),
        }
    }
    for dir in freed {
        start_queued(stack, o, dir, created);
    }
    // Odlozene dotazy relace: propadle odpovi, na uvolnene misto se spusti.
    let mut i = 0;
    while i < o.backlog.len() {
        match header_of(&o.backlog[i]) {
            Some(h) if now >= h.deadline => {
                o.backlog.remove(i);
                reply(stack, o, h.id, Err(crate::stack::expired(h.deadline)), None, created);
            }
            _ => i += 1,
        }
    }
    drain_backlog(stack, o, created);
}

/// Nejpozdejsi okamzik, kdy je treba objekt znovu posoudit.
pub fn wake_at(o: &Obj) -> u64 {
    let ops = o.ops.iter().map(|x| match &x.state {
        OpState::Active(p) => p.wake_at(),
        OpState::Queued { until, .. } => *until,
    });
    let backlog = o.backlog.iter().filter_map(|d| header_of(d).map(|h| h.deadline));
    ops.chain(backlog).min().unwrap_or(u64::MAX)
}

/// Ma objekt neco rozpracovaneho (posouva se v kazdem kole)?
pub fn busy(o: &Obj) -> bool {
    !o.ops.is_empty() || !o.backlog.is_empty()
}
