//! Sockety pro cisla fd (BSD sockety knihovny `cstd`) nad relaci site.
//!
//! Socket z `socket()` jeste nema spojeni u netd: `bind`, `listen`,
//! `connect` ho prevedou na naslouchani, spojeni TCP nebo socket UDP.
//! Objekty netproto jsou `Send + Sync` s `&self`, proto stav drzi jen
//! zamek prechodu a I/O bezi bez nej (cteni a zapis z ruznych vlaken).
//! Neblokujici rezim = termin `Deadline::TRY` (`ShouldWait` -> `EAGAIN`).

use alloc::sync::Arc;
use alloc::vec::Vec;
use core::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use core::time::Duration;

use libos::sync::Mutex;
use netproto::{Buffers, Deadline, Readiness, TcpListener, TcpStream, UdpSocket};

use crate::{Error, Result};

/// Druh socketu (`SOCK_STREAM`, `SOCK_DGRAM`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Stream,
    Datagram,
}

enum State {
    /// Po `socket()` (pripadne s adresou z `bind` u TCP).
    New { bound: Option<SocketAddr> },
    Stream(Arc<TcpStream>),
    Listener(Arc<TcpListener>),
    /// UDP s cilem po `connect` (filtr prijmu a vychozi cil odeslani).
    Udp(Arc<UdpSocket>, Option<SocketAddr>),
}

/// Socket v tabulce fd.
pub struct Socket {
    pub kind: Kind,
    pub v6: bool,
    state: Mutex<State>,
    /// Limity `SO_RCVTIMEO` / `SO_SNDTIMEO` (`None` = bez limitu).
    timeouts: Mutex<(Option<Duration>, Option<Duration>)>,
    /// Pripravenost (event netd), az socket ma kanal.
    ready: Mutex<Option<Arc<Readiness>>>,
}

fn net() -> Result<netproto::Net> {
    crate::net::session_net()
}

fn any(v6: bool) -> SocketAddr {
    let ip = if v6 { IpAddr::V6(Ipv6Addr::UNSPECIFIED) } else { IpAddr::V4(Ipv4Addr::UNSPECIFIED) };
    SocketAddr::new(ip, 0)
}

fn deadline(nonblock: bool, t: Option<Duration>) -> Deadline {
    if nonblock { Deadline::TRY } else { Deadline::after(t) }
}

impl Socket {
    pub fn new(kind: Kind, v6: bool) -> Self {
        Socket {
            kind,
            v6,
            state: Mutex::new(State::New { bound: None }),
            timeouts: Mutex::new((None, None)),
            ready: Mutex::new(None),
        }
    }

    /// Export pro jiny proces (`SCM_RIGHTS`, dedeni v `posix_spawn`):
    /// (druh `abi::params::fd_kind`, data, handle). Socket se spojenim dostane
    /// novy kanal ke stejnemu socketu u netd (`DUP`).
    pub(crate) fn export(&self) -> Result<(u8, Vec<u8>, Option<libos::Handle>)> {
        use abi::params::fd_kind;
        let mut w = netproto::Writer::new();
        w.u8(if self.kind == Kind::Stream { 1 } else { 2 }).u8(u8::from(self.v6));
        let st = self.state.lock();
        let (kind, ch) = match &*st {
            State::New { bound } => {
                match bound {
                    Some(a) => {
                        w.u8(1);
                        netproto::WriteAddr::socket_addr(&mut w, *a);
                    }
                    None => {
                        w.u8(0);
                    }
                }
                (fd_kind::SOCKET_NEW, None)
            }
            State::Stream(s) => {
                w.u8(1);
                (fd_kind::SOCKET, Some(s.dup()?))
            }
            State::Listener(l) => {
                w.u8(2);
                (fd_kind::SOCKET, Some(l.dup()?))
            }
            State::Udp(u, peer) => {
                w.u8(3);
                match peer {
                    Some(p) => {
                        w.u8(1);
                        netproto::WriteAddr::socket_addr(&mut w, *p);
                    }
                    None => {
                        w.u8(0);
                    }
                }
                (fd_kind::SOCKET, Some(u.dup()?))
            }
        };
        Ok((kind, w.finish(), ch.map(|c| c.into_handle())))
    }

