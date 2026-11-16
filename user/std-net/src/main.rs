//! std-net: test `std::net` nad netd (docs/cs/rust-std.md, faze 2).
//!
//! Spousti ho test v initu (`user/init/src/tests/std_net.rs`) s omezenou
//! relaci site (TCP jen na echo server hostitele, UDP na echo a DNS)
//! a promennymi `ECHO=<adresa:port>`, `UDP_ECHO=...`, `CLOSED=...`
//! (port mimo povoleni) a `NAME=<jmeno:port>` (preklad pres DNS).
//! Kazda kontrola vypise `ok <jmeno>`, chyba `FAIL <jmeno>: <popis>` na
//! stderr a nenulovy kod. Posledni radek uspechu: `std-net: hotovo`.

use std::io::{ErrorKind, Read, Write};
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream, ToSocketAddrs, UdpSocket};
use std::thread;
use std::time::Duration;

type Check = Result<(), String>;

fn ensure(cond: bool, msg: impl Into<String>) -> Check {
    if cond { Ok(()) } else { Err(msg.into()) }
}

fn addr(var: &str) -> Result<SocketAddr, String> {
    let v = std::env::var(var).map_err(|_| format!("chybi {var}"))?;
    v.parse().map_err(|e| format!("{var}={v}: {e}"))
}

fn pattern(len: usize, seed: u8) -> Vec<u8> {
    (0..len).map(|i| (i as u8).wrapping_mul(31).wrapping_add(seed)).collect()
}

fn connect(a: SocketAddr) -> Result<TcpStream, String> {
    let s = TcpStream::connect_timeout(&a, Duration::from_secs(10)).map_err(|e| format!("connect {a}: {e}"))?;
    s.set_read_timeout(Some(Duration::from_secs(10))).map_err(|e| e.to_string())?;
    s.set_write_timeout(Some(Duration::from_secs(10))).map_err(|e| e.to_string())?;
    Ok(s)
}

fn tcp_echo() -> Check {
    let echo = addr("ECHO")?;
    let mut s = connect(echo)?;
    ensure(s.peer_addr().map_err(|e| e.to_string())? == echo, "peer_addr")?;
    let data = pattern(64 * 1024, 7);
    let mut back = vec![0u8; data.len()];
    for (chunk, out) in data.chunks(8 * 1024).zip(back.chunks_mut(8 * 1024)) {
        s.write_all(chunk).map_err(|e| format!("zapis: {e}"))?;
        s.read_exact(out).map_err(|e| format!("cteni: {e}"))?;
    }
    ensure(back == data, "echo vratilo jina data")?;
    s.shutdown(Shutdown::Write).map_err(|e| format!("shutdown: {e}"))?;
    let mut rest = Vec::new();
    s.read_to_end(&mut rest).map_err(|e| format!("cteni konce: {e}"))?;
    ensure(rest.is_empty(), format!("po konci {} B navic", rest.len()))?;
    println!("tcp: 64 KiB tam a zpet");
    Ok(())
}

/// Ctyri vlakna, kazde vlastni spojeni pres spolecnou relaci site.
fn tcp_threads() -> Check {
    let echo = addr("ECHO")?;
    let handles: Vec<_> = (0..4u8)
        .map(|t| {
            thread::spawn(move || -> Check {
                let mut s = connect(echo)?;
                let data = pattern(16 * 1024, t);
                s.write_all(&data).map_err(|e| format!("vlakno {t} zapis: {e}"))?;
                let mut back = vec![0u8; data.len()];
                s.read_exact(&mut back).map_err(|e| format!("vlakno {t} cteni: {e}"))?;
                ensure(back == data, format!("vlakno {t}: jina data"))
            })
        })
        .collect();
    for h in handles {
        h.join().map_err(|_| "vlakno spadlo")??;
    }
    Ok(())
}

fn denied() -> Check {
    let closed = addr("CLOSED")?;
    match TcpStream::connect_timeout(&closed, Duration::from_secs(5)) {
        Err(e) if e.kind() == ErrorKind::PermissionDenied => Ok(()),
        Err(e) => Err(format!("port mimo povoleni: {e} (cekano PermissionDenied)")),
        Ok(_) => Err("spojeni mimo povoleni proslo".into()),
    }
}

