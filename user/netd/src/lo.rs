//! Loopback (127.0.0.0/8 a ::1) bez linky a bez zasobniku smoltcp
//! (`docs/cs/sit.md`, kapitola Loopback).
//!
//! - TCP: `TCP_CONNECT` na loopback najde naslouchani v registru
//!   ([`Loopback::listeners`], klic rodina + port) a spoji obe strany
//!   parem front bajtu ([`Conn`]); spojeni ceka ve fronte naslouchani na
//!   `ACCEPT` jako SYN. Proudy maji stejne operace (`SEND`, `RECV`,
//!   `SHUTDOWN`, `ADDRS`, pripravenost) jako spojeni pres sit.
//! - UDP: `SEND_TO` na loopback vlozi datagram do fronty socketu z registru
//!   ([`Loopback::udp`]); plna fronta datagram zahodi (jako sit).
//! - Opravneni: stejna jako u site (prava relace, pravidla cilu `gate`,
//!   rozpocet socketu a bajtu, `allows_bind`); porty sdili prostor se
//!   sitovymi sockety (`Stack::take_port`).
//! - Naslouchani na `0.0.0.0` prijima ze site i z loopbacku, na 127.x jen
//!   z loopbacku; IPv6 je zatim jen loopback (`::` a `::1`).
//!
//! Zmena stavu (data, nove spojeni, zavreni) nastavi `Stack::lo_dirty`:
//! smycka sluzby pak hned udela dalsi kolo (protejsek muze cekat).

use alloc::collections::{BTreeMap, VecDeque};
use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;
use core::cell::RefCell;
use core::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};

use libos::{Error, Result};
use netproto::auth::Usage;
use netproto::{MAX_DATAGRAM, MAX_IO, Reader, WriteAddr, Writer, op};

use crate::obj::{Kind, Pending, Step};
use crate::stack::{PortProto, Stack, expired};

/// Nejvic bajtu ve fronte socketu UDP na loopbacku.
const UDP_QUEUE_BYTES: usize = 256 * 1024;

/// Je adresa loopback (127.0.0.0/8, ::1)?
pub fn is_loopback(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(a) => a.is_loopback(),
        IpAddr::V6(a) => a.is_loopback(),
    }
}

/// Adresa loopbacku rodiny (`127.0.0.1`, `::1`).
pub fn local_ip(v6: bool) -> IpAddr {
    if v6 { IpAddr::V6(Ipv6Addr::LOCALHOST) } else { IpAddr::V4(Ipv4Addr::LOCALHOST) }
}

/// Kam socket navazany na adresu prijima.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Bind {
    pub port: u16,
    /// Sit (rozhrani smoltcp): `0.0.0.0` nebo adresa rozhrani.
    pub net: bool,
    /// Loopback: adresa navazani (`0.0.0.0`, `::` = kazda adresa loopbacku
    /// rodiny), nebo `None` (jen sit).
    pub lo: Option<IpAddr>,
}

impl Bind {
    pub fn v6(&self) -> bool {
        matches!(self.lo, Some(IpAddr::V6(_)))
    }

    /// Mistni adresa pro klienta: sit = adresa rozhrani, jinak loopback.
    pub fn local(&self, stack: &Stack) -> SocketAddr {
        match self.lo {
            Some(ip) if !self.net => SocketAddr::new(if ip.is_unspecified() { local_ip(self.v6()) } else { ip }, self.port),
            _ => stack.local_addr(self.port),
        }
    }
}

/// Rozdeleni adresy navazani: `0.0.0.0` = sit i loopback, 127.x a `::1`/`::`
/// = jen loopback, adresa rozhrani = jen sit, jinak `ADDRESS_NOT_AVAILABLE`.
pub fn bind_of(stack: &Stack, addr: SocketAddr) -> Result<Bind> {
    let port = addr.port();
    match addr.ip() {
        IpAddr::V4(ip) if ip.is_unspecified() => Ok(Bind { port, net: true, lo: Some(addr.ip()) }),
        IpAddr::V4(ip) if ip.is_loopback() => Ok(Bind { port, net: false, lo: Some(addr.ip()) }),
        IpAddr::V4(ip) if stack.local_ip().ok() == Some(ip) => Ok(Bind { port, net: true, lo: None }),
        IpAddr::V6(ip) if ip.is_unspecified() || ip.is_loopback() => Ok(Bind { port, net: false, lo: Some(addr.ip()) }),
        _ => Err(Error::AddressNotAvailable),
    }
}

