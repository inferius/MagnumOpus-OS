//! TCP: navazani spojeni, spojeni (cteni, zapis, ukonceni zapisu),
//! naslouchani a postupne zavirani.

use alloc::collections::VecDeque;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use core::net::{IpAddr, Ipv4Addr, SocketAddr};

use libos::{Error, Result};
use netproto::auth::Usage;
use netproto::fw::audit;
use netproto::{Buffers, MAX_IO, ReadAddr, Reader, WriteAddr, Writer, op, proto, rights};
use smoltcp::iface::SocketHandle;
use smoltcp::socket::tcp::State;
use smoltcp::wire::{IpAddress, IpEndpoint};

use crate::fw::app_name;
use crate::obj::{AuthRef, Deferred, Gate, Kind, Listener, Pending, Step, Stream, ask_wait, gate};
use crate::stack::{PortProto, Stack, deadline, now_ns, v4};

/// Navazani spojeni bez limitu se po teto dobe vzda (`TIMED_OUT`).
const CONNECT_MAX_NS: u64 = 75_000_000_000;
/// Jak dlouho se po zavreni kanalu ceka na korektni ukonceni spojeni.
const CLOSE_LINGER_NS: u64 = 30_000_000_000;
/// Nejvetsi fronta naslouchani (jako `SOMAXCONN` v Linuxu).
const BACKLOG_MAX: usize = 4096;
/// Kolik socketu naraz prijima SYN: tolik soubeznych navazani spojeni
/// zvladne naslouchani mezi dvema koly sluzby. Po prijeti se doplni, takze
/// ve fronte muze cekat az `backlog` spojeni.
const LISTEN_SPARE: usize = 16;

pub fn to_sa(ep: IpEndpoint) -> SocketAddr {
    let ip = match ep.addr {
        IpAddress::Ipv4(a) => IpAddr::V4(a),
        #[allow(unreachable_patterns)]
        _ => IpAddr::V4(Ipv4Addr::UNSPECIFIED),
    };
    SocketAddr::new(ip, ep.port)
}

/// Rozpracovane navazani spojeni.
pub struct Connect {
    pub sock: SocketHandle,
    pub port: u16,
    pub remote: SocketAddr,
    pub deadline: u64,
    app: String,
    usage: Usage,
}

impl Connect {
    pub fn cancel(self, stack: &mut Stack) {
        stack.tcp(self.sock).abort();
        stack.remove(self.sock);
        stack.release_port(PortProto::Tcp, self.port);
    }
}

/// Parametry `TCP_CONNECT`.
pub struct ConnectReq {
    remote: SocketAddr,
    timeout: u64,
    buffers: Buffers,
}

/// `TCP_CONNECT` na relaci.
pub fn connect(stack: &mut Stack, auth: &AuthRef, r: &mut Reader) -> Result<Step> {
    let req = ConnectReq {
        remote: r.socket_addr()?,
        timeout: r.u64()?,
        buffers: Buffers::get(r)?,
    };
    auth.borrow().require(rights::TCP_CONNECT)?;
    connect_start(stack, auth, req, Vec::new())
}

/// Zahaji spojeni (po kontrole cile; `granted` = aplikace, kterym
/// uzivatel pozadavek povolil).
pub fn connect_start(
    stack: &mut Stack,
    auth: &AuthRef,
    req: ConnectReq,
    granted: Vec<String>,
) -> Result<Step> {
    let remote = req.remote;
    let ip = v4(remote.ip())?;
    if remote.port() == 0 || ip.is_unspecified() || ip.is_broadcast() || ip.is_multicast() {
        return Err(Error::InvalidArgs);
    }
    let gated = gate(stack, &auth.borrow(), proto::TCP, remote, &granted)?;
    if let Gate::Ask(id, app) = gated {
        return Ok(ask_wait(id, app, proto::TCP, remote, granted, Deferred::Connect(req)));
    }
    stack.local_ip()?;
    let (rx, tx) = req.buffers.resolved();
    let usage = auth.borrow().charge((rx + tx) as u64)?;
    let port = stack.take_port(PortProto::Tcp, 0)?;
    let sock = stack.new_tcp(rx, tx);
    let cx = stack.iface.context();
    let res = stack
        .sockets
        .get_mut::<smoltcp::socket::tcp::Socket>(sock)
        .connect(cx, (IpAddress::Ipv4(ip), remote.port()), port);
    if res.is_err() {
        stack.remove(sock);
        stack.release_port(PortProto::Tcp, port);
        return Err(Error::Unreachable);
    }
    let app = app_name(&auth.borrow());
    stack.fw.socket_opened(&app);
    let limit = now_ns().saturating_add(CONNECT_MAX_NS);
    Ok(Step::Wait(Pending::Connect(Connect {
        sock,
        port,
        remote,
        deadline: deadline(req.timeout).min(limit),
        app,
        usage,
    })))
}

