//! Objekty sluzby: kazdy kanal od klienta je relace, spojeni TCP,
//! naslouchani TCP nebo socket UDP. Na kanalu je nejvys jeden rozpracovany
//! pozadavek (`Pending`); dalsi zpravu sluzba precte az po odpovedi.

use alloc::rc::Rc;
use alloc::vec::Vec;
use core::cell::RefCell;
use core::net::SocketAddr;

use libos::{Channel, Error, Result};
use netproto::{Auth, Reader, Writer};
use smoltcp::iface::SocketHandle;

use crate::ping::Ping;
use crate::resolve::Resolve;
use crate::stack::Stack;
use crate::{session, tcp, udp};

/// Opravneni relace sdilene s jejimi sockety (sockety UDP podle nej
/// filtruji, a preklady jmen z relace plati i pro ne).
pub type AuthRef = Rc<RefCell<Auth>>;

pub struct Obj {
    pub ch: Channel,
    pub kind: Kind,
    pub pending: Option<Pending>,
}

pub enum Kind {
    Session(AuthRef),
    Stream(Stream),
    Listener(Listener),
    Udp(Udp),
}

pub struct Stream {
    pub sock: SocketHandle,
    pub local: SocketAddr,
    pub remote: SocketAddr,
    /// Docasny port, ktery spojeni obsadilo (u prijatych spojeni zadny -
    /// port patri naslouchani).
    pub port: Option<u16>,
    /// Druha strana poslala FIN (rozliseni konce proudu od RST).
    pub fin_seen: bool,
}

pub struct Listener {
    pub port: u16,
    pub local: SocketAddr,
    /// Sockety ve stavu naslouchani nebo navazane, cekajici na `accept`.
    pub backlog: Vec<SocketHandle>,
}

pub struct Udp {
    pub sock: SocketHandle,
    pub port: u16,
    pub local: SocketAddr,
    pub auth: AuthRef,
}

/// Rozpracovany pozadavek.
pub enum Pending {
    WaitConfigured {
        deadline: u64,
    },
    Connect(tcp::Connect),
    Resolve(Resolve),
    Ping(Ping),
    Send {
        data: Vec<u8>,
        done: usize,
        deadline: u64,
    },
    Recv {
        max: usize,
        deadline: u64,
    },
    Accept {
        deadline: u64,
    },
    SendTo {
        to: SocketAddr,
        data: Vec<u8>,
        deadline: u64,
    },
    RecvFrom {
        max: usize,
        deadline: u64,
    },
}

impl Pending {
    /// Nejpozdejsi okamzik (ns), kdy je treba pozadavek znovu posoudit.
    pub fn wake_at(&self) -> u64 {
        match self {
            Self::WaitConfigured { deadline }
            | Self::Send { deadline, .. }
            | Self::Recv { deadline, .. }
            | Self::Accept { deadline }
            | Self::SendTo { deadline, .. }
            | Self::RecvFrom { deadline, .. } => *deadline,
            Self::Connect(c) => c.deadline,
            Self::Resolve(r) => r.wake_at(),
            Self::Ping(p) => p.deadline,
        }
    }
}

/// Vysledek kroku pozadavku.
pub enum Step {
    /// Pozadavek ceka (na data, spojeni, odpoved site).
    Wait(Pending),
    /// Hotovo: odpoved a pripadne novy objekt, jehoz kanal pujde klientovi
    /// v handlech odpovedi.
    Done(Result<Writer>, Option<Kind>),
}

impl Step {
    pub fn reply(r: Result<Writer>) -> Self {
        Self::Done(r, None)
    }

    pub fn err(e: Error) -> Self {
        Self::Done(Err(e), None)
    }

    pub fn ok() -> Self {
        Self::Done(Ok(Writer::new()), None)
    }
}

/// Novy pozadavek na objektu.
pub fn request(stack: &mut Stack, kind: &mut Kind, data: &[u8]) -> Step {
    let mut r = Reader::new(data);
    let Ok(code) = r.u32() else {
        return Step::err(Error::InvalidArgs);
    };
    let step = match kind {
        Kind::Session(auth) => session::request(stack, auth, code, &mut r),
        Kind::Stream(s) => tcp::stream_request(stack, s, code, &mut r),
        Kind::Listener(l) => tcp::listener_request(l, code, &mut r),
        Kind::Udp(u) => udp::request(stack, u, code, &mut r),
    };
    match step {
        Ok(s) => s,
        Err(e) => Step::err(e),
    }
}

/// Posun rozpracovaneho pozadavku (po zpracovani site nebo vyprseni).
pub fn progress(stack: &mut Stack, kind: &mut Kind, p: Pending) -> Step {
    let now = crate::stack::now_ns();
    match (kind, p) {
        (Kind::Session(auth), p) => session::progress(stack, auth, p, now),
        (Kind::Stream(s), p) => tcp::stream_progress(stack, s, p, now),
        (Kind::Listener(l), Pending::Accept { deadline }) => {
            tcp::accept_progress(stack, l, deadline, now)
        }
        (Kind::Udp(u), p) => udp::progress(stack, u, p, now),
        (_, _) => Step::err(Error::BadState),
    }
}

/// Uklid po zavreni kanalu klientem.
pub fn close(stack: &mut Stack, kind: Kind, pending: Option<Pending>) {
    if let Some(p) = pending {
        match p {
            Pending::Connect(c) => c.cancel(stack),
            Pending::Resolve(r) => r.cancel(stack),
            Pending::Ping(p) => p.cancel(stack),
            _ => {}
        }
    }
    match kind {
        Kind::Session(_) => {}
        Kind::Stream(s) => tcp::close_stream(stack, s),
        Kind::Listener(l) => tcp::close_listener(stack, l),
        Kind::Udp(u) => {
            stack.remove(u.sock);
            stack.release_port(crate::stack::PortProto::Udp, u.port);
        }
    }
}