/// Smeruje navazani `bound` na cil `to` (stejna rodina, kazda adresa nebo
/// presne ta)?
fn matches(bound: IpAddr, to: IpAddr) -> bool {
    bound.is_ipv6() == to.is_ipv6() && (bound.is_unspecified() || bound == to)
}

/// Registr loopbacku ve `Stack`.
#[derive(Default)]
pub struct Loopback {
    /// Naslouchani TCP: (IPv6, port) -> fronta.
    pub listeners: BTreeMap<(bool, u16), AcceptRef>,
    /// Sockety UDP: (IPv6, port) -> fronta.
    pub udp: BTreeMap<(bool, u16), UdpRef>,
}

/// Jeden smer spojeni.
struct Pipe {
    buf: VecDeque<u8>,
    cap: usize,
    /// Zapisujici strana ukoncila zapis (`SHUTDOWN` nebo zavreni).
    shut: bool,
}

impl Pipe {
    fn new(cap: usize) -> Self {
        Self { buf: VecDeque::new(), cap: cap.max(1), shut: false }
    }
}

/// Spojeni na loopbacku: `pipes[s]` = data, ktera pise strana `s`.
pub struct Conn {
    pipes: [Pipe; 2],
    /// Strana zavrela kanal (protejsek pak dostane konec proudu, zapis do
    /// ni = `CONNECTION_RESET`).
    gone: [bool; 2],
    /// Naslouchani zaniklo pred prijetim (klient = `CONNECTION_RESET`).
    reset: bool,
}

pub type ConnRef = Rc<RefCell<Conn>>;

/// Spojeni cekajici na `ACCEPT`.
struct Incoming {
    conn: ConnRef,
    /// Adresa klienta (127.0.0.1 nebo ::1 s jeho portem).
    client: SocketAddr,
    /// Cil, na ktery se klient pripojoval.
    target: SocketAddr,
}

pub struct Accept {
    bound: IpAddr,
    queue: VecDeque<Incoming>,
    backlog: usize,
    /// Buffer prijmu prijatych spojeni.
    rx: usize,
}

pub type AcceptRef = Rc<RefCell<Accept>>;

pub struct UdpQueue {
    bound: IpAddr,
    queue: VecDeque<(SocketAddr, Vec<u8>)>,
    bytes: usize,
}

pub type UdpRef = Rc<RefCell<UdpQueue>>;

/// Strana spojeni na loopbacku (`Kind::LoStream`).
pub struct LoStream {
    conn: ConnRef,
    side: usize,
    pub local: SocketAddr,
    pub remote: SocketAddr,
    /// Docasny port klienta (u prijatych zadny: patri naslouchani).
    port: Option<u16>,
    pub app: String,
    pub _usage: Usage,
}

// --- naslouchani ---

/// Zaregistruje naslouchani TCP na loopbacku (port uz je obsazeny).
pub fn listen(stack: &mut Stack, bound: IpAddr, port: u16, backlog: usize, rx: usize) -> AcceptRef {
    let a = Rc::new(RefCell::new(Accept { bound, queue: VecDeque::new(), backlog, rx }));
    stack.lo.listeners.insert((bound.is_ipv6(), port), a.clone());
    a
}

/// Konec naslouchani: cekajici spojeni dostanou `CONNECTION_RESET`.
pub fn unlisten(stack: &mut Stack, v6: bool, port: u16, a: &AcceptRef) {
    stack.lo.listeners.remove(&(v6, port));
    for i in a.borrow_mut().queue.drain(..) {
        i.conn.borrow_mut().reset = true;
    }
    stack.lo_dirty = true;
}

pub fn has_incoming(a: &AcceptRef) -> bool {
    !a.borrow().queue.is_empty()
}

/// Prijme cekajici spojeni: odpoved `ACCEPT` a strana serveru.
pub fn accept(a: &AcceptRef, app: &str, usage: Usage) -> Option<(Writer, Kind)> {
    let i = a.borrow_mut().queue.pop_front()?;
    let mut w = Writer::new();
    w.socket_addr(i.client).socket_addr(i.target);
    Some((
        w,
        Kind::LoStream(LoStream { conn: i.conn, side: 1, local: i.target, remote: i.client, port: None, app: String::from(app), _usage: usage }),
    ))
}

