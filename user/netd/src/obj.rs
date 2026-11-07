//! Objekty sluzby: kazdy kanal od klienta je relace, spojeni TCP,
//! naslouchani TCP nebo socket UDP. Na kanalu je nejvys jeden rozpracovany
//! pozadavek (`Pending`); dalsi zpravu sluzba precte az po odpovedi.
//!
//! Kazdy socket drzi strzeni z rozpoctu sve relace ([`Usage`]); zanikem
//! objektu se vrati.

use alloc::collections::VecDeque;
use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;
use core::cell::RefCell;
use core::net::SocketAddr;

use libos::{Channel, Error, Result};
use netproto::auth::Usage;
use netproto::{Auth, Reader, Verdict, Writer};
use smoltcp::iface::SocketHandle;

use crate::ping::{Ping, PingReq};
use crate::resolve::{Resolve, ResolveReq};
use crate::stack::{Stack, now_ns};
use crate::{fw, session, tcp, udp};

/// Opravneni relace sdilene s jejimi sockety (sockety UDP podle nej
/// filtruji, a preklady jmen z relace plati i pro ne).
pub type AuthRef = Rc<RefCell<Auth>>;

pub struct Obj {
    pub ch: Channel,
    pub kind: Kind,
    pub pending: Option<Pending>,
    /// Je na portu registrace "kanal je ke cteni"?
    pub read_armed: bool,
    /// Je na portu registrace "klient kanal zavrel"?
    pub close_armed: bool,
}

impl Obj {
    pub fn new(ch: Channel, kind: Kind) -> Self {
        Self {
            ch,
            kind,
            pending: None,
            read_armed: false,
            close_armed: false,
        }
    }
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
    /// Aplikace (statistiky provozu).
    pub app: String,
    pub _usage: Usage,
}

pub struct Listener {
    pub port: u16,
    /// Sockety ve stavu naslouchani (prijimaji SYN).
    pub listening: Vec<SocketHandle>,
    /// Navazana spojeni cekajici na `accept`.
    pub ready: VecDeque<SocketHandle>,
    /// Nejvic spojeni ve fronte (`ready` + rozpracovana).
    pub backlog: usize,
    /// Buffery prijatych spojeni.
    pub rx: usize,
    pub tx: usize,
    pub auth: AuthRef,
    pub app: String,
    /// Strzeni za sockety naslouchani.
    pub _usage: Usage,
}

pub struct Udp {
    pub sock: SocketHandle,
    pub port: u16,
    pub auth: AuthRef,
    pub app: String,
    /// Cile, ktere uzivatel pro tento socket povolil jednorazove (plati do
    /// zavreni socketu, vcetne odpovedi z nich).
    pub allowed: Vec<SocketAddr>,
    /// Cile, ktere uz jsou v auditu (prvni datagram na cil se zapise).
    pub seen: VecDeque<SocketAddr>,
    pub _usage: Usage,
}

/// Nejvic jednorazove povolenych a zapamatovanych cilu socketu UDP.
pub const UDP_TARGETS_MAX: usize = 64;

/// Operace, ktera ceka na rozhodnuti uzivatele.
pub enum Deferred {
    Connect(tcp::ConnectReq),
    Ping(PingReq),
    Resolve(ResolveReq),
    SendTo { to: SocketAddr, data: Vec<u8> },
}

pub struct AskWait {
    pub id: u32,
    pub app: String,
    pub proto: u8,
    pub dest: SocketAddr,
    pub deadline: u64,
    /// Aplikace, kterym uz uzivatel tento pozadavek povolil (vic vrstev
    /// s dotazem).
    pub granted: Vec<String>,
    pub then: Deferred,
}

/// Rozpracovany pozadavek.
pub enum Pending {
    WaitConfigured {
        deadline: u64,
    },
    Connect(tcp::Connect),
    Resolve(Resolve),
    /// Preklad pres dnsd (ceka na odpoved podle id).
    Forward(crate::forward::Forward),
    Ping(Ping),
    Ask(AskWait),
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
            Self::Forward(f) => f.deadline,
            Self::Ping(p) => p.deadline,
            Self::Ask(a) => a.deadline,
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

/// Vysledek kontroly cile operace.
pub enum Gate {
    /// Smi hned.
    Go,
    /// Ceka na uzivatele (dotaz odeslan): id dotazu a aplikace.
    Ask(u32, String),
}

/// Kontrola cile: povoleno, zakazano (`ACCESS_DENIED`), nebo dotaz pres
/// `fwd` (vrstva s `ask`). Bez `fwd` se dotaz rovnou odmitne.
pub fn gate(
    stack: &mut Stack,
    auth: &Auth,
    proto: u8,
    dest: SocketAddr,
    granted: &[String],
) -> Result<Gate> {
    let now = now_ns();
    match stack.fw.check(auth, proto, dest, now, granted) {
        Verdict::Allow => Ok(Gate::Go),
        Verdict::Deny => Err(Error::AccessDenied),
        Verdict::Ask { app } => {
            let host = auth.name_for(dest.ip(), now).unwrap_or("");
            match stack.fw.ask(&app, proto, dest, host) {
                Some(id) => Ok(Gate::Ask(id, app)),
                None => {
                    stack.fw.record_answer(&app, proto, dest, false);
                    Err(Error::AccessDenied)
                }
            }
        }
    }
}

/// Zalozi cekani na uzivatele.
pub fn ask_wait(id: u32, app: String, proto: u8, dest: SocketAddr, granted: Vec<String>, then: Deferred) -> Step {
    Step::Wait(Pending::Ask(AskWait {
        id,
        app,
        proto,
        dest,
        deadline: now_ns().saturating_add(fw::ASK_WAIT_NS),
        granted,
        then,
    }))
}

/// Posun cekani na uzivatele: `Ok(granted)` = povoleno (pokracovat
/// s timto seznamem povolenych aplikaci), `Err(krok)` = jeste ceka nebo
/// zamitnuto.
pub fn ask_progress(stack: &mut Stack, mut a: AskWait, now: u64) -> core::result::Result<(Vec<String>, Deferred), Step> {
    match stack.fw.take_answer(a.id) {
        Some(true) => {
            a.granted.push(a.app);
            Ok((a.granted, a.then))
        }
        Some(false) => {
            stack.fw.record_answer(&a.app, a.proto, a.dest, false);
            Err(Step::err(Error::AccessDenied))
        }
        None if now >= a.deadline => {
            stack.fw.record_answer(&a.app, a.proto, a.dest, false);
            Err(Step::err(Error::AccessDenied))
        }
        None => Err(Step::Wait(Pending::Ask(a))),
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
        Kind::Listener(l) => tcp::listener_request(stack, l, code, &mut r),
        Kind::Udp(u) => udp::request(stack, u, code, &mut r),
    };
    match step {
        Ok(s) => s,
        Err(e) => Step::err(e),
    }
}

/// Posun rozpracovaneho pozadavku (po zpracovani site nebo vyprseni).
pub fn progress(stack: &mut Stack, kind: &mut Kind, p: Pending) -> Step {
    let now = now_ns();
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
            Pending::Forward(f) => stack.resolver.forget(f.id),
            Pending::Ping(p) => p.cancel(stack),
            Pending::Ask(a) => {
                // Odpoved, ktera jeste prijde, uz nikdo nevyzvedne.
                stack.fw.forget_answer(a.id);
            }
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
