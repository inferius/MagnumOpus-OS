//! Klientska knihovna: relace ([`Net`]) a sockety nad kanaly sluzby `netd`.
//!
//! Kazdy typ drzi jeden kanal sdileny pres [`mux::Mux`]: typy jsou `Send +
//! Sync`, metody berou `&self` a na jednom kanalu muze cekat vic vlaken
//! najednou (odpovedi se paruji podle cisla dotazu). Spojeni TCP lze cist
//! z jednoho vlakna a zapisovat z druheho (`try_clone`). Dlouhe operace
//! (connect, resolve, accept, recv) jde zrusit tokenem [`CancelToken`].

mod mux;

use alloc::sync::Arc;
use alloc::vec::Vec;
use core::net::{IpAddr, SocketAddr};
use core::sync::atomic::{AtomicU64, Ordering};
use core::time::Duration;

use abi::time::WallTime;
use libos::{Channel, Error, Handle, Result};

pub use mux::CancelToken;
use mux::{Mux, Wait};

use crate::codec::{ReadAddr, Reader, WriteAddr, Writer};
use crate::fw::{AppStats, AuditRecord};
use crate::msg::{self, NO_DEADLINE};
use crate::{Buffers, Info, MAX_DATAGRAM, MAX_IO, Policy, ntp, op, parse_reply};

/// Termin operace: absolutni cas monotonnich hodin jadra v ns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Deadline(pub u64);

impl Deadline {
    /// Bez limitu.
    pub const NONE: Self = Self(NO_DEADLINE);
    /// Jen zkusit (necekat).
    pub const TRY: Self = Self(msg::TRY);

    /// Za `t` od ted (`None` = bez limitu).
    pub fn after(t: Option<Duration>) -> Self {
        Self(msg::deadline_after(libos::time::now(), t))
    }
}

/// Pripravenost socketu: event, na kterem sluzba nastavuje bity
/// [`crate::ready`] (cteni, zapis, konec, chyba). Pro neblokujici smycky
/// a backend mio: `handle()` se zaregistruje na portu (`Port::wait_async`
/// s maskou podle zajmu), po udalosti se operace zkusi s [`Deadline::TRY`]
/// (`SHOULD_WAIT` = uz neni pripraveny) a registrace se obnovi. Stav je
/// urovnovy, takze bit nastaveny pri registraci se ohlasi hned.
pub struct Readiness(libos::Event);

impl Readiness {
    pub fn handle(&self) -> &Handle {
        self.0.handle()
    }

    /// Pocka, az bude nastaveny nektery z bitu `mask`, nejvys do
    /// `deadline`; vraci vsechny bity (`TIMED_OUT` po terminu).
    pub fn wait(&self, mask: libos::Signals, deadline: Deadline) -> Result<libos::Signals> {
        let d = if deadline == Deadline::NONE { None } else { Some(deadline.0) };
        libos::wait_one(self.handle(), mask, d)
    }

    /// Bity prave ted (bez cekani).
    pub fn current(&self) -> libos::Signals {
        match libos::wait_one(self.handle(), crate::ready::ALL, Some(0)) {
            Ok(s) => libos::Signals(s.0 & crate::ready::ALL.0),
            Err(_) => libos::Signals::NONE,
        }
    }
}

/// `READINESS` na kanalu socketu.
fn readiness_of(m: &Arc<Mux>) -> Result<Readiness> {
    let (_, mut handles) = call(m, op::READINESS, Writer::new(), Deadline::NONE, Wait::Immediate, None)?;
    if handles.len() != 1 {
        return Err(Error::Corrupted);
    }
    Ok(Readiness(libos::Event::from_handle(handles.remove(0))))
}

/// Posle dotaz a vrati telo odpovedi a handly.
fn call(
    m: &Arc<Mux>,
    code: u32,
    body: Writer,
    deadline: Deadline,
    wait: Wait,
    cancel: Option<&CancelToken>,
) -> Result<(Vec<u8>, Vec<Handle>)> {
    let msg = m.call(code, deadline.0, &body.finish(), wait, cancel)?;
    let rest = parse_reply(&msg.data)?.rest().to_vec();
    Ok((rest, msg.handles))
}

/// Dotaz bez cekani na sit.
fn call_now(m: &Arc<Mux>, code: u32, body: Writer) -> Result<Vec<u8>> {
    call(m, code, body, Deadline::NONE, Wait::Immediate, None).map(|(b, _)| b)
}