// --- spojeni ---

/// `TCP_CONNECT` na loopback (po kontrole cile a strzeni z rozpoctu).
pub fn connect(stack: &mut Stack, target: SocketAddr, rx: usize, app: String, usage: Usage) -> Result<Step> {
    let v6 = target.is_ipv6();
    let Some(a) = stack.lo.listeners.get(&(v6, target.port())).cloned() else {
        return Err(Error::ConnectionRefused);
    };
    if !matches(a.borrow().bound, target.ip()) || a.borrow().queue.len() >= a.borrow().backlog {
        return Err(Error::ConnectionRefused);
    }
    let port = stack.take_port(PortProto::Tcp, 0)?;
    let srv_rx = a.borrow().rx;
    // pipes[0] = klient -> server (prijem serveru), pipes[1] = opacne.
    let conn = Rc::new(RefCell::new(Conn { pipes: [Pipe::new(srv_rx), Pipe::new(rx)], gone: [false; 2], reset: false }));
    let client = SocketAddr::new(local_ip(v6), port);
    a.borrow_mut().queue.push_back(Incoming { conn: conn.clone(), client, target });
    stack.lo_dirty = true;
    stack.fw.socket_opened(&app);
    let mut w = Writer::new();
    w.socket_addr(client);
    Ok(Step::Done(Ok(w), Some(Kind::LoStream(LoStream { conn, side: 0, local: client, remote: target, port: Some(port), app, _usage: usage }))))
}

pub fn stream_request(stack: &mut Stack, s: &mut LoStream, code: u32, until: u64, r: &mut Reader) -> Result<Step> {
    match code {
        op::SEND => {
            let data = r.rest().to_vec();
            if data.len() > MAX_IO {
                return Err(Error::OutOfRange);
            }
            Ok(send_progress(stack, s, data, 0, until, crate::stack::now_ns()))
        }
        op::RECV => {
            let max = (r.u32()? as usize).clamp(1, MAX_IO);
            let peek = r.u8().unwrap_or(0) & op::RECV_PEEK != 0;
            Ok(recv_progress(stack, s, max, until, crate::stack::now_ns(), peek))
        }
        op::SHUTDOWN => {
            s.conn.borrow_mut().pipes[s.side].shut = true;
            stack.lo_dirty = true;
            Ok(Step::ok())
        }
        op::ADDRS => {
            let mut w = Writer::new();
            w.socket_addr(s.local).socket_addr(s.remote);
            Ok(Step::reply(Ok(w)))
        }
        _ => Err(Error::NotSupported),
    }
}

pub fn stream_progress(stack: &mut Stack, s: &mut LoStream, p: Pending, now: u64) -> Step {
    match p {
        Pending::Send { data, done, deadline } => send_progress(stack, s, data, done, deadline, now),
        Pending::Recv { max, deadline, peek } => recv_progress(stack, s, max, deadline, now, peek),
        _ => Step::err(Error::BadState),
    }
}

fn sent(done: usize) -> Step {
    let mut w = Writer::new();
    w.u32(done as u32);
    Step::reply(Ok(w))
}

fn send_progress(stack: &mut Stack, s: &mut LoStream, data: Vec<u8>, mut done: usize, deadline: u64, now: u64) -> Step {
    let peer = 1 - s.side;
    {
        let mut c = s.conn.borrow_mut();
        if c.reset || c.gone[peer] {
            return if done > 0 { sent(done) } else { Step::err(Error::ConnectionReset) };
        }
        let pipe = &mut c.pipes[s.side];
        if pipe.shut {
            return if done > 0 { sent(done) } else { Step::err(Error::BadState) };
        }
        let n = (pipe.cap.saturating_sub(pipe.buf.len())).min(data.len() - done);
        if n > 0 {
            pipe.buf.extend(&data[done..done + n]);
            done += n;
            stack.lo_dirty = true;
            stack.fw.traffic(&s.app, n, 0);
        }
    }
    if done >= data.len() {
        return sent(done);
    }
    if now >= deadline {
        if done == 0 && deadline == netproto::TRY {
            return Step::err(Error::ShouldWait);
        }
        return if done > 0 { sent(done) } else { Step::err(expired(deadline)) };
    }
    Step::Wait(Pending::Send { data, done, deadline })
}