pub fn connect_progress(stack: &mut Stack, c: Connect, now: u64) -> Step {
    let state = stack.tcp(c.sock).state();
    match state {
        State::Established | State::CloseWait => {
            let s = stack.tcp(c.sock);
            let local = s
                .local_endpoint()
                .map(to_sa)
                .unwrap_or(SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), c.port));
            let mut w = Writer::new();
            w.socket_addr(local);
            Step::Done(
                Ok(w),
                Some(Kind::Stream(Stream {
                    sock: c.sock,
                    local,
                    remote: c.remote,
                    port: Some(c.port),
                    fin_seen: state == State::CloseWait,
                    app: c.app,
                    _usage: c.usage,
                })),
            )
        }
        State::SynSent | State::SynReceived if now < c.deadline => {
            // ICMP "nedosazitelny" k nasemu SYN: port (3) = odmitnuto,
            // jinak cil nedosazitelny.
            let icmp = match c.remote.ip() {
                IpAddr::V4(ip) => stack.dev.take_icmp_error(ip, c.remote.port(), c.port),
                IpAddr::V6(_) => None,
            };
            match icmp {
                Some(code) => {
                    c.cancel(stack);
                    Step::err(if code == 3 {
                        Error::ConnectionRefused
                    } else {
                        Error::Unreachable
                    })
                }
                None => Step::Wait(Pending::Connect(c)),
            }
        }
        State::SynSent | State::SynReceived => {
            c.cancel(stack);
            Step::err(Error::TimedOut)
        }
        // Closed po SYN = RST od druhe strany (nebo ICMP nedosazitelnost).
        _ => {
            c.cancel(stack);
            Step::err(Error::ConnectionRefused)
        }
    }
}

/// Zapamatuje si, ze druha strana ukoncila zapis (pro rozliseni RST).
pub fn update_fin(stack: &mut Stack, s: &mut Stream) {
    if matches!(
        stack.tcp(s.sock).state(),
        State::CloseWait | State::LastAck | State::Closing | State::TimeWait
    ) {
        s.fin_seen = true;
    }
}

