//! Loopback v netd (127.0.0.0/8 a ::1, `docs/cs/sit.md`, kapitola
//! Loopback; sada `sit`): bez linky, stejna opravneni relace jako sit.
//!
//! - TCP pres 127.0.0.1 i ::1: spojeni, data obema smery, konec zapisu,
//!   zavreni (konec proudu u protejsku), adresy obou stran,
//! - naslouchani na 0.0.0.0 prijima i z loopbacku,
//! - nikdo nenasloucha -> `CONNECTION_REFUSED`,
//! - UDP pres 127.0.0.1 i ::1 (adresa odesilatele),
//! - omezena relace bez pravidla pro loopback -> `ACCESS_DENIED`.

use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use core::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use core::time::Duration;

use libos::{Error, Startup};
use netproto::{Buffers, Policy, Rule, Target, TcpStream, proto, rights};

use super::net::net;
use super::{Test, TestResult, fail};
use crate::{check, check_eq};

pub const TESTS: &[Test] = &[
    ("loopback: TCP 127.0.0.1 - data, konec zapisu, zavreni", tcp_v4),
    ("loopback: TCP ::1", tcp_v6),
    ("loopback: naslouchani na 0.0.0.0 prijima z 127.0.0.1", tcp_any),
    ("loopback: bez naslouchani -> CONNECTION_REFUSED", tcp_refused),
    ("loopback: UDP 127.0.0.1 a ::1", udp),
    ("loopback: omezena relace -> ACCESS_DENIED", restricted),
];

const V4: IpAddr = IpAddr::V4(Ipv4Addr::LOCALHOST);
const V6: IpAddr = IpAddr::V6(Ipv6Addr::LOCALHOST);
const T: Option<Duration> = Some(Duration::from_secs(10));

fn read_exact(s: &TcpStream, n: usize) -> Result<Vec<u8>, String> {
    let mut out = vec![0u8; n];
    let mut got = 0;
    while got < n {
        let k = s.read(&mut out[got..]).map_err(|e| fail("cteni", e))?;
        if k == 0 {
            return Err(format!("konec proudu po {got} z {n} B"));
        }
        got += k;
    }
    Ok(out)
}

fn write_all(s: &TcpStream, data: &[u8]) -> TestResult {
    let mut done = 0;
    while done < data.len() {
        done += s.write(&data[done..]).map_err(|e| fail("zapis", e))?;
    }
    Ok(())
}

/// Spojeni na `ip` (naslouchani na `bind`): data obema smery po kouskach
/// (vic nez buffer), konec zapisu a zavreni.
fn tcp_pair(bind: IpAddr, ip: IpAddr) -> TestResult {
    let net = net()?;
    let l = net.tcp_listen_addr(SocketAddr::new(bind, 0), 4, Buffers::DEFAULT).map_err(|e| fail("naslouchani", e))?;
    let la = l.local_addr().map_err(|e| fail("adresa naslouchani", e))?;
    check!(la.port() != 0, format!("port naslouchani {la}"));
    let target = SocketAddr::new(ip, la.port());
    let c = net.tcp_connect(target, T).map_err(|e| fail("spojeni", e))?;
    let (srv, peer) = l.accept_timeout(T).map_err(|e| fail("prijeti", e))?;
    check_eq!(peer, c.local_addr(), "adresa klienta u serveru");
    check_eq!(c.peer_addr(), target, "cil u klienta");
    check!(c.local_addr().ip() == ip, format!("mistni adresa klienta {}", c.local_addr()));
    // 256 KiB obema smery po 16 KiB (buffer 64 KiB: jedno vlakno).
    let chunk: Vec<u8> = (0..16 * 1024).map(|i| (i * 7 % 251) as u8).collect();
    for round in 0..16 {
        write_all(&c, &chunk)?;
        let got = read_exact(&srv, chunk.len())?;
        check!(got == chunk, format!("data klient -> server, kolo {round}"));
        write_all(&srv, &chunk)?;
        let got = read_exact(&c, chunk.len())?;
        check!(got == chunk, format!("data server -> klient, kolo {round}"));
    }
    // Konec zapisu klienta: server cte konec proudu, opacny smer funguje.
    c.shutdown_write().map_err(|e| fail("shutdown", e))?;
    let mut b = [0u8; 8];
    check_eq!(srv.read(&mut b).map_err(|e| fail("cteni po shutdown", e))?, 0, "konec proudu u serveru");
    write_all(&srv, b"po shutdown")?;
    check!(read_exact(&c, 11)? == b"po shutdown", "data po konci zapisu klienta");
    // Zavreni serveru: klient cte konec proudu.
    drop(srv);
    check_eq!(c.read(&mut b).map_err(|e| fail("cteni po zavreni", e))?, 0, "konec proudu u klienta");
    Ok(())
}