/// `DUP`: novy kanal ke stejnemu socketu (pro jiny proces).
fn dup_of(m: &Arc<Mux>) -> Result<Channel> {
    one_channel(call(m, op::DUP, Writer::new(), Deadline::NONE, Wait::Immediate, None)?).map(|(_, ch)| ch)
}

/// Odpoved s prave jednim novym kanalem.
fn one_channel((body, mut handles): (Vec<u8>, Vec<Handle>)) -> Result<(Vec<u8>, Channel)> {
    if handles.len() != 1 {
        return Err(Error::Corrupted);
    }
    Ok((body, Channel::from_handle(handles.remove(0))))
}

/// Relace u sluzby site (capability). Co smi, urcuji prava a omezeni,
/// se kterymi relaci vytvoril ten, kdo ji programu predal. Klon sdili
/// kanal relace.
#[derive(Clone)]
pub struct Net {
    mux: Arc<Mux>,
}

impl Net {
    pub fn new(ch: Channel) -> Self {
        Self { mux: Mux::new(ch) }
    }

    pub fn channel(&self) -> &Channel {
        self.mux.channel()
    }

    /// Kanal relace (napr. pro predani jinemu procesu). `BAD_STATE`, kdyz
    /// relaci jeste drzi klon nebo na ni bezi dotaz.
    pub fn into_channel(self) -> Result<Channel> {
        Mux::into_channel(self.mux)
    }

    /// Stav rozhrani (pravo `INFO`).
    pub fn info(&self) -> Result<Info> {
        let b = call_now(&self.mux, op::INFO, Writer::new())?;
        Info::get(&mut Reader::new(&b))
    }

    /// Pocka, az ma rozhrani adresu (DHCP), nejvys `timeout`.
    pub fn wait_configured(&self, timeout: Option<Duration>) -> Result<Info> {
        let (b, _) = call(
            &self.mux,
            op::WAIT_CONFIGURED,
            Writer::new(),
            Deadline::after(timeout),
            Wait::Op,
            None,
        )?;
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
        let mut w = Writer::new();
        r.put(&mut w);
        let (_, ch) = one_channel(call(&self.mux, op::RESTRICT, w, Deadline::NONE, Wait::Immediate, None)?)?;
        Ok(Net::new(ch))
    }

    /// Zaznamy auditu firewallu od poradoveho cisla `from` (nejvys `max`).
    /// Vraci zaznamy a cislo, od ktereho cist priste. Pravo `AUDIT`.
    pub fn audit(&self, from: u64, max: u16) -> Result<(Vec<AuditRecord>, u64)> {
        let mut w = Writer::new();
        w.u64(from).u16(max);
        let b = call_now(&self.mux, op::AUDIT, w)?;
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
        let b = call_now(&self.mux, op::STATS, Writer::new())?;
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
        let mut w = Writer::new();
        w.u8(servers.len().min(8) as u8);
        for s in servers.iter().take(8) {
            w.socket_addr(*s);
        }
        call_now(&self.mux, op::SET_DNS, w).map(|_| ())
    }

    /// Preklad jmena systemovym resolverem (servery z DHCP), IPv4 i IPv6.
    pub fn resolve(&self, name: &str) -> Result<Vec<IpAddr>> {
        // Systemovy preklad muze overovat DNSSEC (vic dotazu): 15 s.
        self.resolve_with(name, 0, None, Some(Duration::from_secs(15))).map(|(a, _)| a)
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
        self.resolve_ctx(name, family, server, Deadline::after(timeout), None)
    }

    /// Preklad s terminem a moznosti zruseni. Systemovy preklad bez
    /// terminu ma ve sluzbe limit 30 s.
    pub fn resolve_ctx(
        &self,
        name: &str,
        family: u8,
        server: Option<SocketAddr>,
        deadline: Deadline,
        cancel: Option<&CancelToken>,
    ) -> Result<(Vec<IpAddr>, u32)> {
        let mut w = Writer::new();
        w.str(name).u8(family);
        match server {
            Some(s) => {
                w.u8(1).socket_addr(s);
            }
            None => {
                w.u8(0);
            }
        }
        // Vlastni server muze cekat na dotaz firewallu.
        let (wait, deadline) = if server.is_some() {
            (Wait::Gated, deadline)
        } else if deadline == Deadline::NONE {
            (Wait::Op, Deadline::after(Some(Duration::from_secs(30))))
        } else {
            (Wait::Op, deadline)
        };
        let (b, _) = call(&self.mux, op::RESOLVE, w, deadline, wait, cancel)?;
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
        let mut w = Writer::new();
        w.ip(addr).u16(seq).u16(size);
        let (b, _) = call(&self.mux, op::PING, w, Deadline::after(Some(timeout)), Wait::Gated, None)?;
        Ok(Duration::from_nanos(Reader::new(&b).u64()?))
    }

