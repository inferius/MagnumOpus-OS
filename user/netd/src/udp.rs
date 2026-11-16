//! Sockety UDP.

use alloc::collections::VecDeque;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use core::net::SocketAddr;

use libos::{Error, Result};
use netproto::{Buffers, MAX_DATAGRAM, ReadAddr, Reader, Verdict, WriteAddr, Writer, op, proto, rights};
use smoltcp::wire::IpAddress;

use crate::fw::app_name;
use crate::lo;
use crate::obj::{
    AuthRef, Deferred, Gate, Kind, Pending, Step, UDP_TARGETS_MAX, Udp, ask_progress, ask_wait, gate,
};
use crate::stack::{PortProto, Stack, expired, now_ns, v4};
use crate::tcp::to_sa;

/// Jak dlouho ceka odeslani na misto v plne fronte socketu.
const SEND_WAIT_NS: u64 = 5_000_000_000;

/// `UDP_BIND` na relaci.
pub fn bind(stack: &mut Stack, auth: &AuthRef, r: &mut Reader) -> Result<Step> {
    let bind = lo::bind_of(stack, r.socket_addr()?)?;
    let port = bind.port;
    let buffers = Buffers::get(r)?;
    auth.borrow().require(rights::UDP)?;
    if !auth.borrow().allows_bind(port) {
        return Err(Error::AccessDenied);
    }
    let (rx, tx) = buffers.resolved();
    let usage = auth.borrow().charge((rx + tx) as u64)?;
    let port = stack.take_port(PortProto::Udp, port)?;
    let bind = lo::Bind { port, ..bind };
    // Socket smoltcp jen pro sit (loopback ma vlastni frontu).
    let sock = if bind.net {
        let sock = stack.new_udp(rx, tx);
        if stack.udp(sock).bind(port).is_err() {
            stack.remove(sock);
            stack.release_port(PortProto::Udp, port);
            return Err(Error::InvalidArgs);
        }
        Some(sock)
    } else {
        None
    };
    let lo_queue = bind.lo.map(|ip| lo::udp_bind(stack, ip, port));
    let app = app_name(&auth.borrow());
    stack.fw.socket_opened(&app);
    let local = bind.local(stack);
    let mut w = Writer::new();
    w.socket_addr(local);
    Ok(Step::Done(
        Ok(w),
        Some(Kind::Udp(Udp {
            sock,
            port,
            bind,
            lo: lo_queue,
            auth: auth.clone(),
            app,
            allowed: Vec::new(),
            seen: VecDeque::new(),
            _usage: usage,
        })),
    ))
}

pub fn request(stack: &mut Stack, u: &mut Udp, code: u32, until: u64, r: &mut Reader) -> Result<Step> {
    match code {
        op::SEND_TO => {
            let to = r.socket_addr()?;
            // Na misto v plne fronte nejvys SEND_WAIT_NS.
            let until = until.min(now_ns().saturating_add(SEND_WAIT_NS));
            let data = r.rest().to_vec();
            let cap = u.sock.map_or(MAX_DATAGRAM, |h| stack.udp(h).payload_send_capacity());
            if data.len() > MAX_DATAGRAM || data.len() > cap {
                return Err(Error::OutOfRange);
            }
            if to.port() == 0 || to.ip().is_unspecified() {
                return Err(Error::InvalidArgs);
            }
            // Mimo loopback jen IPv4 a socket se siti.
            if !lo::is_loopback(to.ip()) {
                v4(to.ip())?;
                if u.sock.is_none() {
                    return Err(Error::Unreachable);
                }
            }
            send_to_start(stack, u, to, data, until, Vec::new())
        }
        op::RECV_FROM => {
            let max = (r.u32()? as usize).clamp(1, MAX_DATAGRAM);
            Ok(recv_progress(stack, u, max, until, now_ns()))
        }
        op::LOCAL_ADDR => {
            let mut w = Writer::new();
            w.socket_addr(u.bind.local(stack));
            Ok(Step::reply(Ok(w)))
        }
        _ => Err(Error::NotSupported),
    }
}

fn remember(list: &mut VecDeque<SocketAddr>, to: SocketAddr) -> bool {
    if list.contains(&to) {
        return false;
    }
    if list.len() >= UDP_TARGETS_MAX {
        list.pop_front();
    }
    list.push_back(to);
    true
}

