//! Ping: ICMP echo pres socket ICMP smoltcp.

use alloc::vec;
use alloc::vec::Vec;
use core::net::{Ipv4Addr, SocketAddr};

use libos::{Error, Result};
use netproto::{ReadAddr, Reader, Writer, proto, rights};
use smoltcp::iface::SocketHandle;
use smoltcp::phy::ChecksumCapabilities;
use smoltcp::socket::icmp;
use smoltcp::wire::{Icmpv4Packet, Icmpv4Repr, IpAddress};

use crate::obj::{AuthRef, Pending, Step};
use crate::stack::{Stack, deadline, now_ns, v4};

/// Nejvic dat v jednom pozadavku.
const MAX_PAYLOAD: u16 = 1024;

pub struct Ping {
    sock: SocketHandle,
    ident: u16,
    seq: u16,
    dest: Ipv4Addr,
    payload: Vec<u8>,
    sent_at: Option<u64>,
    pub deadline: u64,
}

impl Ping {
    pub fn cancel(self, stack: &mut Stack) {
        stack.remove(self.sock);
    }

    fn try_send(&mut self, stack: &mut Stack) {
        if self.sent_at.is_some() {
            return;
        }
        let repr = Icmpv4Repr::EchoRequest {
            ident: self.ident,
            seq_no: self.seq,
            data: &self.payload,
        };
        let sock = stack.sockets.get_mut::<icmp::Socket>(self.sock);
        if let Ok(buf) = sock.send(repr.buffer_len(), IpAddress::Ipv4(self.dest)) {
            let mut pkt = Icmpv4Packet::new_unchecked(buf);
            repr.emit(&mut pkt, &ChecksumCapabilities::default());
            self.sent_at = Some(now_ns());
        }
    }
}

/// `PING` na relaci.
pub fn start(stack: &mut Stack, auth: &AuthRef, r: &mut Reader) -> Result<Step> {
    let addr = r.ip()?;
    let seq = r.u16()?;
    let size = r.u16()?;
    let timeout = r.u64()?;
    auth.borrow().require(rights::PING)?;
    if !auth.borrow().allows(proto::ICMP, SocketAddr::new(addr, 0)) {
        return Err(Error::AccessDenied);
    }
    if size > MAX_PAYLOAD {
        return Err(Error::OutOfRange);
    }
    let dest = v4(addr)?;
    stack.local_ip()?;
    let rx = icmp::PacketBuffer::new(vec![icmp::PacketMetadata::EMPTY; 4], vec![0u8; 4096]);
    let tx = icmp::PacketBuffer::new(vec![icmp::PacketMetadata::EMPTY; 4], vec![0u8; 4096]);
    let mut sock = icmp::Socket::new(rx, tx);
    let ident = stack.rng.next_u16();
    if sock.bind(icmp::Endpoint::Ident(ident)).is_err() {
        return Err(Error::NoResources);
    }
    let handle = stack.sockets.add(sock);
    let payload = (0..size).map(|i| (i & 0xFF) as u8).collect();
    let mut p = Ping {
        sock: handle,
        ident,
        seq,
        dest,
        payload,
        sent_at: None,
        deadline: deadline(timeout),
    };
    p.try_send(stack);
    Ok(Step::Wait(Pending::Ping(p)))
}

pub fn progress(stack: &mut Stack, mut p: Ping, now: u64) -> Step {
    p.try_send(stack);
    let mut buf = [0u8; 2048];
    loop {
        let sock = stack.sockets.get_mut::<icmp::Socket>(p.sock);
        let Ok((n, from)) = sock.recv_slice(&mut buf) else {
            break;
        };
        if from != IpAddress::Ipv4(p.dest) {
            continue;
        }
        let Ok(pkt) = Icmpv4Packet::new_checked(&buf[..n]) else {
            continue;
        };
        let Ok(repr) = Icmpv4Repr::parse(&pkt, &ChecksumCapabilities::default()) else {
            continue;
        };
        if let Icmpv4Repr::EchoReply {
            ident,
            seq_no,
            data,
        } = repr
            && ident == p.ident
            && seq_no == p.seq
            && data == p.payload.as_slice()
        {
            let rtt = now.saturating_sub(p.sent_at.unwrap_or(now));
            p.cancel(stack);
            let mut w = Writer::new();
            w.u64(rtt);
            return Step::reply(Ok(w));
        }
    }
    if now >= p.deadline {
        p.cancel(stack);
        return Step::err(Error::TimedOut);
    }
    Step::Wait(Pending::Ping(p))
}
