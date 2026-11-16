//! Pomocne servery na hostiteli pro testy site (`cargo xtask test`).
//!
//! Host je v user-mode siti QEMU (slirp) a hostitele vidi jako 10.0.2.2:
//! spojeni a datagramy na 10.0.2.2 slirp preklada na 127.0.0.1 hostitele.
//! Servery posloucha jen na 127.0.0.1 na volnych portech (soucasne behy
//! testu v jinych pracovnich kopiich si nevadi). Porty dostane host
//! v souboru `net-test` v initfs.
//!
//! - `echo` (TCP): vraci, co prijme; po konci zapisu klienta spojeni zavre.
//! - `udp_echo` (UDP): vraci datagramy odesilateli.
//! - `dns` (UDP i TCP na stejnem portu): zna `test.magnumopus` (10.0.2.77),
//!   `echo.magnumopus` (10.0.2.2), `many.magnumopus` (100 adres, odpoved
//!   vetsi nez 512 B - vejde se jen s EDNS0) a `tcp.magnumopus` (10.0.2.88,
//!   pres UDP jen zkracena odpoved - klient se musi zeptat pres TCP);
//!   ostatni jmena NXDOMAIN.
//! - `ntp` (UDP): server NTP s casem hostitele.
//! - `closed` (TCP): vyhrazeny port bez `listen` (odmitnuti spojeni);
//!   xtask ho drzi po cely beh, takze ho nikdo jiny neotevre.
//!   `refuses` = 1, kdyz se od slirp ceka odmitnuti (RST, Linux a macOS).
//!   Slirp na Windows zatim neohlasi nic (SYN zustane bez odpovedi); test
//!   tam prijme odmitnuti i vyprseni, takze novejsi QEMU ho nerozbije.
//! - `reset` (TCP): prijme spojeni, pocka na data a zavre socket
//!   s neprectenymi daty - system posle RST (preruseni spojeni).
//! - `callback` (TCP): po prijeti radku se pripoji do hosta pres `hostfwd`
//!   na port `forwarded`, posle `ahoj z hostitele` a zavre.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{IpAddr, Ipv4Addr, TcpListener, TcpStream, UdpSocket};
use std::sync::Arc;
use std::sync::atomic::{AtomicU16, Ordering};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use netproto::{dns, ntp};

/// Port v hostovi, na ktery slirp presmeruje spojeni z hostitele.
pub const GUEST_FORWARDED: u16 = 8080;

pub struct Helper {
    /// Obsah souboru `net-test` pro initfs.
    pub config: String,
    /// Argument `hostfwd` pro `-netdev user`.
    pub hostfwd: String,
    /// Port pro `hostfwd` drzeny obsazeny, aby ho mezitim nevzal jiny
    /// proces (soubezny beh testu). Uvolnit tesne pred startem QEMU
    /// ([`Helper::release_forwarded_port`]), QEMU ho pak hned obsadi.
    hold: Option<TcpListener>,
    /// Aktualni port `hostfwd` (sdileny s vlaknem `callback`; pri novem
    /// startu QEMU se zmeni, viz [`Helper::rearm_forwarded_port`]).
    host_fwd: Arc<AtomicU16>,
    /// Port pro test odmitnuti spojeni: navazany, ale bez `listen`, po
    /// celou dobu behu. Spojeni na nej jadro odmitne (RST) a zadny jiny
    /// proces ho mezitim nemuze obsadit.
    _closed: socket2::Socket,
    /// Dalsi presmerovani (`hostfwd=...`), ktera prezijou novy port
    /// [`Helper::rearm_forwarded_port`] (test SSH).
    extra: Vec<String>,
}

impl Helper {
    /// Prida presmerovani do argumentu `-netdev user` (napr. port 22 hosta).
    pub fn add_forward(&mut self, f: &str) {
        self.extra.push(f.to_string());
        self.hostfwd = format!("{},{f}", self.hostfwd);
    }

    pub fn release_forwarded_port(&mut self) {
        self.hold = None;
    }

    /// Novy port pro `hostfwd` (QEMU nenastartovalo, port mezitim nekdo
    /// obsadil); vraci novy argument `hostfwd`.
    pub fn rearm_forwarded_port(&mut self) -> std::io::Result<String> {
        let hold = tcp_listener("hostfwd")?;
        let port = hold.local_addr()?.port();
        self.host_fwd.store(port, Ordering::Relaxed);
        self.hold = Some(hold);
        self.hostfwd = hostfwd_arg(port);
        for f in &self.extra {
            self.hostfwd = format!("{},{f}", self.hostfwd);
        }
        Ok(self.hostfwd.clone())
    }
}

fn hostfwd_arg(port: u16) -> String {
    format!("tcp:127.0.0.1:{port}-:{GUEST_FORWARDED}")
}

fn localhost() -> IpAddr {
    IpAddr::V4(Ipv4Addr::LOCALHOST)
}

/// Opakovani otevreni socketu pomocneho serveru. Windows obcas odmitne
/// i dynamicky port (os error 10013 - port ve vyhrazenem rozsahu, napr.
/// Hyper-V); dalsi pokus dostane jiny.
const BIND_ATTEMPTS: usize = 20;

