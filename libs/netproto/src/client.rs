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

use crate::codec::{ReadAddr, Reader, WriteAddr, Writer};
use crate::fw::{AppStats, AuditRecord};
use crate::{Buffers, Info, MAX_DATAGRAM, MAX_IO, Policy, io, ntp, op, parse_reply, timeout_ms};

/// Jak dlouho cekat na odpoved sluzby site.
///
/// Limity operaci (spojeni, cteni, preklad) hlida `netd`. Klient ma navic
/// vlastni pojistku: kdyby sluzba pozadavek nedokoncila (chyba ve sluzbe,
/// zaseknuty ovladac), program dostane `TIMED_OUT` s hlaskou v logu misto
/// cekani navzdy.
#[derive(Clone, Copy)]
enum Wait {
    /// Operace bez cekani na sit (stav, relace, socket): nejvys 30 s.
    Immediate,
    /// Operace s limitem programu (`None` = bez limitu - program o to
    /// pozadal).
    Op(Option<Duration>),
    /// Operace, ktera muze cekat i na rozhodnuti uzivatele (dotaz
    /// firewallu): limit + az 300 s na dotaz.
    Gated(Option<Duration>),
}

/// Rezerva nad limit operace (zpracovani ve sluzbe, zatizeny system).
const REPLY_MARGIN: Duration = Duration::from_secs(10);
const IMMEDIATE: Duration = Duration::from_secs(30);
/// Nejdelsi cekani na rozhodnuti uzivatele ve sluzbe (`fw::ASK_WAIT_NS`).
const ASK_WAIT: Duration = Duration::from_secs(300);

impl Wait {
    fn limit(self) -> Option<Duration> {
        match self {
            Self::Immediate => Some(IMMEDIATE),
            Self::Op(t) => t.map(|t| t.saturating_add(REPLY_MARGIN)),
            Self::Gated(t) => t.map(|t| t.saturating_add(ASK_WAIT).saturating_add(REPLY_MARGIN)),
        }
    }
}

/// Kanaly, na ktere sluzba neodpovedela vcas. Pozdni odpoved by se
/// precetla jako odpoved na dalsi pozadavek, proto kanal uz nejde pouzit.
static STALE: [core::sync::atomic::AtomicU32; 32] = [const { core::sync::atomic::AtomicU32::new(0) }; 32];

fn is_stale(raw: u32) -> bool {
    raw != 0 && STALE.iter().any(|s| s.load(core::sync::atomic::Ordering::Relaxed) == raw)
}

fn mark_stale(raw: u32) {
    use core::sync::atomic::Ordering;
    for s in &STALE {
        if s.compare_exchange(0, raw, Ordering::Relaxed, Ordering::Relaxed).is_ok() {
            return;
        }
    }
}

/// Posle pozadavek a pocka na odpoved: (telo, handly).
fn call(ch: &Channel, req: Writer) -> Result<(Vec<u8>, Vec<Handle>)> {
    call_wait(ch, req, Wait::Immediate)
}

fn call_wait(ch: &Channel, req: Writer, wait: Wait) -> Result<(Vec<u8>, Vec<Handle>)> {
    let raw = ch.handle().raw();
    if is_stale(raw) {
        return Err(Error::BadState);
    }
    let data = req.finish();
    ch.write(&data, Vec::new())?;
    let limit = wait.limit();
    let msg = match ch.read(limit.map(libos::time::deadline_after)) {
        Ok(m) => m,
        Err(Error::TimedOut) => {
            mark_stale(raw);
            let op = data.get(..4).map_or(0, |b| u32::from_le_bytes(b.try_into().unwrap()));
            libos::println!(
                "netproto: sluzba site neodpovedela na pozadavek {op} do {} s (kanal uz nejde pouzit)",
                limit.map_or(0, |l| l.as_secs())
            );
            return Err(Error::TimedOut);
        }
        Err(e) => return Err(e),
    };
    let rest = parse_reply(&msg.data)?.rest().to_vec();
    Ok((rest, msg.handles))
}