fn tcp_v4(_s: &Startup) -> TestResult {
    tcp_pair(V4, V4)
}

fn tcp_v6(_s: &Startup) -> TestResult {
    tcp_pair(V6, V6)
}

fn tcp_any(_s: &Startup) -> TestResult {
    tcp_pair(IpAddr::V4(Ipv4Addr::UNSPECIFIED), V4)
}

fn tcp_refused(_s: &Startup) -> TestResult {
    let net = net()?;
    // Port, na kterem nikdo nenasloucha: obsadit a uvolnit.
    let l = net.tcp_listen_addr(SocketAddr::new(V4, 0), 1, Buffers::DEFAULT).map_err(|e| fail("naslouchani", e))?;
    let port = l.local_addr().map_err(|e| fail("adresa", e))?.port();
    drop(l);
    for ip in [V4, V6] {
        check_eq!(net.tcp_connect(SocketAddr::new(ip, port), T).err(), Some(Error::ConnectionRefused), format!("spojeni na {ip}:{port}"));
    }
    Ok(())
}

fn udp(_s: &Startup) -> TestResult {
    let net = net()?;
    for ip in [V4, V6] {
        let a = net.udp_bind_addr(SocketAddr::new(ip, 0), Buffers::DEFAULT).map_err(|e| fail("UDP a", e))?;
        let b = net.udp_bind_addr(SocketAddr::new(ip, 0), Buffers::DEFAULT).map_err(|e| fail("UDP b", e))?;
        let (aa, ba) = (a.local_addr().map_err(|e| fail("adresa a", e))?, b.local_addr().map_err(|e| fail("adresa b", e))?);
        check!(aa.ip() == ip && ba.ip() == ip && aa.port() != ba.port(), format!("adresy {aa} {ba}"));
        a.send_to(b"ahoj", ba).map_err(|e| fail("send_to", e))?;
        let mut buf = [0u8; 64];
        let (n, from) = b.recv_from(&mut buf, T).map_err(|e| fail("recv_from", e))?;
        check!(&buf[..n] == b"ahoj" && from == aa, format!("datagram {:?} od {from} (cekano {aa})", &buf[..n]));
        b.send_to(b"zpet", aa).map_err(|e| fail("odpoved", e))?;
        let (n, from) = a.recv_from(&mut buf, T).map_err(|e| fail("prijem odpovedi", e))?;
        check!(&buf[..n] == b"zpet" && from == ba, format!("odpoved {:?} od {from}", &buf[..n]));
    }
    Ok(())
}

fn restricted(_s: &Startup) -> TestResult {
    let net = net()?;
    let l = net.tcp_listen_addr(SocketAddr::new(V4, 0), 1, Buffers::DEFAULT).map_err(|e| fail("naslouchani", e))?;
    let port = l.local_addr().map_err(|e| fail("adresa", e))?.port();
    // Relace smi jen TCP na jiny port: loopback stejna pravidla jako sit.
    let policy = Policy::only(vec![Rule::port(proto::TCP, Target::Any, port.wrapping_add(1))]);
    let app = net.restrict(rights::CLIENT, &policy).map_err(|e| fail("restrict", e))?;
    check_eq!(app.tcp_connect(SocketAddr::new(V4, port), T).err(), Some(Error::AccessDenied), "loopback mimo pravidla");
    Ok(())
}
