//! Objekty sluzby: kazdy kanal od klienta je relace, spojeni TCP,
//! naslouchani TCP nebo socket UDP. Na kanalu muze byt vic rozpracovanych
//! dotazu najednou ([`Op`], podle cisla dotazu z hlavicky); dotazy stejneho
//! smeru na socketu ([`Dir`]) cekaji ve fronte FIFO (`ops.rs`).
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
use netproto::auth::{OpTicket, Usage};
use netproto::{Auth, Reader, Verdict, Writer};
use smoltcp::iface::SocketHandle;

use crate::ping::{Ping, PingReq};
use crate::resolve::{Resolve, ResolveReq};
use crate::stack::{Stack, now_ns};
use crate::{fw, lo, session, tcp, udp};

/// Opravneni relace sdilene s jejimi sockety (sockety UDP podle nej
/// filtruji, a preklady jmen z relace plati i pro ne).
pub type AuthRef = Rc<RefCell<Auth>>;

/// Stav objektu sdileny kanaly (`DUP`: vic kanalu ke stejnemu socketu).
pub type KindRef = Rc<RefCell<Kind>>;

pub struct Obj {
    pub ch: Channel,
    /// Stav (sdileny s dalsimi kanaly po `DUP`; uklidi se s poslednim).
    pub kind: KindRef,
    /// Rozpracovane a cekajici dotazy (poradi = poradi prijeti).
    pub ops: Vec<Op>,
    /// Relace s plnym limitem dotazu: odlozene zpravy (FIFO, nejvys
    /// `ops::SESSION_BACKLOG`); pak se kanal prestane cist.
    pub backlog: VecDeque<Vec<u8>>,
    /// Cteni kanalu zastavene (plny `backlog`).
    pub read_paused: bool,
    /// Event pripravenosti socketu (az po `READINESS`).
    pub ready: Option<crate::ready::Readiness>,
    /// Je na portu registrace "kanal je ke cteni"?
    pub read_armed: bool,
    /// Je na portu registrace "klient kanal zavrel"?
    pub close_armed: bool,
}

impl Obj {
    pub fn new(ch: Channel, kind: Kind) -> Self {
        Self::shared(ch, Rc::new(RefCell::new(kind)))
    }

    /// Dalsi kanal ke stejnemu objektu (`DUP`).
    pub fn shared(ch: Channel, kind: KindRef) -> Self {
        Self {
            ch,
            kind,
            ops: Vec::new(),
            backlog: VecDeque::new(),
            read_paused: false,
            ready: None,
            read_armed: false,
            close_armed: false,
        }
    }
}

/// Smer dotazu na socketu: dotazy stejneho smeru se vyrizuji postupne
/// (data se neprolozi), ruzne smery soubezne. `Free` = bez fronty
/// (dotazy relace, okamzite dotazy socketu).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dir {
    Free,
    /// RECV, RECV_FROM.
    Recv,
    /// SEND, SHUTDOWN (po drivejsich zapisech), SEND_TO.
    Send,
    /// ACCEPT.
    Accept,
}

/// Smer dotazu `code` na objektu `kind`.
pub fn dir_of(kind: &Kind, code: u32) -> Dir {
    use netproto::op;
    match (kind, code) {
        (Kind::Session(_), _) => Dir::Free,
        (_, op::RECV | op::RECV_FROM) => Dir::Recv,
        (_, op::SEND | op::SHUTDOWN | op::SEND_TO) => Dir::Send,
        (_, op::ACCEPT) => Dir::Accept,
        _ => Dir::Free,
    }
}

/// Jeden dotaz na kanalu.
pub struct Op {
    pub id: u32,
    pub dir: Dir,
    pub state: OpState,
    /// Misto v limitu soubeznych dotazu relace (jen dotazy relace).
    pub _ticket: Option<OpTicket>,
}

pub enum OpState {
    /// Rozpracovany.
    Active(Pending),
    /// Ceka ve fronte smeru: cela zprava dotazu a jeho termin.
    Queued { msg: Vec<u8>, until: u64 },
}

pub enum Kind {
    Session(AuthRef),
    Stream(Stream),
    Listener(Listener),
    Udp(Udp),
    /// Spojeni na loopbacku (`lo.rs`).
    LoStream(lo::LoStream),
    /// Navazany socket TCP pred `LISTEN`/`CONNECT` (`bound.rs`).
    Bound(crate::bound::Bound),
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
    /// Kam nasloucha (sit, loopback, obe).
    pub bind: lo::Bind,
    /// Fronta spojeni z loopbacku.
    pub lo: Option<lo::AcceptRef>,
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
    /// Socket smoltcp (`None` = jen loopback: 127.x, `::1`, `::`).
    pub sock: Option<SocketHandle>,
    pub port: u16,
    pub bind: lo::Bind,
    /// Fronta datagramu z loopbacku.
    pub lo: Option<lo::UdpRef>,
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
    SendTo { to: SocketAddr, data: Vec<u8>, until: u64 },
}

impl Deferred {
    /// Posune termin o dobu dotazu na uzivatele (ta se do terminu
    /// operace nepocita); "bez limitu" zustava.
    pub fn extend(&mut self, by: u64) {
        let until = match self {
            Deferred::Connect(r) => &mut r.until,
            Deferred::Ping(r) => &mut r.until,
            Deferred::Resolve(r) => &mut r.until,
            Deferred::SendTo { until, .. } => until,
        };
        if *until != netproto::NO_DEADLINE {
            *until = until.saturating_add(by).min(netproto::NO_DEADLINE - 1);
        }
    }
}

