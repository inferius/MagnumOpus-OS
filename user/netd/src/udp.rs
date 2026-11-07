//! Sockety UDP.

use alloc::vec;
use alloc::vec::Vec;
use core::net::SocketAddr;

use libos::{Error, Result};
use netproto::{MAX_DATAGRAM, ReadAddr, Reader, WriteAddr, Writer, op, proto, rights};
use smoltcp::wire::IpAddress;

use crate::obj::{AuthRef, Kind, Pending, Step, Udp};
use crate::stack::{PortProto, Stack, deadline, now_ns, v4};
use crate::tcp::to_sa;

/// Jak dlouho ceka odeslani na misto v plne fronte socketu.
const SEND_WAIT_NS: u64 = 5_000_000_000;

/// `UDP_BIND` na relaci.
pub fn bind(stack: &mut Stack, auth: &AuthRef, r: &mut Reader) -> Result<Step> {
    let port = r.u16()?;
    auth.borrow().require(rights::UDP)?;
    if !auth.borrow().allows_bind(port) {
        return Err(Error::AccessDenied);
    }
    let port = stack.take_port(PortProto::Udp, port)?;
    let sock = stack.new_udp();
    if stack.udp(sock).bind(port).is_err() {
        stack.remove(sock);
        stack.release_port(PortProto::Udp, port);
        return Err(Error::InvalidArgs);
    }
    let local = stack.local_addr(port);
    let mut w = Writer::new();
    w.socket_addr(local);
    Ok(Step::Done(
        Ok(w),
        Some(Kind::Udp(Udp {
            sock,
            port,
            auth: auth.clone(),
        })),
    ))
}

pub fn request(stack: &mut Stack, u: &mut Udp, code: u32, r: &mut Reader) -> Result<Step> {
    match code {
        op::SEND_TO => {
            let to = r.socket_addr()?;
            let data = r.rest().to_vec();
            if data.len() > MAX_DATAGRAM {
                return Err(Error::OutOfRange);
            }
            if !u.auth.borrow().allows(proto::UDP, to) {
                return Err(Error::AccessDenied);
            }
            let ip = v4(to.ip())?;
            if to.port() == 0 || ip.is_unspecified() {
                return Err(Error::InvalidArgs);
            }
            stack.local_ip()?;
            Ok(send_progress(
                stack,
                u,
                to,
                data,
                now_ns().saturating_add(SEND_WAIT_NS),
                now_ns(),
            ))
        }
        op::RECV_FROM => {
            let max = (r.u32()? as usize).clamp(1, MAX_DATAGRAM);
            let timeout = r.u64()?;
            Ok(recv_progress(stack, u, max, deadline(timeout), now_ns()))
        }
        op::LOCAL_ADDR => {
            let mut w = Writer::new();
            w.socket_addr(stack.local_addr(u.port));
            Ok(Step::reply(Ok(w)))
        }
        _ => Err(Error::NotSupported),
    }
}

pub fn progress(stack: &mut Stack, u: &mut Udp, p: Pending, now: u64) -> Step {
    match p {
        Pending::SendTo { to, data, deadline } => send_progress(stack, u, to, data, deadline, now),
        Pending::RecvFrom { max, deadline } => recv_progress(stack, u, max, deadline, now),
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
    let Ok(ip) = v4(to.ip()) else {
        return Step::err(Error::Unreachable);
    };
    match stack
        .udp(u.sock)
        .send_slice(&data, (IpAddress::Ipv4(ip), to.port()))
    {
        Ok(()) => Step::ok(),
        Err(smoltcp::socket::udp::SendError::BufferFull) if now < deadline => {
            Step::Wait(Pending::SendTo { to, data, deadline })
        }
        Err(smoltcp::socket::udp::SendError::BufferFull) => Step::err(Error::TimedOut),
        Err(_) => Step::err(Error::Unreachable),
    }
}

fn recv_progress(stack: &mut Stack, u: &Udp, max: usize, deadline: u64, now: u64) -> Step {
    let mut buf = vec![0u8; MAX_DATAGRAM];
    while let Ok((n, meta)) = stack.udp(u.sock).recv_slice(&mut buf) {
        let from = to_sa(meta.endpoint);
        // Datagram od odesilatele mimo omezeni relace se zahodi.
        if !u.auth.borrow().allows(proto::UDP, from) {
            continue;
        }
        let mut w = Writer::new();
        w.socket_addr(from).bytes(&buf[..n.min(max)]);
        return Step::reply(Ok(w));
    }
    if now >= deadline {
        return Step::err(Error::TimedOut);
    }
    Step::Wait(Pending::RecvFrom { max, deadline })
}