/// Odpoved s prave jednim novym kanalem.
fn call_channel(ch: &Channel, req: Writer) -> Result<(Vec<u8>, Channel)> {
    call_channel_wait(ch, req, Wait::Immediate)
}

fn call_channel_wait(ch: &Channel, req: Writer, wait: Wait) -> Result<(Vec<u8>, Channel)> {
    let (body, mut handles) = call_wait(ch, req, wait)?;
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
        let (b, _) = call_wait(&self.ch, w, Wait::Op(timeout))?;
        Info::get(&mut Reader::new(&b))
    }

    /// Uzsi relace pro jiny program: prava `rights` (jen podmnozina
    /// vlastnich) a dalsi vrstva omezeni `policy`. Vyzaduje pravo `RESTRICT`.
    pub fn restrict(&self, rights: u32, policy: &Policy) -> Result<Net> {
        self.restrict_with(&crate::Restriction::new(rights, policy.clone()))
    }

    /// Uzsi relace s identitou aplikace, dotazem na uzivatele a rozpoctem
    /// (init podle manifestu aplikace).
    pub fn restrict_with(&self, r: &crate::Restriction) -> Result<Net> {
        let mut w = Writer::op(op::RESTRICT);
        r.put(&mut w);
        let (_, ch) = call_channel(&self.ch, w)?;
        Ok(Net::new(ch))
    }

    /// Zaznamy auditu firewallu od poradoveho cisla `from` (nejvys `max`).
    /// Vraci zaznamy a cislo, od ktereho cist priste. Pravo `AUDIT`.
    pub fn audit(&self, from: u64, max: u16) -> Result<(Vec<AuditRecord>, u64)> {
        let mut w = Writer::op(op::AUDIT);
        w.u64(from).u16(max);
        let (b, _) = call(&self.ch, w)?;
        let mut r = Reader::new(&b);
        let next = r.u64()?;
        let n = r.u16()?;
        let mut out = Vec::with_capacity(n as usize);
        for _ in 0..n {
            out.push(AuditRecord::get(&mut r)?);
        }
        Ok((out, next))
    }

    /// Aktivni profil site a provoz podle aplikaci. Pravo `AUDIT`.
    pub fn stats(&self) -> Result<(alloc::string::String, Vec<AppStats>)> {
        let (b, _) = call(&self.ch, Writer::op(op::STATS))?;
        let mut r = Reader::new(&b);
        let profile = r.str()?;
        let n = r.u16()?;
        let mut out = Vec::with_capacity(n as usize);
        for _ in 0..n {
            out.push(AppStats::get(&mut r)?);
        }
        Ok((profile, out))
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
        // Systemovy preklad muze overovat DNSSEC (vic dotazu): 15 s.
        self.resolve_with(name, 0, None, Some(Duration::from_secs(15)))
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
        // Vlastni server muze cekat na dotaz firewallu; systemovy preklad
        // nejvys 30 s ve sluzbe.
        let wait = if server.is_some() { Wait::Gated(timeout) } else { Wait::Op(Some(timeout.unwrap_or(Duration::from_secs(30)))) };
        let (b, _) = call_wait(&self.ch, w, wait)?;
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
        let (b, _) = call_wait(&self.ch, w, Wait::Gated(Some(timeout)))?;
        Ok(Duration::from_nanos(Reader::new(&b).u64()?))
    }

    /// Spojeni TCP (vychozi buffery).
    pub fn tcp_connect(&self, addr: SocketAddr, timeout: Option<Duration>) -> Result<TcpStream> {
        self.tcp_connect_with(addr, timeout, Buffers::DEFAULT)
    }

    /// Spojeni TCP s velikosti bufferu (vetsi = vyssi propustnost na
    /// vzdalene lince; strhava se z rozpoctu relace).
    pub fn tcp_connect_with(
        &self,
        addr: SocketAddr,
        timeout: Option<Duration>,
        buffers: Buffers,
    ) -> Result<TcpStream> {
        let mut w = Writer::op(op::TCP_CONNECT);
        w.socket_addr(addr).u64(timeout_ms(timeout));
        buffers.put(&mut w);
        let (b, ch) = call_channel_wait(&self.ch, w, Wait::Gated(timeout))?;
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
        self.tcp_listen_with(port, backlog, Buffers::DEFAULT)
    }

    /// Naslouchani s velikosti bufferu prijatych spojeni.
    pub fn tcp_listen_with(&self, port: u16, backlog: u16, buffers: Buffers) -> Result<TcpListener> {
        let mut w = Writer::op(op::TCP_LISTEN);
        w.u16(port).u16(backlog);
        buffers.put(&mut w);
        let (_, ch) = call_channel(&self.ch, w)?;
        Ok(TcpListener { ch })
    }

    /// Socket UDP na mistnim portu (`0` = prideli sluzba).
    pub fn udp_bind(&self, port: u16) -> Result<UdpSocket> {
        self.udp_bind_with(port, Buffers::DEFAULT)
    }

    /// Socket UDP s velikosti front (nejvetsi odeslany datagram = `tx`).
    pub fn udp_bind_with(&self, port: u16, buffers: Buffers) -> Result<UdpSocket> {
        let mut w = Writer::op(op::UDP_BIND);
        w.u16(port);
        buffers.put(&mut w);
        let (_, ch) = call_channel(&self.ch, w)?;
        Ok(UdpSocket { ch })
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
        let (b, _) = call_wait(&self.ch, w, Wait::Op(self.read_timeout))?;
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
        let (b, _) = call_wait(&self.ch, w, Wait::Op(self.write_timeout))?;
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
}