/// Odeslani po kontrole cile (cile povolene pro socket se nekontroluji
/// znovu; prvni datagram na cil se zapise do auditu).
fn send_to_start(
    stack: &mut Stack,
    u: &mut Udp,
    to: SocketAddr,
    data: Vec<u8>,
    until: u64,
    granted: Vec<String>,
) -> Result<Step> {
    if !u.allowed.contains(&to) {
        // Uz videny cil s platnym povolenim: bez noveho zapisu do auditu.
        let quiet = granted.is_empty()
            && u.seen.contains(&to)
            && u.auth.borrow().check(proto::UDP, to, now_ns(), &stack.fw.user, &[]) == Verdict::Allow;
        if !quiet {
            match gate(stack, &u.auth.borrow(), proto::UDP, to, &granted)? {
                Gate::Go => {}
                Gate::Ask(id, app) => {
                    return Ok(ask_wait(id, app, proto::UDP, to, granted, Deferred::SendTo { to, data, until }));
                }
            }
            remember(&mut u.seen, to);
        }
        if !granted.is_empty() {
            if u.allowed.len() >= UDP_TARGETS_MAX {
                u.allowed.remove(0);
            }
            u.allowed.push(to);
        }
    }
    if !lo::is_loopback(to.ip()) {
        stack.local_ip()?;
    }
    Ok(send_progress(stack, u, to, data, until, now_ns()))
}

pub fn progress(stack: &mut Stack, u: &mut Udp, p: Pending, now: u64) -> Step {
    match p {
        Pending::SendTo { to, data, deadline } => send_progress(stack, u, to, data, deadline, now),
        Pending::RecvFrom { max, deadline } => recv_progress(stack, u, max, deadline, now),
        Pending::Ask(a) => match ask_progress(stack, a, now) {
            Ok((granted, Deferred::SendTo { to, data, until })) => {
                send_to_start(stack, u, to, data, until, granted).unwrap_or_else(Step::err)
            }
            Ok(_) => Step::err(Error::BadState),
            Err(step) => step,
        },
        _ => Step::err(Error::BadState),
    }
}

fn send_progress(
    stack: &mut Stack,
    u: &Udp,
    to: SocketAddr,
    data: Vec<u8>,
    deadline: u64,
    now: u64,
) -> Step {
    // Loopback: rovnou do fronty ciloveho socketu (plna = zahozeno).
    if lo::is_loopback(to.ip()) {
        stack.fw.traffic(&u.app, data.len(), 0);
        lo::udp_send(stack, u.port, to, data);
        return Step::ok();
    }
    let (Ok(ip), Some(h)) = (v4(to.ip()), u.sock) else {
        return Step::err(Error::Unreachable);
    };
    match stack
        .udp(h)
        .send_slice(&data, (IpAddress::Ipv4(ip), to.port()))
    {
        Ok(()) => {
            stack.note_udp_out(u.port, to);
            stack.fw.traffic(&u.app, data.len(), 0);
            Step::ok()
        }
        Err(smoltcp::socket::udp::SendError::BufferFull) if now < deadline => {
            Step::Wait(Pending::SendTo { to, data, deadline })
        }
        Err(smoltcp::socket::udp::SendError::BufferFull) => Step::err(expired(deadline)),
        Err(_) => Step::err(Error::Unreachable),
    }
}

fn recv_progress(stack: &mut Stack, u: &Udp, max: usize, deadline: u64, now: u64) -> Step {
    // Nejdriv loopback (stejny filtr odesilatelu jako ze site).
    if let Some(q) = &u.lo {
        while let Some((from, data)) = lo::udp_recv(stack, q) {
            if !(u.allowed.contains(&from) || u.auth.borrow().check(proto::UDP, from, now, &stack.fw.user, &[]) == Verdict::Allow) {
                continue;
            }
            stack.fw.traffic(&u.app, 0, data.len());
            let mut w = Writer::new();
            w.socket_addr(from).bytes(&data[..data.len().min(max)]);
            return Step::reply(Ok(w));
        }
    }
    let Some(h) = u.sock else {
        if now >= deadline {
            return Step::err(expired(deadline));
        }
        return Step::Wait(Pending::RecvFrom { max, deadline });
    };
    let mut buf = vec![0u8; stack.udp(h).payload_recv_capacity().min(MAX_DATAGRAM)];
    while let Ok((n, meta)) = stack.udp(h).recv_slice(&mut buf) {
        let from = to_sa(meta.endpoint);
        // Datagram od odesilatele mimo omezeni relace se zahodi (bez dotazu:
        // prichozi provoz se uzivateli nenabizi).
        let ok = u.allowed.contains(&from)
            || u.auth.borrow().check(proto::UDP, from, now, &stack.fw.user, &[]) == Verdict::Allow;
        if !ok {
            continue;
        }
        stack.fw.traffic(&u.app, 0, n);
        let mut w = Writer::new();
        w.socket_addr(from).bytes(&buf[..n.min(max)]);
        return Step::reply(Ok(w));
    }
    if now >= deadline {
        return Step::err(expired(deadline));
    }
    Step::Wait(Pending::RecvFrom { max, deadline })
}

/// Kanal socketu zavren klientem.
pub fn close(stack: &mut Stack, u: Udp) {
    if let Some(h) = u.sock {
        stack.remove(h);
    }
    if u.lo.is_some() {
        lo::udp_unbind(stack, u.bind.v6(), u.port);
    }
    stack.release_port(PortProto::Udp, u.port);
}