    /// Socket od jineho procesu (viz [`Socket::export`]).
    pub(crate) fn import(kind: u8, data: &[u8], handle: Option<libos::Handle>) -> Result<Self> {
        use abi::params::fd_kind;
        use netproto::ReadAddr;
        let mut r = netproto::Reader::new(data);
        let k = if r.u8()? == 1 { Kind::Stream } else { Kind::Datagram };
        let v6 = r.u8()? != 0;
        let me = Socket::new(k, v6);
        let st = match kind {
            fd_kind::SOCKET_NEW => State::New { bound: if r.u8()? != 0 { Some(r.socket_addr()?) } else { None } },
            fd_kind::SOCKET => {
                let ch = libos::Channel::from_handle(handle.ok_or(Error::Corrupted)?);
                match r.u8()? {
                    1 => State::Stream(Arc::new(TcpStream::from_channel(ch)?)),
                    2 => State::Listener(Arc::new(TcpListener::from_channel(ch))),
                    3 => {
                        let peer = if r.u8()? != 0 { Some(r.socket_addr()?) } else { None };
                        State::Udp(Arc::new(UdpSocket::from_channel(ch)), peer)
                    }
                    _ => return Err(Error::Corrupted),
                }
            }
            _ => return Err(Error::NotSupported),
        };
        *me.state.lock() = st;
        Ok(me)
    }

    /// Prijate spojeni z `accept`.
    fn from_stream(s: TcpStream, v6: bool) -> Self {
        let me = Socket::new(Kind::Stream, v6);
        *me.state.lock() = State::Stream(Arc::new(s));
        me
    }

    pub fn bind(&self, addr: SocketAddr) -> Result<()> {
        let mut st = self.state.lock();
        let State::New { bound } = &mut *st else { return Err(Error::InvalidArgs) };
        if bound.is_some() {
            return Err(Error::InvalidArgs);
        }
        match self.kind {
            // TCP: adresu si pamatuje, kanal vznikne az pri listen/connect.
            Kind::Stream => *bound = Some(addr),
            Kind::Datagram => {
                let u = net()?.udp_bind_addr(addr, Buffers::DEFAULT)?;
                *st = State::Udp(Arc::new(u), None);
            }
        }
        Ok(())
    }

    pub fn listen(&self, backlog: u16) -> Result<()> {
        let mut st = self.state.lock();
        match &*st {
            State::Listener(_) => return Ok(()),
            State::New { bound } if self.kind == Kind::Stream => {
                let addr = bound.unwrap_or_else(|| any(self.v6));
                let l = net()?.tcp_listen_addr(addr, backlog.max(1), Buffers::DEFAULT)?;
                *st = State::Listener(Arc::new(l));
            }
            _ => return Err(Error::InvalidArgs),
        }
        Ok(())
    }

    /// `connect`: TCP spojeni (blokuje i v neblokujicim rezimu - POSIX
    /// dovoluje dokonceni hned), UDP jen zapamatuje cil.
    pub fn connect(&self, to: SocketAddr, nonblock: bool) -> Result<()> {
        let _ = nonblock;
        if self.kind == Kind::Datagram {
            let mut st = self.state.lock();
            if let State::New { .. } = &*st {
                let u = net()?.udp_bind_addr(any(self.v6), Buffers::DEFAULT)?;
                *st = State::Udp(Arc::new(u), None);
            }
            if let State::Udp(_, peer) = &mut *st {
                *peer = Some(to);
            }
            return Ok(());
        }
        {
            let st = self.state.lock();
            match &*st {
                State::New { .. } => {}
                State::Stream(_) => return Err(Error::AlreadyExists),
                _ => return Err(Error::InvalidArgs),
            }
        }
        let t = self.timeouts.lock().1;
        let s = net()?.tcp_connect_ctx(to, Deadline::after(t), Buffers::DEFAULT, None)?;
        *self.state.lock() = State::Stream(Arc::new(s));
        Ok(())
    }

    pub fn accept(&self, nonblock: bool) -> Result<(Socket, SocketAddr)> {
        let l = match &*self.state.lock() {
            State::Listener(l) => l.clone(),
            _ => return Err(Error::InvalidArgs),
        };
        let (s, from) = l.accept_ctx(deadline(nonblock, self.timeouts.lock().0), None)?;
        Ok((Socket::from_stream(s, self.v6), from))
    }

    fn stream(&self) -> Result<Arc<TcpStream>> {
        match &*self.state.lock() {
            State::Stream(s) => Ok(s.clone()),
            _ => Err(Error::BadState),
        }
    }

    fn udp(&self) -> Result<(Arc<UdpSocket>, Option<SocketAddr>)> {
        match &*self.state.lock() {
            State::Udp(u, p) => Ok((u.clone(), *p)),
            _ => Err(Error::BadState),
        }
    }