pub fn stream_request(
    stack: &mut Stack,
    s: &mut Stream,
    code: u32,
    r: &mut Reader,
) -> Result<Step> {
    match code {
        op::SEND => {
            let timeout = r.u64()?;
            let data = r.rest().to_vec();
            if data.len() > MAX_IO {
                return Err(Error::OutOfRange);
            }
            Ok(send_progress(stack, s, data, 0, deadline(timeout)))
        }
        op::RECV => {
            let max = (r.u32()? as usize).clamp(1, MAX_IO);
            let timeout = r.u64()?;
            Ok(recv_progress(stack, s, max, deadline(timeout), now_ns()))
        }
        op::SHUTDOWN => {
            stack.tcp(s.sock).close();
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

pub fn stream_progress(stack: &mut Stack, s: &mut Stream, p: Pending, now: u64) -> Step {
    match p {
        Pending::Send {
            data,
            done,
            deadline,
        } => {
            if now >= deadline {
                let mut w = Writer::new();
                w.u32(done as u32);
                return Step::reply(Ok(w));
            }
            send_progress(stack, s, data, done, deadline)
        }
        Pending::Recv { max, deadline } => recv_progress(stack, s, max, deadline, now),
        _ => Step::err(Error::BadState),
    }
}

/// Chyba spojeni, ktere uz nejde pouzit.
fn dead_stream_error(stack: &mut Stack, s: &Stream) -> Error {
    if stack.tcp(s.sock).state() == State::Closed && !s.fin_seen {
        Error::ConnectionReset
    } else {
        // Zapis po vlastnim `SHUTDOWN`.
        Error::BadState
    }
}

fn send_progress(
    stack: &mut Stack,
    s: &mut Stream,
    data: Vec<u8>,
    mut done: usize,
    deadline: u64,
) -> Step {
    update_fin(stack, s);
    let sock = stack.tcp(s.sock);
    if !sock.may_send() {
        if done > 0 {
            let mut w = Writer::new();
            w.u32(done as u32);
            return Step::reply(Ok(w));
        }
        return Step::err(dead_stream_error(stack, s));
    }
    if let Ok(n) = sock.send_slice(&data[done..]) {
        done += n;
        stack.fw.traffic(&s.app, n, 0);
    }
    if done >= data.len() {
        let mut w = Writer::new();
        w.u32(done as u32);
        return Step::reply(Ok(w));
    }
    Step::Wait(Pending::Send {
        data,
        done,
        deadline,
    })
}

fn recv_progress(stack: &mut Stack, s: &mut Stream, max: usize, deadline: u64, now: u64) -> Step {
    update_fin(stack, s);
    let sock = stack.tcp(s.sock);
    if sock.can_recv() {
        let mut buf = vec![0u8; max];
        return match sock.recv_slice(&mut buf) {
            Ok(n) => {
                buf.truncate(n);
                stack.fw.traffic(&s.app, 0, n);
                let mut w = Writer::new();
                w.bytes(&buf);
                Step::reply(Ok(w))
            }
            Err(_) => Step::err(dead_stream_error(stack, s)),
        };
    }
    if !sock.may_recv() {
        // Zadna dalsi data neprijdou: konec proudu, nebo preruseni.
        if sock.state() == State::Closed && !s.fin_seen {
            return Step::err(Error::ConnectionReset);
        }
        return Step::reply(Ok(Writer::new()));
    }
    if now >= deadline {
        return Step::err(Error::TimedOut);
    }
    Step::Wait(Pending::Recv { max, deadline })
}

/// Kanal spojeni zavren klientem: korektni ukonceni (FIN) a uklid az po
/// dobehnuti nebo po `CLOSE_LINGER_NS`. Rozpocet relace se vrati hned
/// (strzeni zanikne se `Stream`), dobihajici socket uz klient nevidi.
pub fn close_stream(stack: &mut Stack, s: Stream) {
    stack.tcp(s.sock).close();
    stack
        .closing
        .push((s.sock, now_ns().saturating_add(CLOSE_LINGER_NS), s.port));
}

/// Uklid zavrenych spojeni.
pub fn reap_closing(stack: &mut Stack) {
    let now = now_ns();
    let mut i = 0;
    while i < stack.closing.len() {
        let (h, deadline, port) = stack.closing[i];
        let state = stack.tcp(h).state();
        if matches!(state, State::Closed | State::TimeWait) || now >= deadline {
            if state != State::Closed {
                stack.tcp(h).abort();
            }
            stack.remove(h);
            if let Some(p) = port {
                stack.release_port(PortProto::Tcp, p);
            }
            stack.closing.swap_remove(i);
        } else {
            i += 1;
        }
    }
}

/// `TCP_LISTEN` na relaci.
pub fn listen(stack: &mut Stack, auth: &AuthRef, r: &mut Reader) -> Result<Step> {
    let port = r.u16()?;
    let backlog = (r.u16()? as usize).clamp(1, BACKLOG_MAX);
    let buffers = Buffers::get(r)?;
    auth.borrow().require(rights::TCP_LISTEN)?;
    if !auth.borrow().allows_bind(port) {
        return Err(Error::AccessDenied);
    }
    let (rx, tx) = buffers.resolved();
    let spare = backlog.min(LISTEN_SPARE);
    let usage = auth.borrow().charge((spare * (rx + tx)) as u64)?;
    let port = stack.take_port(PortProto::Tcp, port)?;
    let mut listening = Vec::new();
    for _ in 0..spare {
        match listening_socket(stack, port, rx, tx) {
            Ok(h) => listening.push(h),
            Err(e) => {
                for h in listening {
                    stack.remove(h);
                }
                stack.release_port(PortProto::Tcp, port);
                return Err(e);
            }
        }
    }
    stack.listen_started(port);
    let app = app_name(&auth.borrow());
    stack.fw.record(audit::LISTEN, &app, proto::TCP, None, port, "");
    let local = stack.local_addr(port);
    let mut w = Writer::new();
    w.socket_addr(local);
    Ok(Step::Done(
        Ok(w),
        Some(Kind::Listener(Listener {
            port,
            listening,
            ready: VecDeque::new(),
            backlog,
            rx,
            tx,
            auth: auth.clone(),
            app,
            _usage: usage,
        })),
    ))
}

fn listening_socket(stack: &mut Stack, port: u16, rx: usize, tx: usize) -> Result<SocketHandle> {
    let h = stack.new_tcp(rx, tx);
    if stack.tcp(h).listen(port).is_err() {
        stack.remove(h);
        return Err(Error::InvalidArgs);
    }
    Ok(h)
}

pub fn listener_request(
    stack: &mut Stack,
    l: &mut Listener,
    code: u32,
    r: &mut Reader,
) -> Result<Step> {
    match code {
        op::ACCEPT => {
            let timeout = r.u64()?;
            // Hned se zkusi v `progress` (main), tady jen zalozit.
            Ok(Step::Wait(Pending::Accept {
                deadline: deadline(timeout),
            }))
        }
        op::LOCAL_ADDR => {
            let mut w = Writer::new();
            w.socket_addr(stack.local_addr(l.port));
            Ok(Step::reply(Ok(w)))
        }
        _ => Err(Error::NotSupported),
    }
}

/// Presune navazana spojeni do fronty `ready` a doplni naslouchajici
/// sockety (dokud fronta nedosahne `backlog`).
fn refill(stack: &mut Stack, l: &mut Listener) {
    let mut i = 0;
    while i < l.listening.len() {
        let h = l.listening[i];
        match stack.tcp(h).state() {
            State::Listen | State::SynReceived => i += 1,
            State::Closed => {
                // Spojeni zrusene pred prijetim: znovu naslouchat.
                let _ = stack.tcp(h).listen(l.port);
                i += 1;
            }
            _ => {
                l.listening.swap_remove(i);
                l.ready.push_back(h);
            }
        }
    }
    let want = LISTEN_SPARE.min(l.backlog.saturating_sub(l.ready.len()));
    while l.listening.len() < want {
        match listening_socket(stack, l.port, l.rx, l.tx) {
            Ok(h) => l.listening.push(h),
            Err(_) => break,
        }
    }
}

pub fn accept_progress(stack: &mut Stack, l: &mut Listener, deadline: u64, now: u64) -> Step {
    refill(stack, l);
    while let Some(h) = l.ready.pop_front() {
        // Prijate spojeni ma vlastni strzeni (naslouchani drzi jen sve
        // sockety); bez rozpoctu se spojeni zrusi.
        let usage = match l.auth.borrow().charge((l.rx + l.tx) as u64) {
            Ok(u) => u,
            Err(_) => {
                stack.tcp(h).abort();
                stack.remove(h);
                continue;
            }
        };
        let sock = stack.tcp(h);
        let state = sock.state();
        let remote = sock.remote_endpoint().map(to_sa);
        let local = sock
            .local_endpoint()
            .map(to_sa)
            .unwrap_or_else(|| stack.local_addr(l.port));
        let Some(remote) = remote else {
            stack.remove(h);
            continue;
        };
        refill(stack, l);
        stack.fw.socket_opened(&l.app);
        let mut w = Writer::new();
        w.socket_addr(remote).socket_addr(local);
        return Step::Done(
            Ok(w),
            Some(Kind::Stream(Stream {
                sock: h,
                local,
                remote,
                port: None,
                fin_seen: matches!(
                    state,
                    State::CloseWait | State::LastAck | State::TimeWait | State::Closing
                ),
                app: l.app.clone(),
                _usage: usage,
            })),
        );
    }
    if now >= deadline {
        return Step::err(Error::TimedOut);
    }
    Step::Wait(Pending::Accept { deadline })
}

pub fn close_listener(stack: &mut Stack, l: Listener) {
    for h in l.listening.into_iter().chain(l.ready) {
        stack.tcp(h).abort();
        stack.remove(h);
    }
    stack.listen_stopped(l.port);
    stack.release_port(PortProto::Tcp, l.port);
}
