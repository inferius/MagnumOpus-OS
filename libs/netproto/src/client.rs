//! Klientska knihovna: relace ([`Net`]) a sockety nad kanaly sluzby `netd`.
//!
//! Kazdy typ drzi jeden kanal. Metody jsou blokujici; na jednom socketu
//! smi v jednu chvili cekat jen jedno vlakno (sluzba drzi na kanalu jen
//! jeden pozadavek).

use alloc::vec::Vec;
use core::net::{IpAddr, SocketAddr};
use core::time::Duration;

use abi::time::WallTime;
use libos::{Channel, Error, Handle, Result};

use crate::codec::{Reader, Writer};
use crate::{Info, MAX_DATAGRAM, MAX_IO, Policy, io, ntp, op, parse_reply, timeout_ms};

/// Posle pozadavek a pocka na odpoved: (telo, handly).
fn call(ch: &Channel, req: Writer) -> Result<(Vec<u8>, Vec<Handle>)> {
    ch.write(&req.finish(), Vec::new())?;
    let msg = ch.read(None)?;
    let rest = parse_reply(&msg.data)?.rest().to_vec();
    Ok((rest, msg.handles))
}

/// Odpoved s prave jednim novym kanalem.
fn call_channel(ch: &Channel, req: Writer) -> Result<(Vec<u8>, Channel)> {
    let (body, mut handles) = call(ch, req)?;
    if handles.len() != 1 {
        return Err(Error::Corrupted);
    }
    Ok((body, Channel::from_handle(handles.remove(0))))
}

/// Relace u sluzby site (capability). Co smi, urcuji prava a omezeni,
/// se kterymi relaci vytvoril ten, kdo ji programu predal.
pub struct Net {
    ch: Channel,
}

impl Net {
    pub fn new(ch: Channel) -> Self {
        Self { ch }
    }

    pub fn channel(&self) -> &Channel {
        &self.ch
    }

    pub fn into_channel(self) -> Channel {
        self.ch
    }

    /// Stav rozhrani (pravo `INFO`).
    pub fn info(&self) -> Result<Info> {
        let (b, _) = call(&self.ch, Writer::op(op::INFO))?;
        Info::get(&mut Reader::new(&b))
    }

    /// Pocka, az ma rozhrani adresu (DHCP), nejvys `timeout`.
    pub fn wait_configured(&self, timeout: Option<Duration>) -> Result<Info> {
        let mut w = Writer::op(op::WAIT_CONFIGURED);
        w.u64(timeout_ms(timeout));
        let (b, _) = call(&self.ch, w)?;
        Info::get(&mut Reader::new(&b))
    }

    /// Uzsi relace pro jiny program: prava `rights` (jen podmnozina
    /// vlastnich) a dalsi vrstva omezeni `policy`. Vyzaduje pravo `RESTRICT`.
    pub fn restrict(&self, rights: u32, policy: &Policy) -> Result<Net> {
        let mut w = Writer::op(op::RESTRICT);
        w.u32(rights);
        policy.put(&mut w);
        let (_, ch) = call_channel(&self.ch, w)?;
        Ok(Net::new(ch))
    }

    /// Nastavi systemove servery DNS (pravo `CONFIGURE`); prazdny seznam =
    /// zpet servery z DHCP. Plati pro vsechny relace.
    pub fn set_dns(&self, servers: &[SocketAddr]) -> Result<()> {
        let mut w = Writer::op(op::SET_DNS);
        w.u8(servers.len().min(8) as u8);
        for s in servers.iter().take(8) {
            w.socket_addr(*s);
        }
        call(&self.ch, w).map(|_| ())
    }

    /// Preklad jmena systemovym resolverem (servery z DHCP), IPv4 i IPv6.
    pub fn resolve(&self, name: &str) -> Result<Vec<IpAddr>> {
        self.resolve_with(name, 0, None, Some(Duration::from_secs(5)))
            .map(|(a, _)| a)
    }

    /// Preklad jmena: `family` 0 = obe, 4 nebo 6; `server` = vlastni server
    /// DNS (musi ho povolit omezeni relace jako cil UDP). Vraci adresy a TTL.
    /// Jmeno neexistuje = `NOT_FOUND`.
    pub fn resolve_with(
        &self,
        name: &str,
        family: u8,
        server: Option<SocketAddr>,
        timeout: Option<Duration>,
    ) -> Result<(Vec<IpAddr>, u32)> {
        let mut w = Writer::op(op::RESOLVE);
        w.str(name).u8(family);
        match server {
            Some(s) => {
                w.u8(1).socket_addr(s);
            }
            None => {
                w.u8(0);
            }
        }
        w.u64(timeout_ms(timeout));
        let (b, _) = call(&self.ch, w)?;
        let mut r = Reader::new(&b);
        let n = r.u16()?;
        let mut addrs = Vec::with_capacity(n as usize);
        for _ in 0..n {
            addrs.push(r.ip()?);
        }
        Ok((addrs, r.u32()?))
    }