/// Mistni adresa od sluzby (meni se s adresou rozhrani, napr. po DHCP).
fn local_addr_of(ch: &Channel) -> Result<SocketAddr> {
    let (b, _) = call(ch, Writer::op(op::LOCAL_ADDR))?;
    Reader::new(&b).socket_addr()
}

impl TcpListener {
    /// Mistni adresa (pred nastavenim rozhrani 0.0.0.0:port).
    pub fn local_addr(&self) -> Result<SocketAddr> {
        local_addr_of(&self.ch)
    }

    /// Pocka na prichozi spojeni.
    pub fn accept(&self) -> Result<(TcpStream, SocketAddr)> {
        self.accept_timeout(None)
    }

    pub fn accept_timeout(&self, timeout: Option<Duration>) -> Result<(TcpStream, SocketAddr)> {
        let mut w = Writer::op(op::ACCEPT);
        w.u64(timeout_ms(timeout));
        let (b, ch) = call_channel_wait(&self.ch, w, Wait::Op(timeout))?;
        let mut r = Reader::new(&b);
        let remote = r.socket_addr()?;
        let local = r.socket_addr()?;
        Ok((TcpStream::from_accept(ch, local, remote), remote))
    }
}

/// Socket UDP.
pub struct UdpSocket {
    ch: Channel,
}

impl UdpSocket {
    /// Mistni adresa (pred nastavenim rozhrani 0.0.0.0:port).
    pub fn local_addr(&self) -> Result<SocketAddr> {
        local_addr_of(&self.ch)
    }

    /// Posle datagram (nejvys [`MAX_DATAGRAM`] bajtu).
    pub fn send_to(&self, data: &[u8], to: SocketAddr) -> Result<()> {
        if data.len() > MAX_DATAGRAM {
            return Err(Error::OutOfRange);
        }
        let mut w = Writer::op(op::SEND_TO);
        w.socket_addr(to).bytes(data);
        // Muze cekat na dotaz firewallu (neznamy cil).
        call_wait(&self.ch, w, Wait::Gated(Some(Duration::ZERO))).map(|_| ())
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
        let (b, _) = call_wait(&self.ch, w, Wait::Op(timeout))?;
        let mut r = Reader::new(&b);
        let from = r.socket_addr()?;
        let data = r.rest();
        let n = data.len().min(buf.len());
        buf[..n].copy_from_slice(&data[..n]);
        Ok((n, from))
    }
}