    /// Spojeni TCP (vychozi buffery).
    pub fn tcp_connect(&self, addr: SocketAddr, timeout: Option<Duration>) -> Result<TcpStream> {
        self.tcp_connect_ctx(addr, Deadline::after(timeout), Buffers::DEFAULT, None)
    }

    /// Spojeni TCP s velikosti bufferu (vetsi = vyssi propustnost na
    /// vzdalene lince; strhava se z rozpoctu relace).
    pub fn tcp_connect_with(&self, addr: SocketAddr, timeout: Option<Duration>, buffers: Buffers) -> Result<TcpStream> {
        self.tcp_connect_ctx(addr, Deadline::after(timeout), buffers, None)
    }

    /// Spojeni TCP s terminem a moznosti zruseni. Cas, kdy sluzba ceka na
    /// rozhodnuti uzivatele (firewall), se do terminu nepocita.
    pub fn tcp_connect_ctx(
        &self,
        addr: SocketAddr,
        deadline: Deadline,
        buffers: Buffers,
        cancel: Option<&CancelToken>,
    ) -> Result<TcpStream> {
        let mut w = Writer::new();
        w.socket_addr(addr);
        buffers.put(&mut w);
        let (b, ch) = one_channel(call(&self.mux, op::TCP_CONNECT, w, deadline, Wait::Gated, cancel)?)?;
        let local = Reader::new(&b).socket_addr()?;
        Ok(TcpStream::new(ch, local, addr))
    }

    /// Spojeni TCP na jmeno: prelozi ho a zkusi adresy postupne.
    pub fn tcp_connect_host(&self, host: &str, port: u16, timeout: Option<Duration>) -> Result<TcpStream> {
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
        self.tcp_listen_addr(SocketAddr::from(([0, 0, 0, 0], port)), backlog, buffers)
    }

    /// Naslouchani na mistni adrese: `0.0.0.0` (vsechna rozhrani) nebo
    /// adresa rozhrani, jinak `ADDRESS_NOT_AVAILABLE`.
    pub fn tcp_listen_addr(&self, addr: SocketAddr, backlog: u16, buffers: Buffers) -> Result<TcpListener> {
        let mut w = Writer::new();
        w.socket_addr(addr).u16(backlog);
        buffers.put(&mut w);
        let (_, ch) = one_channel(call(&self.mux, op::TCP_LISTEN, w, Deadline::NONE, Wait::Immediate, None)?)?;
        Ok(TcpListener { mux: Mux::new(ch) })
    }

    /// Socket UDP na mistnim portu (`0` = prideli sluzba).
    pub fn udp_bind(&self, port: u16) -> Result<UdpSocket> {
        self.udp_bind_with(port, Buffers::DEFAULT)
    }

    /// Socket UDP s velikosti front (nejvetsi odeslany datagram = `tx`).
    pub fn udp_bind_with(&self, port: u16, buffers: Buffers) -> Result<UdpSocket> {
        self.udp_bind_addr(SocketAddr::from(([0, 0, 0, 0], port)), buffers)
    }

    /// Socket UDP na mistni adrese (jako [`Net::tcp_listen_addr`]).
    pub fn udp_bind_addr(&self, addr: SocketAddr, buffers: Buffers) -> Result<UdpSocket> {
        let mut w = Writer::new();
        w.socket_addr(addr);
        buffers.put(&mut w);
        let (_, ch) = one_channel(call(&self.mux, op::UDP_BIND, w, Deadline::NONE, Wait::Immediate, None)?)?;
        Ok(UdpSocket { mux: Mux::new(ch) })
    }