pub struct AskWait {
    pub id: u32,
    pub app: String,
    pub proto: u8,
    pub dest: SocketAddr,
    pub deadline: u64,
    /// Kdy dotaz na uzivatele zacal.
    pub started: u64,
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
        /// `RECV_PEEK`: data ve fronte zustanou.
        peek: bool,
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
        started: now_ns(),
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
            a.then.extend(now.saturating_sub(a.started));
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

/// Novy dotaz `code` s terminem `until` na objektu.
pub fn request(stack: &mut Stack, kind: &mut Kind, code: u32, until: u64, r: &mut Reader) -> Step {
    let step = match kind {
        Kind::Session(auth) => session::request(stack, auth, code, until, r),
        Kind::Stream(s) => tcp::stream_request(stack, s, code, until, r),
        Kind::Listener(l) => tcp::listener_request(stack, l, code, until, r),
        Kind::Udp(u) => udp::request(stack, u, code, until, r),
        Kind::LoStream(s) => lo::stream_request(stack, s, code, until, r),
        Kind::Bound(b) => crate::bound::request(stack, b, code, until, r),
    };
    let step = match step {
        Ok(s) => s,
        Err(e) => Step::err(e),
    };
    convert(kind, step)
}

/// Navazany socket po `LISTEN`/`CONNECT`: novy stav na miste (vidi ho
/// vsechny kanaly po `DUP`), klient zadny novy kanal nedostane. Port
/// prechazi na naslouchani / spojeni.
fn convert(kind: &mut Kind, step: Step) -> Step {
    match step {
        Step::Done(Ok(w), Some(new)) if matches!(kind, Kind::Bound(_)) => {
            *kind = new;
            Step::Done(Ok(w), None)
        }
        s => s,
    }
}

/// Posun rozpracovaneho pozadavku (po zpracovani site nebo vyprseni).
pub fn progress(stack: &mut Stack, kind: &mut Kind, p: Pending) -> Step {
    let now = now_ns();
    if let Kind::Bound(b) = kind {
        let step = crate::bound::progress(stack, b, p, now);
        return convert(kind, step);
    }
    match (kind, p) {
        (Kind::Session(auth), p) => session::progress(stack, auth, p, now),
        (Kind::Stream(s), p) => tcp::stream_progress(stack, s, p, now),
        (Kind::Listener(l), Pending::Accept { deadline }) => {
            tcp::accept_progress(stack, l, deadline, now)
        }
        (Kind::Udp(u), p) => udp::progress(stack, u, p, now),
        (Kind::LoStream(s), p) => lo::stream_progress(stack, s, p, now),
        (_, _) => Step::err(Error::BadState),
    }
}

/// Zrusi rozpracovany dotaz (uvolni sockety, porty a cekani). Vraci
/// odpoved: `CANCELED`, u odesilani s uz prijatou casti jeji delku.
pub fn cancel_pending(stack: &mut Stack, p: Pending) -> Result<Writer> {
    match p {
        Pending::Connect(c) => c.cancel(stack),
        Pending::Resolve(r) => r.cancel(stack),
        Pending::Forward(f) => stack.resolver.forget(f.id),
        Pending::Ping(p) => p.cancel(stack),
        Pending::Ask(a) => {
            // Odpoved, ktera jeste prijde, uz nikdo nevyzvedne.
            stack.fw.forget_answer(a.id);
        }
        Pending::Send { done, .. } if done > 0 => {
            let mut w = Writer::new();
            w.u32(done as u32);
            return Ok(w);
        }
        _ => {}
    }
    Err(Error::Canceled)
}

/// Druh objektu pro `op::KIND` (navazany socket mohl prevest jiny kanal).
pub fn sock_kind(kind: &Kind) -> u8 {
    use netproto::sock_kind as k;
    match kind {
        Kind::Session(_) => k::SESSION,
        Kind::Stream(_) | Kind::LoStream(_) => k::STREAM,
        Kind::Listener(_) => k::LISTENER,
        Kind::Udp(_) => k::UDP,
        Kind::Bound(_) => k::BOUND,
    }
}

/// Identita aplikace objektu (relace podle vrstev, socket podle relace,
/// ktera ho vytvorila; systemove = "system").
pub fn identity(kind: &Kind) -> String {
    match kind {
        Kind::Session(auth) => fw::app_name(&auth.borrow()),
        Kind::Stream(s) => s.app.clone(),
        Kind::Listener(l) => l.app.clone(),
        Kind::Udp(u) => u.app.clone(),
        Kind::LoStream(s) => s.app.clone(),
        Kind::Bound(b) => b.app.clone(),
    }
}

/// Kanal zavren klientem: zrusi jeho dotazy; objekt se uklidi az
/// s poslednim kanalem (`DUP`).
pub fn close_channel(stack: &mut Stack, kind: KindRef, ops: Vec<Op>) {
    for op in ops {
        if let OpState::Active(p) = op.state {
            let _ = cancel_pending(stack, p);
        }
    }
    if let Ok(k) = Rc::try_unwrap(kind) {
        close(stack, k.into_inner(), Vec::new());
    }
}

/// Uklid objektu (posledni kanal zavren, nebo novy objekt bez odpovedi).
pub fn close(stack: &mut Stack, kind: Kind, ops: Vec<Op>) {
    for op in ops {
        if let OpState::Active(p) = op.state {
            let _ = cancel_pending(stack, p);
        }
    }
    match kind {
        Kind::Session(_) => {}
        Kind::Stream(s) => tcp::close_stream(stack, s),
        Kind::Listener(l) => tcp::close_listener(stack, l),
        Kind::Udp(u) => udp::close(stack, u),
        Kind::LoStream(s) => lo::close_stream(stack, s),
        Kind::Bound(b) => crate::bound::close(stack, b),
    }
}