    /// ICMP echo: vraci dobu odezvy.
    pub fn ping(&self, addr: IpAddr, seq: u16, size: u16, timeout: Duration) -> Result<Duration> {
        let mut w = Writer::op(op::PING);
        w.ip(addr).u16(seq).u16(size).u64(timeout_ms(Some(timeout)));
        let (b, _) = call(&self.ch, w)?;
        Ok(Duration::from_nanos(Reader::new(&b).u64()?))
    }

    /// Spojeni TCP.
    pub fn tcp_connect(&self, addr: SocketAddr, timeout: Option<Duration>) -> Result<TcpStream> {
        let mut w = Writer::op(op::TCP_CONNECT);
        w.socket_addr(addr).u64(timeout_ms(timeout));
        let (b, ch) = call_channel(&self.ch, w)?;
        let local = Reader::new(&b).socket_addr()?;
        Ok(TcpStream {
            ch,
            local,
            remote: addr,
            read_timeout: None,
            write_timeout: None,
        })
    }

    /// Spojeni TCP na jmeno: prelozi ho a zkusi adresy postupne.
    pub fn tcp_connect_host(
        &self,
        host: &str,
        port: u16,
        timeout: Option<Duration>,
    ) -> Result<TcpStream> {
        let addrs = self.resolve(host)?;
        let mut last = Error::NotFound;
        for a in addrs {
            match self.tcp_connect(SocketAddr::new(a, port), timeout) {
                Ok(s) => return Ok(s),
                Err(e) => last = e,
            }
        }
        Err(last)
    }

    /// Naslouchani TCP na mistnim portu (`0` = prideli sluzba); `backlog`
    /// = kolik spojeni muze cekat na `accept`.
    pub fn tcp_listen(&self, port: u16, backlog: u16) -> Result<TcpListener> {
        let mut w = Writer::op(op::TCP_LISTEN);
        w.u16(port).u16(backlog);
        let (b, ch) = call_channel(&self.ch, w)?;
        Ok(TcpListener {
            ch,
            local: Reader::new(&b).socket_addr()?,
        })
    }

    /// Socket UDP na mistnim portu (`0` = prideli sluzba).
    pub fn udp_bind(&self, port: u16) -> Result<UdpSocket> {
        let mut w = Writer::op(op::UDP_BIND);
        w.u16(port);
        let (b, ch) = call_channel(&self.ch, w)?;
        Ok(UdpSocket {
            ch,
            local: Reader::new(&b).socket_addr()?,
        })
    }

    /// Dotaz NTP na `server`: presny cas a posun mistnich hodin. Hodiny
    /// nenastavuje - to smi jen sluzba casu (`timed`), viz `docs/cs/cas.md`.
    pub fn ntp_query(&self, server: SocketAddr, timeout: Duration) -> Result<NtpSync> {
        let sock = self.udp_bind(0)?;
        let deadline = libos::time::deadline_after(timeout);
        let mut attempt_timeout = Duration::from_secs(1);
        loop {
            let t1 = libos::time::wall().unwrap_or(WallTime::UNIX_EPOCH);
            sock.send_to(&ntp::build_request(t1), server)?;
            let wait_until = libos::time::now().saturating_add(attempt_timeout.as_nanos() as u64);
            loop {
                let now = libos::time::now();
                let until = wait_until.min(deadline);
                if now >= until {
                    break;
                }
                let mut buf = [0u8; 128];
                match sock.recv_from(&mut buf, Some(Duration::from_nanos(until - now))) {
                    Ok((n, from)) if from == server => {
                        let t4 = libos::time::wall().unwrap_or(WallTime::UNIX_EPOCH);
                        match ntp::parse_reply(t1, &buf[..n]) {
                            Ok(reply) => {
                                let (offset, delay) = ntp::offset_delay(t1, &reply, t4);
                                return Ok(NtpSync {
                                    time: ntp::corrected(t4, offset),
                                    offset_ns: offset,
                                    delay_ns: delay,
                                    stratum: reply.stratum,
                                });
                            }
                            Err(Error::InvalidArgs) => continue,
                            Err(e) => return Err(e),
                        }
                    }
                    Ok(_) => continue,
                    Err(Error::TimedOut) => break,
                    Err(e) => return Err(e),
                }
            }
            if libos::time::now() >= deadline {
                return Err(Error::TimedOut);
            }
            attempt_timeout = (attempt_timeout * 2).min(Duration::from_secs(4));
        }
    }
}

/// Vysledek dotazu NTP.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NtpSync {
    /// Opraveny cas v okamziku prijeti odpovedi.
    pub time: WallTime,
    /// Posun mistnich hodin v ns (kladny = mistni hodiny jdou pozadu).
    pub offset_ns: i128,
    /// Zpozdeni cesty tam a zpet v ns.
    pub delay_ns: i128,
    pub stratum: u8,
}

