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
//! - `closed` (TCP): port, na kterem nic neposloucha (odmitnuti spojeni).
//!   `refuses` = 1, kdyz se od slirp ceka odmitnuti (RST, Linux a macOS).
//!   Slirp na Windows zatim neohlasi nic (SYN zustane bez odpovedi); test
//!   tam prijme odmitnuti i vyprseni, takze novejsi QEMU ho nerozbije.
//! - `reset` (TCP): prijme spojeni, pocka na data a zavre socket
//!   s neprectenymi daty - system posle RST (preruseni spojeni).
//! - `callback` (TCP): po prijeti radku se pripoji do hosta pres `hostfwd`
//!   na port `forwarded`, posle `ahoj z hostitele` a zavre.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{IpAddr, Ipv4Addr, TcpListener, TcpStream, UdpSocket};
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
}

impl Helper {
    pub fn release_forwarded_port(&mut self) {
        self.hold = None;
    }
}

fn localhost() -> IpAddr {
    IpAddr::V4(Ipv4Addr::LOCALHOST)
}

/// Volny port na 127.0.0.1 (zjisteny navazanim a uvolnenim). Port pro
/// test odmitnuti zustane volny: nahodny docasny port, ktery by jiny proces
/// behem testu zacal poslouchat, je velmi nepravdepodobny.
fn free_tcp_port() -> std::io::Result<u16> {
    Ok(TcpListener::bind((localhost(), 0))?.local_addr()?.port())
}

pub fn start() -> std::io::Result<Helper> {
    let echo = TcpListener::bind((localhost(), 0))?;
    let udp_echo = UdpSocket::bind((localhost(), 0))?;
    let (dns_sock, dns_tcp) = dns_sockets()?;
    let ntp_sock = UdpSocket::bind((localhost(), 0))?;
    let callback = TcpListener::bind((localhost(), 0))?;
    let reset = TcpListener::bind((localhost(), 0))?;
    let closed = free_tcp_port()?;
    let hold = TcpListener::bind((localhost(), 0))?;
    let host_fwd = hold.local_addr()?.port();

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
    thread::spawn(move || {
        for conn in callback.incoming().flatten() {
            thread::spawn(move || connect_back(conn, host_fwd));
        }
    });

    Ok(Helper {
        config,
        hostfwd: format!("tcp:127.0.0.1:{host_fwd}-:{GUEST_FORWARDED}"),
        hold: Some(hold),
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
    while let Ok((n, from)) = sock.recv_from(&mut buf) {
        if let Some(reply) = answer(&buf[..n]) {
            let _ = sock.send_to(&reply, from);
        }
    }
}

/// Port DNS pro UDP i TCP (slirp preklada oboji na stejny port hostitele).
fn dns_sockets() -> std::io::Result<(UdpSocket, TcpListener)> {
    let mut last = None;
    for _ in 0..20 {
        let udp = UdpSocket::bind((localhost(), 0))?;
        match TcpListener::bind(udp.local_addr()?) {
            Ok(tcp) => return Ok((udp, tcp)),
            Err(e) => last = Some(e),
        }
    }
    Err(last.unwrap())
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