    /// Dotaz NTP na `server`: presny cas a posun mistnich hodin. Hodiny
    /// nenastavuje - to smi jen sluzba casu (`timed`), viz `docs/cs/cas.md`.
    pub fn ntp_query(&self, server: SocketAddr, timeout: Duration) -> Result<NtpSync> {
        let sock = self.udp_bind(0)?;
        let deadline = libos::time::deadline_after(timeout);
        let mut attempt_timeout = Duration::from_secs(1);
        loop {
            let t1 = libos::time::wall().unwrap_or(WallTime::UNIX_EPOCH);
            let mono1 = libos::time::now();
            sock.send_to(&ntp::build_request(t1), server)?;
            let wait_until = libos::time::now().saturating_add(attempt_timeout.as_nanos() as u64);
            loop {
                let until = wait_until.min(deadline);
                if libos::time::now() >= until {
                    break;
                }
                let mut buf = [0u8; 128];
                match sock.recv_from_ctx(&mut buf, Deadline(until), None) {
                    Ok((n, from)) if from == server => {
                        let t4 = libos::time::wall().unwrap_or(WallTime::UNIX_EPOCH);
                        if ntp::clock_stepped(t1, t4, libos::time::now() - mono1) {
                            // Hodiny se behem mereni posunuly: novy dotaz.
                            break;
                        }
                        match ntp::parse_reply(t1, &buf[..n]) {
                            Ok(reply) => {
                                let (offset, delay) = ntp::offset_delay(t1, &reply, t4);
                                if !ntp::plausible(&ntp::Sample { offset_ns: offset, delay_ns: delay }) {
                                    continue;
                                }
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

/// Limit cekani v ns (`u64::MAX` = bez limitu), sdileny klony socketu
/// (jako `SO_RCVTIMEO` u socketu POSIX).
struct Timeout(AtomicU64);

impl Timeout {
    const NONE: u64 = u64::MAX;

    fn new() -> Self {
        Self(AtomicU64::new(Self::NONE))
    }

    fn set(&self, t: Option<Duration>) {
        let v = t.map_or(Self::NONE, |d| u64::try_from(d.as_nanos()).unwrap_or(Self::NONE - 1).min(Self::NONE - 1));
        self.0.store(v, Ordering::Relaxed);
    }

    fn get(&self) -> Option<Duration> {
        match self.0.load(Ordering::Relaxed) {
            Self::NONE => None,
            ns => Some(Duration::from_nanos(ns)),
        }
    }
}

struct StreamShared {
    read_timeout: Timeout,
    write_timeout: Timeout,
}

/// Spojeni TCP. Zavreni (drop posledniho klonu) spojeni ukonci; data,
/// ktera sluzba prijala, jeste dorazi (sluzba spojeni zavira postupne).
///
/// Cteni a zapis z ruznych vlaken si neprekazi (sluzba drzi na spojeni
/// zvlast rozpracovany RECV a SEND). Vic ctenaru (zapisovatelu) najednou
/// sluzba obslouzi postupne ve fronte (nejvys [`crate::DIRECTION_QUEUE`]).
pub struct TcpStream {
    mux: Arc<Mux>,
    shared: Arc<StreamShared>,
    local: SocketAddr,
    remote: SocketAddr,
}

impl TcpStream {
    fn new(ch: Channel, local: SocketAddr, remote: SocketAddr) -> Self {
        Self {
            mux: Mux::new(ch),
            shared: Arc::new(StreamShared {
                read_timeout: Timeout::new(),
                write_timeout: Timeout::new(),
            }),
            local,
            remote,
        }
    }

    pub fn local_addr(&self) -> SocketAddr {
        self.local
    }

    pub fn peer_addr(&self) -> SocketAddr {
        self.remote
    }

    /// Dalsi handle na stejne spojeni (sdileny kanal a limity cekani).
    pub fn try_clone(&self) -> Result<TcpStream> {
        Ok(Self {
            mux: self.mux.clone(),
            shared: self.shared.clone(),
            local: self.local,
            remote: self.remote,
        })
    }

    /// Limit cekani na data (`None` = bez limitu). Vyprseni = `TIMED_OUT`.
    pub fn set_read_timeout(&self, t: Option<Duration>) {
        self.shared.read_timeout.set(t);
    }

    pub fn read_timeout(&self) -> Option<Duration> {
        self.shared.read_timeout.get()
    }

    /// Limit cekani na misto v odesilaci fronte.
    pub fn set_write_timeout(&self, t: Option<Duration>) {
        self.shared.write_timeout.set(t);
    }

    pub fn write_timeout(&self) -> Option<Duration> {
        self.shared.write_timeout.get()
    }

    /// Event pripravenosti spojeni (viz [`Readiness`]).
    pub fn readiness(&self) -> Result<Readiness> {
        readiness_of(&self.mux)
    }

    /// Ukonci zapis (druha strana dostane konec proudu), cteni zustava.
    pub fn shutdown_write(&self) -> Result<()> {
        call_now(&self.mux, op::SHUTDOWN, Writer::new()).map(|_| ())
    }

    /// Kanal spojeni.
    pub fn channel(&self) -> &Channel {
        self.mux.channel()
    }

    /// Kanal spojeni (napr. pro predani jinemu procesu). `BAD_STATE`, kdyz
    /// spojeni drzi klon nebo na nem bezi dotaz.
    pub fn into_channel(self) -> Result<Channel> {
        Mux::into_channel(self.mux)
    }

    /// Novy kanal ke stejnemu spojeni (`DUP`): pro jiny proces (`SCM_RIGHTS`,
    /// dedeni); spojeni zanikne az se zavrenim posledniho kanalu.
    pub fn dup(&self) -> Result<Channel> {
        dup_of(&self.mux)
    }

    /// Spojeni z kanalu, ktery prisel od jineho procesu.
    pub fn from_channel(ch: Channel) -> Result<Self> {
        let s = Self::new(ch, SocketAddr::from(([0, 0, 0, 0], 0)), SocketAddr::from(([0, 0, 0, 0], 0)));
        let b = call_now(&s.mux, op::ADDRS, Writer::new())?;
        let mut r = Reader::new(&b);
        let local = r.socket_addr()?;
        let remote = r.socket_addr()?;
        Ok(Self { local, remote, ..s })
    }

    /// Cteni s terminem a moznosti zruseni: pocet bajtu, 0 = konec proudu.
    pub fn recv_ctx(&self, buf: &mut [u8], deadline: Deadline, cancel: Option<&CancelToken>) -> Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        let mut w = Writer::new();
        w.u32(buf.len().min(MAX_IO) as u32);
        let (b, _) = call(&self.mux, op::RECV, w, deadline, Wait::Op, cancel)?;
        let n = b.len().min(buf.len());
        buf[..n].copy_from_slice(&b[..n]);
        Ok(n)
    }

    /// Cteni s limitem z [`TcpStream::set_read_timeout`].
    pub fn read(&self, buf: &mut [u8]) -> Result<usize> {
        self.recv_ctx(buf, Deadline::after(self.read_timeout()), None)
    }

    /// Zapis s terminem: pocet prijatych bajtu (cast, kdyz termin vyprsel
    /// po prijeti casti; nic = `TIMED_OUT`).
    pub fn send_ctx(&self, buf: &[u8], deadline: Deadline) -> Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        let mut w = Writer::new();
        w.bytes(&buf[..buf.len().min(MAX_IO)]);
        let (b, _) = call(&self.mux, op::SEND, w, deadline, Wait::Op, None)?;
        match Reader::new(&b).u32()? as usize {
            0 => Err(Error::TimedOut),
            n => Ok(n),
        }
    }

    /// Zapis s limitem z [`TcpStream::set_write_timeout`].
    pub fn write(&self, buf: &[u8]) -> Result<usize> {
        self.send_ctx(buf, Deadline::after(self.write_timeout()))
    }
}

impl crate::io::Read for TcpStream {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        TcpStream::read(self, buf)
    }
}

impl crate::io::Write for TcpStream {
    fn write(&mut self, buf: &[u8]) -> Result<usize> {
        TcpStream::write(self, buf)
    }
}

impl crate::io::Read for &TcpStream {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        TcpStream::read(self, buf)
    }
}

impl crate::io::Write for &TcpStream {
    fn write(&mut self, buf: &[u8]) -> Result<usize> {
        TcpStream::write(self, buf)
    }
}

/// Mistni adresa od sluzby (meni se s adresou rozhrani, napr. po DHCP).
fn local_addr_of(m: &Arc<Mux>) -> Result<SocketAddr> {
    let b = call_now(m, op::LOCAL_ADDR, Writer::new())?;
    Reader::new(&b).socket_addr()
}

/// Naslouchani TCP.
pub struct TcpListener {
    mux: Arc<Mux>,
}

impl TcpListener {
    /// Mistni adresa (pred nastavenim rozhrani 0.0.0.0:port).
    pub fn local_addr(&self) -> Result<SocketAddr> {
        local_addr_of(&self.mux)
    }

    pub fn try_clone(&self) -> Result<TcpListener> {
        Ok(Self { mux: self.mux.clone() })
    }

    /// Novy kanal ke stejnemu naslouchani (`DUP`, pro jiny proces).
    pub fn dup(&self) -> Result<Channel> {
        dup_of(&self.mux)
    }

    /// Naslouchani z kanalu od jineho procesu.
    pub fn from_channel(ch: Channel) -> Self {
        Self { mux: Mux::new(ch) }
    }

    /// Event pripravenosti (`READABLE` = ceka spojeni na `accept`).
    pub fn readiness(&self) -> Result<Readiness> {
        readiness_of(&self.mux)
    }

    /// Pocka na prichozi spojeni.
    pub fn accept(&self) -> Result<(TcpStream, SocketAddr)> {
        self.accept_ctx(Deadline::NONE, None)
    }

    pub fn accept_timeout(&self, timeout: Option<Duration>) -> Result<(TcpStream, SocketAddr)> {
        self.accept_ctx(Deadline::after(timeout), None)
    }

    /// Prijeti spojeni s terminem a moznosti zruseni.
    pub fn accept_ctx(&self, deadline: Deadline, cancel: Option<&CancelToken>) -> Result<(TcpStream, SocketAddr)> {
        let (b, ch) = one_channel(call(&self.mux, op::ACCEPT, Writer::new(), deadline, Wait::Op, cancel)?)?;
        let mut r = Reader::new(&b);
        let remote = r.socket_addr()?;
        let local = r.socket_addr()?;
        Ok((TcpStream::new(ch, local, remote), remote))
    }
}

/// Socket UDP.
pub struct UdpSocket {
    mux: Arc<Mux>,
}

impl UdpSocket {
    /// Mistni adresa (pred nastavenim rozhrani 0.0.0.0:port).
    pub fn local_addr(&self) -> Result<SocketAddr> {
        local_addr_of(&self.mux)
    }

    pub fn try_clone(&self) -> Result<UdpSocket> {
        Ok(Self { mux: self.mux.clone() })
    }

    /// Novy kanal ke stejnemu socketu (`DUP`, pro jiny proces).
    pub fn dup(&self) -> Result<Channel> {
        dup_of(&self.mux)
    }

    /// Socket z kanalu od jineho procesu.
    pub fn from_channel(ch: Channel) -> Self {
        Self { mux: Mux::new(ch) }
    }

    /// Event pripravenosti (`READABLE` = datagram, `WRITABLE` = misto).
    pub fn readiness(&self) -> Result<Readiness> {
        readiness_of(&self.mux)
    }

    /// Posle datagram (nejvys [`MAX_DATAGRAM`] bajtu).
    pub fn send_to(&self, data: &[u8], to: SocketAddr) -> Result<()> {
        if data.len() > MAX_DATAGRAM {
            return Err(Error::OutOfRange);
        }
        let mut w = Writer::new();
        w.socket_addr(to).bytes(data);
        // Muze cekat na dotaz firewallu (neznamy cil) a na misto v plne
        // fronte socketu (sluzba nejvys 5 s).
        call(&self.mux, op::SEND_TO, w, Deadline::after(Some(Duration::from_secs(5))), Wait::Gated, None).map(|_| ())
    }

    /// Prijme datagram: (delka, odesilatel). Delsi datagram se zkrati.
    pub fn recv_from(&self, buf: &mut [u8], timeout: Option<Duration>) -> Result<(usize, SocketAddr)> {
        self.recv_from_ctx(buf, Deadline::after(timeout), None)
    }

    /// Prijeti datagramu s terminem a moznosti zruseni.
    pub fn recv_from_ctx(
        &self,
        buf: &mut [u8],
        deadline: Deadline,
        cancel: Option<&CancelToken>,
    ) -> Result<(usize, SocketAddr)> {
        let mut w = Writer::new();
        w.u32(buf.len().min(MAX_DATAGRAM) as u32);
        let (b, _) = call(&self.mux, op::RECV_FROM, w, deadline, Wait::Op, cancel)?;
        let mut r = Reader::new(&b);
        let from = r.socket_addr()?;
        let data = r.rest();
        let n = data.len().min(buf.len());
        buf[..n].copy_from_slice(&data[..n]);
        Ok((n, from))
    }
}
