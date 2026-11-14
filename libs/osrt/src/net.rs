//! Sit pro `std::net` nad relaci site (netproto, sluzba netd).
//!
//! Relaci preda rodic ve startovni zprave (stitek `abi::params::tag::NET`);
//! co smi (protokoly, cile, porty, dotazy uzivateli), urcil ten, kdo ji
//! vytvoril (manifest aplikace). Bez relace vraci operace `NotSupported`.
//!
//! Omezeni protokolu: odpovedi netd nemaji cislo dotazu a na jednom kanalu
//! (relace, spojeni, socket) je rozpracovany jen jeden dotaz. Runtime proto
//! kazdy kanal zamyka: soubezne volani z vic vlaken na stejnem objektu
//! cekaji za sebou (cteni blokovane na data zdrzi zapis na stejnem
//! spojeni). Viz docs/cs/rust-std.md, kapitola Sit.

use alloc::vec::Vec;
use core::net::{IpAddr, SocketAddr};
use core::time::Duration;

use libos::Channel;
use libos::sync::Mutex;
use netproto::Net;
use netproto::io::{Read as _, Write as _};

use crate::start::Init;
use crate::{Error, Result};

static SESSION: Init<Mutex<Net>> = Init::new();

/// # Safety
///
/// Jen pri startu (viz `start::Init`).
pub(crate) unsafe fn init(channel: Option<Channel>) {
    if let Some(c) = channel {
        // SAFETY: predano volajicim.
        unsafe { SESSION.set(Mutex::new(Net::new(c))) };
    }
}

fn session() -> Result<&'static Mutex<Net>> {
    SESSION.get().ok_or(Error::NotSupported)
}

/// Klon relace site (sdili kanal) pro sockety knihovny C ([`crate::sock`]).
pub fn session_net() -> Result<Net> {
    Ok(session()?.lock().clone())
}

/// Ma proces relaci site?
pub fn available() -> bool {
    SESSION.get().is_some()
}

/// Duplikat kanalu relace (pro dite: `Command`).
pub fn session_channel() -> Option<Channel> {
    let s = SESSION.get()?;
    let h = s.lock().channel().handle().duplicate(None).ok()?;
    Some(Channel::from_handle(h))
}

/// Preklad jmena na adresy (netd -> dnsd).
pub fn resolve(host: &str) -> Result<Vec<IpAddr>> {
    session()?.lock().resolve(host)
}

/// Spojeni TCP.
pub struct TcpStream {
    inner: Mutex<netproto::TcpStream>,
    local: SocketAddr,
    peer: SocketAddr,
    timeouts: Mutex<(Option<Duration>, Option<Duration>)>,
}

impl TcpStream {
    fn wrap(s: netproto::TcpStream) -> Self {
        let (local, peer) = (s.local_addr(), s.peer_addr());
        Self { inner: Mutex::new(s), local, peer, timeouts: Mutex::new((None, None)) }
    }

    pub fn connect(addr: SocketAddr, timeout: Option<Duration>) -> Result<TcpStream> {
        let s = session()?.lock().tcp_connect(addr, timeout)?;
        Ok(Self::wrap(s))
    }

    pub fn read(&self, buf: &mut [u8]) -> Result<usize> {
        match self.inner.lock().read(buf) {
            // Konec proudu i zavrene spojeni = 0 (jako POSIX read).
            Err(Error::PeerClosed) => Ok(0),
            r => r,
        }
    }

    pub fn write(&self, buf: &[u8]) -> Result<usize> {
        self.inner.lock().write(buf)
    }

    pub fn local_addr(&self) -> SocketAddr {
        self.local
    }

    pub fn peer_addr(&self) -> SocketAddr {
        self.peer
    }

    pub fn shutdown_write(&self) -> Result<()> {
        self.inner.lock().shutdown_write()
    }

    pub fn set_read_timeout(&self, t: Option<Duration>) {
        self.inner.lock().set_read_timeout(t);
        self.timeouts.lock().0 = t;
    }

    pub fn set_write_timeout(&self, t: Option<Duration>) {
        self.inner.lock().set_write_timeout(t);
        self.timeouts.lock().1 = t;
    }

    pub fn read_timeout(&self) -> Option<Duration> {
        self.timeouts.lock().0
    }

    pub fn write_timeout(&self) -> Option<Duration> {
        self.timeouts.lock().1
    }
}

/// Naslouchani TCP.
pub struct TcpListener {
    inner: Mutex<netproto::TcpListener>,
}

impl TcpListener {
    /// Naslouchani na portu `addr`. netd posloucha na vsech rozhranich:
    /// adresa rozhrani v `addr` se zatim nepouzije (127.0.0.1 = vsechna).
    pub fn bind(addr: SocketAddr) -> Result<TcpListener> {
        let l = session()?.lock().tcp_listen(addr.port(), 64)?;
        Ok(TcpListener { inner: Mutex::new(l) })
    }

    pub fn local_addr(&self) -> Result<SocketAddr> {
        self.inner.lock().local_addr()
    }

    pub fn accept(&self) -> Result<(TcpStream, SocketAddr)> {
        let (s, from) = self.inner.lock().accept()?;
        Ok((TcpStream::wrap(s), from))
    }
}

/// Socket UDP.
pub struct UdpSocket {
    inner: Mutex<netproto::UdpSocket>,
    /// Cil po `connect` (std ho emuluje: `send`/`recv` jen s timto cilem).
    peer: Mutex<Option<SocketAddr>>,
    read_timeout: Mutex<Option<Duration>>,
}

impl UdpSocket {
    pub fn bind(addr: SocketAddr) -> Result<UdpSocket> {
        let s = session()?.lock().udp_bind(addr.port())?;
        Ok(UdpSocket { inner: Mutex::new(s), peer: Mutex::new(None), read_timeout: Mutex::new(None) })
    }

    pub fn local_addr(&self) -> Result<SocketAddr> {
        self.inner.lock().local_addr()
    }

    pub fn send_to(&self, data: &[u8], to: SocketAddr) -> Result<usize> {
        self.inner.lock().send_to(data, to).map(|_| data.len())
    }

    pub fn recv_from(&self, buf: &mut [u8]) -> Result<(usize, SocketAddr)> {
        let t = *self.read_timeout.lock();
        self.inner.lock().recv_from(buf, t)
    }

    pub fn connect(&self, to: SocketAddr) {
        *self.peer.lock() = Some(to);
    }

    pub fn peer_addr(&self) -> Result<SocketAddr> {
        (*self.peer.lock()).ok_or(Error::BadState)
    }

    pub fn send(&self, data: &[u8]) -> Result<usize> {
        let to = self.peer_addr()?;
        self.send_to(data, to)
    }

    /// Prijme datagram jen od cile z `connect` (ostatni zahodi).
    pub fn recv(&self, buf: &mut [u8]) -> Result<usize> {
        let peer = self.peer_addr()?;
        loop {
            let (n, from) = self.recv_from(buf)?;
            if from == peer {
                return Ok(n);
            }
        }
    }

    pub fn set_read_timeout(&self, t: Option<Duration>) {
        *self.read_timeout.lock() = t;
    }

    pub fn read_timeout(&self) -> Option<Duration> {
        *self.read_timeout.lock()
    }
}