fn retry<T>(what: &str, mut f: impl FnMut() -> std::io::Result<T>) -> std::io::Result<T> {
    let mut last = None;
    for _ in 0..BIND_ATTEMPTS {
        match f() {
            Ok(v) => return Ok(v),
            Err(e) => last = Some(e),
        }
    }
    let e = last.unwrap();
    Err(std::io::Error::new(e.kind(), format!("pomocny server {what}: {e}")))
}

/// Naslouchani TCP na volnem portu 127.0.0.1 (s opakovanim, viz `retry`).
pub fn tcp_listener(what: &str) -> std::io::Result<TcpListener> {
    retry(what, || TcpListener::bind((localhost(), 0)))
}

/// Socket UDP na volnem portu 127.0.0.1 (s opakovanim).
pub fn udp_socket(what: &str) -> std::io::Result<UdpSocket> {
    retry(what, || UdpSocket::bind((localhost(), 0)))
}

/// Vyhrazeny zavreny port na 127.0.0.1: socket TCP navazany bez `listen`.
/// Drzi ho `Helper` az do konce behu QEMU.
fn closed_tcp_port() -> std::io::Result<(socket2::Socket, u16)> {
    use socket2::{Domain, Protocol, Socket, Type};
    retry("closed", || {
        let s = Socket::new(Domain::IPV4, Type::STREAM, Some(Protocol::TCP))?;
        s.bind(&std::net::SocketAddr::new(localhost(), 0).into())?;
        let port = s.local_addr()?.as_socket().map(|a| a.port()).unwrap_or(0);
        Ok((s, port))
    })
}

pub fn start() -> std::io::Result<Helper> {
    let echo = tcp_listener("echo")?;
    let udp_echo = udp_socket("udp_echo")?;
    let (dns_sock, dns_tcp) = dns_sockets()?;
    let ntp_sock = udp_socket("ntp")?;
    let callback = tcp_listener("callback")?;
    let reset = tcp_listener("reset")?;
    let (closed_sock, closed) = closed_tcp_port()?;
    let hold = tcp_listener("hostfwd")?;
    let host_fwd = Arc::new(AtomicU16::new(hold.local_addr()?.port()));

    let config = format!(
        "echo = {}\nudp_echo = {}\ndns = {}\nntp = {}\nclosed = {}\nrefuses = {}\nreset = {}\ncallback = {}\nforwarded = {}\n",
        echo.local_addr()?.port(),
        udp_echo.local_addr()?.port(),
        dns_sock.local_addr()?.port(),
        ntp_sock.local_addr()?.port(),
        closed,
        u8::from(!cfg!(windows)),
        reset.local_addr()?.port(),
        callback.local_addr()?.port(),
        GUEST_FORWARDED,
    );

    thread::spawn(move || {
        for conn in echo.incoming().flatten() {
            thread::spawn(move || tcp_echo(conn));
        }
    });
    thread::spawn(move || udp_loop(udp_echo, |req| Some(req.to_vec())));
    thread::spawn(move || udp_loop(dns_sock, |q| dns_answer(q, false)));
    thread::spawn(move || {
        for conn in dns_tcp.incoming().flatten() {
            thread::spawn(move || dns_tcp_conn(conn));
        }
    });
    thread::spawn(move || udp_loop(ntp_sock, ntp_answer));
    thread::spawn(move || {
        for conn in reset.incoming().flatten() {
            thread::spawn(move || reset_after_data(conn));
        }
    });
    let fwd = host_fwd.clone();
    thread::spawn(move || {
        for conn in callback.incoming().flatten() {
            let port = fwd.load(Ordering::Relaxed);
            thread::spawn(move || connect_back(conn, port));
        }
    });

    Ok(Helper {
        config,
        hostfwd: hostfwd_arg(host_fwd.load(Ordering::Relaxed)),
        hold: Some(hold),
        host_fwd,
        _closed: closed_sock,
        extra: Vec::new(),
    })
}

fn tcp_echo(mut conn: TcpStream) {
    let _ = conn.set_read_timeout(Some(Duration::from_secs(60)));
    let mut buf = [0u8; 16 * 1024];
    loop {
        match conn.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                if conn.write_all(&buf[..n]).is_err() {
                    return;
                }
            }
        }
    }
    let _ = conn.shutdown(std::net::Shutdown::Both);
}

/// Pocka na data od klienta a zavre spojeni, aniz je precte: neprectena
/// data v prijimaci fronte = system posle RST misto FIN.
fn reset_after_data(conn: TcpStream) {
    let mut probe = [0u8; 1];
    let _ = conn.set_read_timeout(Some(Duration::from_secs(10)));
    // `peek` data ve fronte necha.
    let _ = conn.peek(&mut probe);
    thread::sleep(Duration::from_millis(100));
    drop(conn);
}

