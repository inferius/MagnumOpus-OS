//! Navazany socket TCP (`TCP_BIND`, C `bind` pred `listen`/`connect`):
//! drzi port od navazani (jako Linux: `getsockname` hned zna port, kolize
//! = `ADDRESS_IN_USE` uz z `bind`). `LISTEN` a `CONNECT` ho prevedou na
//! misto (`obj::request`/`progress`), takze to vidi i dalsi kanaly po
//! `DUP` (wineserver a program Wine maji kazdy svuj kanal).
//!
//! Opravneni se kontroluji pri navazani (pravo, `allows_bind`, prijem ze
//! site na zvolenem portu) a znovu pri `LISTEN` (pravo `TCP_LISTEN`, sit
//! i pro port 0) a `CONNECT` (pravo `TCP_CONNECT`, cil).

use alloc::string::String;
use alloc::vec::Vec;

use libos::{Error, Result};
use netproto::auth::Usage;
use netproto::{Buffers, ReadAddr, Reader, WriteAddr, Writer, op, rights};

use crate::fw::app_name;
use crate::obj::{AuthRef, Deferred, Kind, Pending, Step, ask_progress};
use crate::stack::{PortProto, Stack};
use crate::{lo, tcp};

pub struct Bound {
    /// Obsazeny port (patri socketu az do prevedeni nebo zavreni).
    pub port: u16,
    pub bind: lo::Bind,
    pub auth: AuthRef,
    pub app: String,
    /// Strzeni socketu; `None` = predane `LISTEN`/`CONNECT`, ktery selhal
    /// (socket je pouzity, dalsi pokus `BAD_STATE`, port drzi dal).
    pub usage: Option<Usage>,
    /// Bezi `CONNECT` (dalsi `LISTEN`/`CONNECT` = `BAD_STATE`).
    pub busy: bool,
}

impl Bound {
    /// Strzeni socketu navysene o buffery (pro naslouchani / spojeni).
    pub fn take_usage(&mut self, bytes: u64) -> Result<Usage> {
        let mut u = self.usage.take().ok_or(Error::BadState)?;
        if let Err(e) = u.grow(bytes) {
            self.usage = Some(u);
            return Err(e);
        }
        Ok(u)
    }
}

/// `TCP_BIND` na relaci.
pub fn bind(stack: &mut Stack, auth: &AuthRef, r: &mut Reader) -> Result<Step> {
    let bind = lo::bind_of(stack, r.socket_addr()?)?;
    {
        let a = auth.borrow();
        if !a.has(rights::TCP_CONNECT) && !a.has(rights::TCP_LISTEN) {
            return Err(Error::AccessDenied);
        }
        // Port 0 (klient pred `connect`) smi vzdy; zvoleny port na siti jen
        // s prijmem ze site.
        if !a.allows_bind(bind.port) || (bind.net && bind.port != 0 && !a.allows_bind_net()) {
            return Err(Error::AccessDenied);
        }
    }
    let usage = auth.borrow().charge(0)?;
    let port = stack.take_port(PortProto::Tcp, bind.port)?;
    let bind = lo::Bind { port, ..bind };
    let app = app_name(&auth.borrow());
    let mut w = Writer::new();
    w.socket_addr(bind.local(stack));
    Ok(Step::Done(
        Ok(w),
        Some(Kind::Bound(Bound { port, bind, auth: auth.clone(), app, usage: Some(usage), busy: false })),
    ))
}

/// Dotaz na kanalu navazaneho socketu. `Done(Ok, Some(kind))` = prevest
/// objekt na misto (`obj::request`).
pub fn request(stack: &mut Stack, b: &mut Bound, code: u32, until: u64, r: &mut Reader) -> Result<Step> {
    match code {
        op::LOCAL_ADDR => {
            let mut w = Writer::new();
            w.socket_addr(b.bind.local(stack));
            Ok(Step::reply(Ok(w)))
        }
        op::LISTEN => {
            let backlog = r.u16()?;
            let buffers = Buffers::get(r)?;
            if b.busy {
                return Err(Error::BadState);
            }
            b.auth.borrow().require(rights::TCP_LISTEN)?;
            if b.bind.net && !b.auth.borrow().allows_bind_net() {
                return Err(Error::AccessDenied);
            }
            let backlog = tcp::backlog_of(backlog);
            let (rx, tx) = buffers.resolved();
            let usage = b.take_usage((tcp::spare_of(&b.bind, backlog) * (rx + tx)) as u64)?;
            let auth = b.auth.clone();
            let (w, kind) = tcp::start_listener(stack, &auth, b.bind, backlog, rx, tx, usage)?;
            Ok(Step::Done(Ok(w), Some(kind)))
        }
        op::CONNECT => {
            let remote = r.socket_addr()?;
            let buffers = Buffers::get(r)?;
            if b.busy || b.usage.is_none() {
                return Err(Error::BadState);
            }
            b.auth.borrow().require(rights::TCP_CONNECT)?;
            // Cil musi byt dosazitelny z adresy navazani (rodina, loopback
            // nebo sit).
            let lo_target = lo::is_loopback(remote.ip());
            if remote.is_ipv6() != b.bind.v6() {
                return Err(Error::InvalidArgs);
            }
            if (lo_target && b.bind.lo.is_none()) || (!lo_target && !b.bind.net) {
                return Err(Error::AddressNotAvailable);
            }
            b.busy = true;
            let auth = b.auth.clone();
            let req = tcp::ConnectReq::new(remote, until, buffers);
            let step = tcp::connect_start(stack, &auth, req, Vec::new(), Some(b));
            Ok(settled(b, step.unwrap_or_else(Step::err)))
        }
        _ => Err(Error::NotSupported),
    }
}

/// Posun `CONNECT` (dotaz uzivateli, navazovani).
pub fn progress(stack: &mut Stack, b: &mut Bound, p: Pending, now: u64) -> Step {
    let step = match p {
        Pending::Connect(c) => tcp::connect_progress(stack, c, now),
        Pending::Ask(a) => match ask_progress(stack, a, now) {
            Ok((granted, Deferred::Connect(req))) => {
                let auth = b.auth.clone();
                tcp::connect_start(stack, &auth, req, granted, Some(b)).unwrap_or_else(Step::err)
            }
            Ok(_) => Step::err(Error::BadState),
            Err(step) => step,
        },
        _ => Step::err(Error::BadState),
    };
    settled(b, step)
}

/// Konec `CONNECT` s chybou: socket uz neceka (pouzity, port drzi dal).
fn settled(b: &mut Bound, step: Step) -> Step {
    if matches!(step, Step::Done(Err(_), _)) {
        b.busy = false;
    }
    step
}

pub fn close(stack: &mut Stack, b: Bound) {
    stack.release_port(PortProto::Tcp, b.port);
}