fn udp_echo() -> Check {
    let to = addr("UDP_ECHO")?;
    let s = UdpSocket::bind("0.0.0.0:0").map_err(|e| format!("bind: {e}"))?;
    s.set_read_timeout(Some(Duration::from_secs(5))).map_err(|e| e.to_string())?;
    s.connect(to).map_err(|e| format!("connect: {e}"))?;
    for i in 0..5u8 {
        let msg = pattern(512, i);
        s.send(&msg).map_err(|e| format!("send: {e}"))?;
        let mut buf = [0u8; 1024];
        let n = s.recv(&mut buf).map_err(|e| format!("recv {i}: {e}"))?;
        ensure(buf[..n] == msg[..], format!("datagram {i}"))?;
    }
    Ok(())
}

fn dns() -> Check {
    let name = std::env::var("NAME").map_err(|_| "chybi NAME")?;
    let echo = addr("ECHO")?;
    let addrs: Vec<SocketAddr> =
        (name.as_str(), echo.port()).to_socket_addrs().map_err(|e| format!("{name}: {e}"))?.collect();
    ensure(addrs.iter().any(|a| a.ip() == echo.ip()), format!("{name} -> {addrs:?}"))?;
    // Spojeni na jmeno (TcpStream::connect s retezcem "jmeno:port").
    let host = format!("{name}:{}", echo.port());
    let mut s = TcpStream::connect(&host).map_err(|e| format!("connect {host}: {e}"))?;
    s.write_all(b"ahoj").map_err(|e| e.to_string())?;
    let mut b = [0u8; 4];
    s.set_read_timeout(Some(Duration::from_secs(10))).map_err(|e| e.to_string())?;
    s.read_exact(&mut b).map_err(|e| e.to_string())?;
    ensure(&b == b"ahoj", "echo pres jmeno")?;
    println!("dns: {name} -> {}", addrs[0].ip());
    Ok(())
}

/// Plny duplex (docs/cs/rust-std.md, Sit): vlakno blokovane ve `read`
/// (klon pres `try_clone`) nezdrzi zapis na stejnem spojeni (netproto v2,
/// osrt socket nezamyka). Cteni dostane echo zapisu.
fn shared_stream_duplex() -> Check {
    let echo = addr("ECHO")?;
    let s = connect(echo)?;
    let mut reader = s.try_clone().map_err(|e| format!("try_clone: {e}"))?;
    reader.set_read_timeout(Some(Duration::from_secs(2))).map_err(|e| e.to_string())?;
    let t = thread::spawn(move || {
        let mut b = [0u8; 4];
        // Echo nic neposle, dokud nezapiseme: cteni ceka na zapis.
        reader.read(&mut b).map(|n| b[..n].to_vec()).map_err(|e| e.kind())
    });
    // Cteni nejdriv zabere spojeni.
    thread::sleep(Duration::from_millis(300));
    let start = std::time::Instant::now();
    (&s).write_all(b"x").map_err(|e| format!("zapis: {e}"))?;
    let waited = start.elapsed();
    let r = t.join().map_err(|_| "vlakno cteni spadlo")?;
    ensure(r == Ok(b"x".to_vec()), format!("cteni: {r:?} (cekano echo x)"))?;
    ensure(waited < Duration::from_millis(1200), format!("zapis cekal na cteni {waited:?}"))?;
    println!("sdilene spojeni: zapis za {} ms, cteni dostalo echo", waited.as_millis());
    Ok(())
}

/// Naslouchani bez prava LISTEN: chyba opravneni, ne panika.
fn listen_denied() -> Check {
    match TcpListener::bind("0.0.0.0:0") {
        Err(e) if e.kind() == ErrorKind::PermissionDenied => Ok(()),
        Err(e) => Err(format!("listen: {e} (cekano PermissionDenied)")),
        Ok(_) => Err("listen bez prava proslo".into()),
    }
}

fn main() {
    let checks: [(&str, fn() -> Check); 7] = [
        ("tcp_echo", tcp_echo),
        ("tcp_threads", tcp_threads),
        ("denied", denied),
        ("udp_echo", udp_echo),
        ("dns", dns),
        ("listen_denied", listen_denied),
        ("shared_stream_duplex", shared_stream_duplex),
    ];
    let mut failed = 0;
    for (name, check) in checks {
        match check() {
            Ok(()) => println!("ok {name}"),
            Err(e) => {
                eprintln!("FAIL {name}: {e}");
                failed += 1;
            }
        }
    }
    if failed > 0 {
        std::process::exit(1);
    }
    println!("std-net: hotovo");
}