/// Spojeni TCP. Zavreni (drop) spojeni ukonci; data, ktera sluzba prijala,
/// jeste dorazi (sluzba spojeni zavira postupne).
pub struct TcpStream {
    ch: Channel,
    local: SocketAddr,
    remote: SocketAddr,
    read_timeout: Option<Duration>,
    write_timeout: Option<Duration>,
}

impl TcpStream {
    fn from_accept(ch: Channel, local: SocketAddr, remote: SocketAddr) -> Self {
        Self {
            ch,
            local,
            remote,
            read_timeout: None,
            write_timeout: None,
        }
    }

    pub fn local_addr(&self) -> SocketAddr {
        self.local
    }

    pub fn peer_addr(&self) -> SocketAddr {
        self.remote
    }

    /// Limit cekani na data (`None` = bez limitu). Vyprseni = `TIMED_OUT`.
    pub fn set_read_timeout(&mut self, t: Option<Duration>) {
        self.read_timeout = t;
    }

    /// Limit cekani na misto v odesilaci fronte.
    pub fn set_write_timeout(&mut self, t: Option<Duration>) {
        self.write_timeout = t;
    }

    /// Ukonci zapis (druha strana dostane konec proudu), cteni zustava.
    pub fn shutdown_write(&self) -> Result<()> {
        call(&self.ch, Writer::op(op::SHUTDOWN)).map(|_| ())
    }

    /// Kanal spojeni (napr. pro predani jinemu procesu: `into_channel`).
    pub fn channel(&self) -> &Channel {
        &self.ch
    }

    pub fn into_channel(self) -> Channel {
        self.ch
    }

    /// Spojeni z kanalu, ktery prisel od jineho procesu.
    pub fn from_channel(ch: Channel) -> Result<Self> {
        let (b, _) = call(&ch, Writer::op(op::ADDRS))?;
        let mut r = Reader::new(&b);
        let local = r.socket_addr()?;
        let remote = r.socket_addr()?;
        Ok(Self::from_accept(ch, local, remote))
    }
}

impl io::Read for TcpStream {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        let mut w = Writer::op(op::RECV);
        w.u32(buf.len().min(MAX_IO) as u32)
            .u64(timeout_ms(self.read_timeout));
        let (b, _) = call(&self.ch, w)?;
        let n = b.len().min(buf.len());
        buf[..n].copy_from_slice(&b[..n]);
        Ok(n)
    }
}

impl io::Write for TcpStream {
    fn write(&mut self, buf: &[u8]) -> Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        let chunk = &buf[..buf.len().min(MAX_IO)];
        let mut w = Writer::op(op::SEND);
        w.u64(timeout_ms(self.write_timeout)).bytes(chunk);
        let (b, _) = call(&self.ch, w)?;
        let n = Reader::new(&b).u32()? as usize;
        if n == 0 {
            // Nic neprijato az do vyprseni.
            return Err(Error::TimedOut);
        }
        Ok(n)
    }
}

/// Naslouchani TCP.
pub struct TcpListener {
    ch: Channel,
    local: SocketAddr,
}

impl TcpListener {
    pub fn local_addr(&self) -> SocketAddr {
        self.local
    }

    /// Pocka na prichozi spojeni.
    pub fn accept(&self) -> Result<(TcpStream, SocketAddr)> {
        self.accept_timeout(None)
    }

    pub fn accept_timeout(&self, timeout: Option<Duration>) -> Result<(TcpStream, SocketAddr)> {
        let mut w = Writer::op(op::ACCEPT);
        w.u64(timeout_ms(timeout));
        let (b, ch) = call_channel(&self.ch, w)?;
        let remote = Reader::new(&b).socket_addr()?;
        Ok((TcpStream::from_accept(ch, self.local, remote), remote))
    }
}

/// Socket UDP.
pub struct UdpSocket {
    ch: Channel,
    local: SocketAddr,
}

impl UdpSocket {
    pub fn local_addr(&self) -> SocketAddr {
        self.local
    }

    /// Posle datagram (nejvys [`MAX_DATAGRAM`] bajtu).
    pub fn send_to(&self, data: &[u8], to: SocketAddr) -> Result<()> {
        if data.len() > MAX_DATAGRAM {
            return Err(Error::OutOfRange);
        }
        let mut w = Writer::op(op::SEND_TO);
        w.socket_addr(to).bytes(data);
        call(&self.ch, w).map(|_| ())
    }

    /// Prijme datagram: (delka, odesilatel). Delsi datagram se zkrati.
    pub fn recv_from(
        &self,
        buf: &mut [u8],
        timeout: Option<Duration>,
    ) -> Result<(usize, SocketAddr)> {
        let mut w = Writer::op(op::RECV_FROM);
        w.u32(buf.len().min(MAX_DATAGRAM) as u32)
            .u64(timeout_ms(timeout));
        let (b, _) = call(&self.ch, w)?;
        let mut r = Reader::new(&b);
        let from = r.socket_addr()?;
        let data = r.rest();
        let n = data.len().min(buf.len());
        buf[..n].copy_from_slice(&data[..n]);
        Ok((n, from))
    }
}