fn recv_progress(stack: &mut Stack, s: &mut LoStream, max: usize, deadline: u64, now: u64, peek: bool) -> Step {
    let peer = 1 - s.side;
    let mut c = s.conn.borrow_mut();
    if c.reset {
        return Step::err(Error::ConnectionReset);
    }
    let pipe = &mut c.pipes[peer];
    if !pipe.buf.is_empty() {
        let n = pipe.buf.len().min(max);
        let data: Vec<u8> = if peek {
            pipe.buf.iter().take(n).copied().collect()
        } else {
            stack.lo_dirty = true;
            stack.fw.traffic(&s.app, 0, n);
            pipe.buf.drain(..n).collect()
        };
        let mut w = Writer::new();
        w.bytes(&data);
        return Step::reply(Ok(w));
    }
    // Konec proudu: protejsek ukoncil zapis nebo zavrel kanal.
    if pipe.shut || c.gone[peer] {
        return Step::reply(Ok(Writer::new()));
    }
    if now >= deadline {
        return Step::err(expired(deadline));
    }
    Step::Wait(Pending::Recv { max, deadline, peek })
}

/// Pripravenost strany spojeni: (cteni, zapis, konec, chyba).
pub fn stream_ready(s: &LoStream) -> (bool, bool, bool, bool) {
    let c = s.conn.borrow();
    let peer = 1 - s.side;
    let inbound = &c.pipes[peer];
    let outbound = &c.pipes[s.side];
    let eof = inbound.shut || c.gone[peer];
    let readable = !inbound.buf.is_empty() || eof || c.reset;
    let writable = outbound.buf.len() < outbound.cap || c.gone[peer] || c.reset;
    (readable, writable, eof || c.reset, c.reset)
}

/// Kanal strany zavren: protejsek dostane konec proudu.
pub fn close_stream(stack: &mut Stack, s: LoStream) {
    {
        let mut c = s.conn.borrow_mut();
        c.gone[s.side] = true;
        c.pipes[s.side].shut = true;
    }
    if let Some(p) = s.port {
        stack.release_port(PortProto::Tcp, p);
    }
    stack.lo_dirty = true;
}

// --- UDP ---

/// Zaregistruje socket UDP na loopbacku (port uz je obsazeny).
pub fn udp_bind(stack: &mut Stack, bound: IpAddr, port: u16) -> UdpRef {
    let q = Rc::new(RefCell::new(UdpQueue { bound, queue: VecDeque::new(), bytes: 0 }));
    stack.lo.udp.insert((bound.is_ipv6(), port), q.clone());
    q
}

pub fn udp_unbind(stack: &mut Stack, v6: bool, port: u16) {
    stack.lo.udp.remove(&(v6, port));
}

/// Datagram na loopback od socketu na portu `from_port`. Cil bez socketu
/// nebo s plnou frontou: zahozeno (jako sit).
pub fn udp_send(stack: &mut Stack, from_port: u16, to: SocketAddr, data: Vec<u8>) {
    if data.len() > MAX_DATAGRAM {
        return;
    }
    let Some(q) = stack.lo.udp.get(&(to.is_ipv6(), to.port())).cloned() else { return };
    let mut q = q.borrow_mut();
    if !matches(q.bound, to.ip()) || q.bytes + data.len() > UDP_QUEUE_BYTES {
        return;
    }
    q.bytes += data.len();
    q.queue.push_back((SocketAddr::new(local_ip(to.is_ipv6()), from_port), data));
    stack.lo_dirty = true;
}

/// Prvni datagram z loopbacku: (odesilatel, data).
pub fn udp_recv(stack: &mut Stack, q: &UdpRef) -> Option<(SocketAddr, Vec<u8>)> {
    let mut q = q.borrow_mut();
    let (from, data) = q.queue.pop_front()?;
    q.bytes -= data.len();
    stack.lo_dirty = true;
    Some((from, data))
}

pub fn udp_has_data(q: &UdpRef) -> bool {
    !q.borrow().queue.is_empty()
}