fn udp_loop(sock: UdpSocket, answer: impl Fn(&[u8]) -> Option<Vec<u8>>) {
    let mut buf = vec![0u8; 65536];
    loop {
        // Chyba prijmu neukoncuje server: Windows po ICMP "port unreachable"
        // (odpoved na uz zavreny port hosta) vrati z dalsiho recv_from
        // WSAECONNRESET. Driv tim server skoncil a dalsi dotazy visely.
        let Ok((n, from)) = sock.recv_from(&mut buf) else {
            // Trvala chyba (zavreny socket) nesmi tocit procesor.
            thread::sleep(Duration::from_millis(1));
            continue;
        };
        if let Some(reply) = answer(&buf[..n]) {
            let _ = sock.send_to(&reply, from);
        }
    }
}

/// Port DNS pro UDP i TCP (slirp preklada oboji na stejny port hostitele).
/// UDP i TCP na stejnem portu (zkracena odpoved -> dotaz pres TCP na tentyz
/// port). Prvni pokusy: docasny port TCP od jadra (ten se vyhyba vyhrazenym
/// rozsahum TCP, napr. Hyper-V na Windows) a k nemu UDP. Jadro ale prideluje
/// docasne porty po sobe, takze kdyz je UDP v bloku obsazenem jinak, dalsi
/// pokusy se mu nevyhnou - zbyle pokusy proto zkousi nahodne porty
/// z dynamickeho rozsahu.
fn dns_sockets() -> std::io::Result<(UdpSocket, TcpListener)> {
    let mut attempt = 0u32;
    retry("dns", || {
        attempt += 1;
        let tcp = if attempt <= 5 {
            TcpListener::bind((localhost(), 0))?
        } else {
            let mut b = [0u8; 2];
            ring::rand::SecureRandom::fill(&ring::rand::SystemRandom::new(), &mut b)
                .map_err(|_| std::io::Error::other("nahodne cislo"))?;
            TcpListener::bind((localhost(), 49152 + u16::from_le_bytes(b) % 16384))?
        };
        let udp = UdpSocket::bind(tcp.local_addr()?)?;
        Ok((udp, tcp))
    })
}

fn dns_answer(req: &[u8], tcp: bool) -> Option<Vec<u8>> {
    let q = dns::parse_query(req).ok()?;
    let addrs: Vec<IpAddr> = match q.name.to_ascii_lowercase().as_str() {
        "test.magnumopus" => vec![IpAddr::V4(Ipv4Addr::new(10, 0, 2, 77))],
        "echo.magnumopus" => vec![IpAddr::V4(Ipv4Addr::new(10, 0, 2, 2))],
        "many.magnumopus" => (0..100u8).map(|i| IpAddr::V4(Ipv4Addr::new(10, 9, 0, i))).collect(),
        "tcp.magnumopus" if !tcp => return Some(dns::build_truncated(&q)),
        "tcp.magnumopus" => vec![IpAddr::V4(Ipv4Addr::new(10, 0, 2, 88))],
        _ => return Some(dns::build_response(&q, &[], 0, dns::RCODE_NXDOMAIN)),
    };
    let r = dns::build_response(&q, &addrs, 60, dns::RCODE_OK);
    // Pres UDP jen tolik, kolik klient nabidl (EDNS0, jinak 512 B).
    if !tcp && r.len() > dns::query_udp_size(req) {
        return Some(dns::build_truncated(&q));
    }
    Some(r)
}

/// DNS pres TCP: zpravy s delkou u16 na zacatku.
fn dns_tcp_conn(mut conn: TcpStream) {
    let _ = conn.set_read_timeout(Some(Duration::from_secs(10)));
    loop {
        let mut len = [0u8; 2];
        if conn.read_exact(&mut len).is_err() {
            return;
        }
        let mut q = vec![0u8; u16::from_be_bytes(len) as usize];
        if conn.read_exact(&mut q).is_err() {
            return;
        }
        let Some(r) = dns_answer(&q, true) else {
            return;
        };
        let mut out = (r.len() as u16).to_be_bytes().to_vec();
        out.extend_from_slice(&r);
        if conn.write_all(&out).is_err() {
            return;
        }
    }
}

fn wall_now() -> abi::time::WallTime {
    let d = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default();
    abi::time::WallTime {
        secs: d.as_secs() as i64,
        nanos: d.subsec_nanos(),
    }
}

fn ntp_answer(req: &[u8]) -> Option<Vec<u8>> {
    let now = wall_now();
    ntp::build_reply(req, now, wall_now()).ok().map(|r| r.to_vec())
}

fn connect_back(ctl: TcpStream, host_fwd: u16) {
    let _ = ctl.set_read_timeout(Some(Duration::from_secs(30)));
    let mut line = String::new();
    if BufReader::new(&ctl).read_line(&mut line).is_err() {
        return;
    }
    if let Ok(mut c) = TcpStream::connect((localhost(), host_fwd)) {
        let _ = c.write_all(b"ahoj z hostitele\n");
        let _ = c.shutdown(std::net::Shutdown::Write);
        // Pockat, az host spojeni zavre, at data nezahodi RST.
        let mut sink = [0u8; 64];
        let _ = c.set_read_timeout(Some(Duration::from_secs(10)));
        while let Ok(n) = c.read(&mut sink) {
            if n == 0 {
                break;
            }
        }
    }
}