    /// Cteni (`recv`, `read`); 0 = konec proudu.
    pub fn recv(&self, buf: &mut [u8], nonblock: bool) -> Result<usize> {
        let d = deadline(nonblock, self.timeouts.lock().0);
        match self.kind {
            Kind::Stream => match self.stream()?.recv_ctx(buf, d, None) {
                Err(Error::PeerClosed) => Ok(0),
                r => r,
            },
            Kind::Datagram => {
                let (u, peer) = self.udp()?;
                loop {
                    let (n, from) = u.recv_from_ctx(buf, d, None)?;
                    if peer.is_none_or(|p| p == from) {
                        return Ok(n);
                    }
                }
            }
        }
    }

    pub fn send(&self, data: &[u8], nonblock: bool) -> Result<usize> {
        match self.kind {
            Kind::Stream => {
                let d = deadline(nonblock, self.timeouts.lock().1);
                self.stream()?.send_ctx(data, d)
            }
            Kind::Datagram => {
                let (u, peer) = self.udp()?;
                let to = peer.ok_or(Error::BadState)?;
                u.send_to(data, to).map(|_| data.len())
            }
        }
    }

    pub fn recv_from(&self, buf: &mut [u8], nonblock: bool) -> Result<(usize, SocketAddr)> {
        if self.kind == Kind::Stream {
            let n = self.recv(buf, nonblock)?;
            return Ok((n, self.peer_addr()?));
        }
        let d = deadline(nonblock, self.timeouts.lock().0);
        self.udp()?.0.recv_from_ctx(buf, d, None)
    }

    pub fn send_to(&self, data: &[u8], to: SocketAddr) -> Result<usize> {
        if self.kind == Kind::Stream {
            return self.send(data, false);
        }
        // UDP bez bind: automaticky na libovolny port (jako POSIX).
        {
            let mut st = self.state.lock();
            if let State::New { .. } = &*st {
                let u = net()?.udp_bind_addr(any(self.v6), Buffers::DEFAULT)?;
                *st = State::Udp(Arc::new(u), None);
            }
        }
        self.udp()?.0.send_to(data, to).map(|_| data.len())
    }

    pub fn shutdown_write(&self) -> Result<()> {
        self.stream()?.shutdown_write()
    }

    pub fn local_addr(&self) -> Result<SocketAddr> {
        match &*self.state.lock() {
            State::New { bound } => Ok(bound.unwrap_or_else(|| any(self.v6))),
            State::Stream(s) => Ok(s.local_addr()),
            State::Listener(l) => l.local_addr(),
            State::Udp(u, _) => u.local_addr(),
        }
    }

    pub fn peer_addr(&self) -> Result<SocketAddr> {
        match &*self.state.lock() {
            State::Stream(s) => Ok(s.peer_addr()),
            State::Udp(_, Some(p)) => Ok(*p),
            _ => Err(Error::BadState),
        }
    }

    pub fn set_timeouts(&self, read: Option<Option<Duration>>, write: Option<Option<Duration>>) {
        let mut t = self.timeouts.lock();
        if let Some(r) = read {
            t.0 = r;
        }
        if let Some(w) = write {
            t.1 = w;
        }
    }

    pub fn timeouts(&self) -> (Option<Duration>, Option<Duration>) {
        *self.timeouts.lock()
    }

    /// Je socket naslouchani (`SO_ACCEPTCONN`)?
    pub fn is_listening(&self) -> bool {
        matches!(&*self.state.lock(), State::Listener(_))
    }

    /// Event pripravenosti (bity `netproto::ready`); `None` = socket jeste
    /// nema kanal (po `socket()` bez `connect`/`listen`/`bind`).
    pub fn readiness(&self) -> Result<Option<Arc<Readiness>>> {
        let mut r = self.ready.lock();
        if let Some(x) = r.as_ref() {
            return Ok(Some(x.clone()));
        }
        let new = match &*self.state.lock() {
            State::New { .. } => return Ok(None),
            State::Stream(s) => s.readiness()?,
            State::Listener(l) => l.readiness()?,
            State::Udp(u, _) => u.readiness()?,
        };
        let a = Arc::new(new);
        *r = Some(a.clone());
        Ok(Some(a))
    }
}

/// Preklad jmena (`getaddrinfo`).
pub fn resolve(host: &str) -> Result<Vec<IpAddr>> {
    net()?.resolve(host)
}
