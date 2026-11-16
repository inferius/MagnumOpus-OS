//! nettools: sitove prikazy shellu v jednom programu, chovani podle jmena,
//! pod kterym bezi (`argv[0]`). V initfs jako `ping`, `nc` a `resolve`,
//! kazdy s vlastnim manifestem (`*.manifest.toml`): vlastni id aplikace
//! pro firewall a jen potrebna prava site. Spousti je shell pres sluzbu
//! exec initu (`docs/cs/navrh-shell.md`, kapitola Programy).
//!
//! Prikazy a prepinace anglicky, hlaseni cesky. Navratovy kod: 0 uspech,
//! 1 cil neodpovedel / spojeni selhalo, 2 spatne pouziti nebo bez site.

#![feature(magnumopus_runtime, rustc_private)]

use std::io::{BufRead, Read, Write};
use std::net::{IpAddr, Shutdown, SocketAddr, TcpListener, TcpStream, UdpSocket};
use std::os::magnumopus::runtime as rt;
use std::sync::Arc;
use std::time::{Duration, Instant};

fn main() {
    let mut args = std::env::args();
    let me = args.next().unwrap_or_default();
    let name = me.rsplit('/').next().unwrap_or(&me).to_string();
    let rest: Vec<String> = args.collect();
    let code = match name.as_str() {
        "ping" => ping(&rest),
        "nc" => nc(&rest),
        "resolve" => resolve(&rest),
        _ => {
            eprintln!("nettools: spusťte jako ping, nc nebo resolve");
            2
        }
    };
    std::process::exit(code);
}

/// Argumenty: pozicni a prepinace (`--jmeno` nebo `-x`; `with_value` =
/// prepinace s hodnotou v dalsim argumentu nebo za `=`).
struct Opts {
    pos: Vec<String>,
    flags: Vec<(String, Option<String>)>,
}

impl Opts {
    fn parse(args: &[String], with_value: &[&str]) -> Result<Self, String> {
        let mut o = Opts { pos: Vec::new(), flags: Vec::new() };
        let mut it = args.iter();
        while let Some(a) = it.next() {
            let Some(flag) = a.strip_prefix("--").or_else(|| a.strip_prefix('-').filter(|f| f.len() == 1 && !f.starts_with(|c: char| c.is_ascii_digit()))) else {
                o.pos.push(a.clone());
                continue;
            };
            let (name, inline) = match flag.split_once('=') {
                Some((n, v)) => (n, Some(v.to_string())),
                None => (flag, None),
            };
            let value = if with_value.contains(&name) {
                match inline {
                    Some(v) => Some(v),
                    None => Some(it.next().ok_or_else(|| format!("přepínač --{name} čeká hodnotu"))?.clone()),
                }
            } else {
                inline
            };
            o.flags.push((name.to_string(), value));
        }
        Ok(o)
    }

    fn has(&self, names: &[&str]) -> bool {
        self.flags.iter().any(|(n, _)| names.contains(&n.as_str()))
    }

    fn num(&self, names: &[&str], default: u64) -> Result<u64, String> {
        match self.flags.iter().rev().find(|(n, _)| names.contains(&n.as_str())) {
            None => Ok(default),
            Some((n, v)) => v.as_deref().unwrap_or("").parse().map_err(|_| format!("--{n}: čekám číslo")),
        }
    }

    fn unknown(&self, known: &[&str]) -> Option<String> {
        self.flags.iter().find(|(n, _)| !known.contains(&n.as_str())).map(|(n, _)| n.clone())
    }
}

/// Adresa cile: IP primo, jinak preklad jmena (prednost IPv4).
fn lookup(host: &str) -> Result<IpAddr, String> {
    if let Ok(a) = host.parse::<IpAddr>() {
        return Ok(a);
    }
    let addrs = rt::net::resolve(host).map_err(|e| format!("{host}: jméno nejde přeložit ({e})"))?;
    addrs.iter().find(|a| a.is_ipv4()).or(addrs.first()).copied().ok_or_else(|| format!("{host}: jméno nemá adresu"))
}

fn usage(cmd: &str, text: &str) -> i32 {
    eprintln!("použití: {cmd} {text}");
    2
}

// ---------------------------------------------------------------- ping

fn ping(args: &[String]) -> i32 {
    const USE: &str = "<cíl> [--count N] [--size B] [--timeout ms]";
    let o = match Opts::parse(args, &["count", "c", "size", "s", "timeout", "W"]) {
        Ok(o) => o,
        Err(e) => return usage("ping", &format!("{USE} ({e})")),
    };
    if let Some(f) = o.unknown(&["count", "c", "size", "s", "timeout", "W"]) {
        return usage("ping", &format!("{USE} (neznámý přepínač --{f})"));
    }
    let [host] = o.pos.as_slice() else { return usage("ping", USE) };
    let (count, size, timeout) = match (o.num(&["count", "c"], 4), o.num(&["size", "s"], 56), o.num(&["timeout", "W"], 2000)) {
        (Ok(c), Ok(s), Ok(t)) if s <= 1400 => (c, s as u16, Duration::from_millis(t.max(1))),
        (Ok(_), Ok(_), Ok(_)) => return usage("ping", "--size nejvýš 1400 B"),
        (Err(e), _, _) | (_, Err(e), _) | (_, _, Err(e)) => return usage("ping", &format!("{USE} ({e})")),
    };
    let net = match rt::net::session_net() {
        Ok(n) => n,
        Err(e) => {
            eprintln!("ping: program nemá síť ({e})");
            return 2;
        }
    };
    let addr = match lookup(host) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("ping: {e}");
            return 2;
        }
    };
    println!("PING {host} ({addr}): {size} B dat");
    let (mut sent, mut got) = (0u64, 0u64);
    let mut times: Vec<f64> = Vec::new();
    let mut seq: u16 = 0;
    loop {
        seq = seq.wrapping_add(1);
        sent += 1;
        let start = Instant::now();
        match net.ping(addr, seq, size, timeout) {
            Ok(rtt) => {
                got += 1;
                let ms = rtt.as_secs_f64() * 1000.0;
                times.push(ms);
                println!("odpověď od {addr}: seq={seq} čas={ms:.1} ms");
            }
            Err(e) if e == rt::Error::TimedOut => println!("seq={seq}: bez odpovědi do {} ms", timeout.as_millis()),
            Err(e) => println!("seq={seq}: chyba {e}"),
        }
        let _ = std::io::stdout().flush();
        if count != 0 && sent >= count {
            break;
        }
        if let Some(rest) = Duration::from_secs(1).checked_sub(start.elapsed()) {
            std::thread::sleep(rest);
        }
    }
    let loss = (sent - got) * 100 / sent.max(1);
    println!("--- {host}: odesláno {sent}, přijato {got}, ztráta {loss} %");
    if !times.is_empty() {
        let min = times.iter().cloned().fold(f64::MAX, f64::min);
        let max = times.iter().cloned().fold(0.0, f64::max);
        let avg = times.iter().sum::<f64>() / times.len() as f64;
        println!("čas min/průměr/max = {min:.1}/{avg:.1}/{max:.1} ms");
    }
    if got > 0 { 0 } else { 1 }
}

// ------------------------------------------------------------- resolve

fn resolve(args: &[String]) -> i32 {
    let o = match Opts::parse(args, &[]) {
        Ok(o) => o,
        Err(e) => return usage("resolve", &format!("<jméno>... ({e})")),
    };
    if o.pos.is_empty() || o.unknown(&[]).is_some() {
        return usage("resolve", "<jméno>...");
    }
    let many = o.pos.len() > 1;
    let mut code = 0;
    for name in &o.pos {
        match rt::net::resolve(name) {
            Ok(addrs) if !addrs.is_empty() => {
                for a in addrs {
                    if many { println!("{name}\t{a}") } else { println!("{a}") }
                }
            }
            Ok(_) => {
                eprintln!("resolve: {name}: jméno nemá adresu");
                code = 1;
            }
            Err(e) => {
                eprintln!("resolve: {name}: {e}");
                code = 1;
            }
        }
    }
    code
}

// ------------------------------------------------------------------ nc

fn nc(args: &[String]) -> i32 {
    const USE: &str = "<cíl> <port> [--udp] [--timeout s] | --listen <port> [--udp]";
    let known = ["udp", "u", "listen", "l", "timeout", "w"];
    let o = match Opts::parse(args, &["timeout", "w"]) {
        Ok(o) => o,
        Err(e) => return usage("nc", &format!("{USE} ({e})")),
    };
    if let Some(f) = o.unknown(&known) {
        return usage("nc", &format!("{USE} (neznámý přepínač --{f})"));
    }
    let udp = o.has(&["udp", "u"]);
    let timeout = match o.num(&["timeout", "w"], 10) {
        Ok(t) => Duration::from_secs(t.max(1)),
        Err(e) => return usage("nc", &e),
    };
    let port = |s: &str| s.parse::<u16>().ok().filter(|p| *p != 0);
    if !rt::net::available() {
        eprintln!("nc: program nemá síť");
        return 2;
    }
    let r = if o.has(&["listen", "l"]) {
        let [p] = o.pos.as_slice() else { return usage("nc", USE) };
        let Some(p) = port(p) else { return usage("nc", "port 1 až 65535") };
        if udp { udp_listen(p) } else { tcp_listen(p) }
    } else {
        let [host, p] = o.pos.as_slice() else { return usage("nc", USE) };
        let Some(p) = port(p) else { return usage("nc", "port 1 až 65535") };
        let addr = match lookup(host) {
            Ok(a) => SocketAddr::new(a, p),
            Err(e) => {
                eprintln!("nc: {e}");
                return 2;
            }
        };
        if udp { udp_connect(addr) } else { tcp_connect(addr, timeout) }
    };
    match r {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("nc: {e}");
            1
        }
    }
}

fn tcp_connect(addr: SocketAddr, timeout: Duration) -> Result<(), String> {
    let s = TcpStream::connect_timeout(&addr, timeout).map_err(|e| format!("{addr}: spojení selhalo ({e})"))?;
    pump_tcp(s);
    Ok(())
}

fn tcp_listen(port: u16) -> Result<(), String> {
    let l = TcpListener::bind(("0.0.0.0", port)).map_err(|e| format!("port {port}: nejde naslouchat ({e})"))?;
    eprintln!("nc: čekám na spojení na portu {port}");
    let (s, peer) = l.accept().map_err(|e| format!("přijetí spojení: {e}"))?;
    eprintln!("nc: spojení z {peer}");
    pump_tcp(s);
    Ok(())
}

/// Prijem na stdout ve vlakne, stdin do spojeni. Konec vstupu = konec
/// zapisu (druha strana dostane EOF) a cekani na zbytek odpovedi; konec
/// spojeni druhou stranou = konec programu.
fn pump_tcp(s: TcpStream) {
    let s = Arc::new(s);
    let r = s.clone();
    let reader = std::thread::spawn(move || {
        let mut buf = [0u8; 4096];
        let mut out = std::io::stdout();
        loop {
            match (&*r).read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    let _ = out.write_all(&buf[..n]);
                    let _ = out.flush();
                }
            }
        }
    });
    let stdin = std::io::stdin();
    let mut line = Vec::new();
    let mut input = stdin.lock();
    loop {
        line.clear();
        match input.read_until(b'\n', &mut line) {
            Ok(0) | Err(_) => break,
            Ok(_) => {
                if (&*s).write_all(&line).is_err() {
                    break;
                }
            }
        }
        if reader.is_finished() {
            break;
        }
    }
    let _ = s.shutdown(Shutdown::Write);
    let _ = reader.join();
}

fn udp_connect(addr: SocketAddr) -> Result<(), String> {
    let any: SocketAddr = if addr.is_ipv4() { "0.0.0.0:0".parse().unwrap() } else { "[::]:0".parse().unwrap() };
    let s = UdpSocket::bind(any).map_err(|e| format!("socket UDP: {e}"))?;
    s.connect(addr).map_err(|e| format!("{addr}: {e}"))?;
    pump_udp(s);
    Ok(())
}

fn udp_listen(port: u16) -> Result<(), String> {
    let s = UdpSocket::bind(("0.0.0.0", port)).map_err(|e| format!("port {port}: nejde naslouchat ({e})"))?;
    eprintln!("nc: čekám na data na portu UDP {port}");
    let mut buf = [0u8; 2048];
    let (n, peer) = s.recv_from(&mut buf).map_err(|e| format!("příjem: {e}"))?;
    let _ = std::io::stdout().write_all(&buf[..n]);
    let _ = std::io::stdout().flush();
    s.connect(peer).map_err(|e| format!("{peer}: {e}"))?;
    pump_udp(s);
    Ok(())
}

/// Kazdy radek vstupu = jeden datagram; prijate datagramy na stdout. Po
/// konci vstupu jeste 2 s cekani na odpovedi (UDP nema konec spojeni).
fn pump_udp(s: UdpSocket) {
    let s = Arc::new(s);
    let r = s.clone();
    let _ = r.set_read_timeout(Some(Duration::from_millis(200)));
    let done = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let stop = done.clone();
    let reader = std::thread::spawn(move || {
        let mut buf = [0u8; 2048];
        let mut out = std::io::stdout();
        let mut quiet_since: Option<Instant> = None;
        loop {
            match r.recv(&mut buf) {
                Ok(n) => {
                    let _ = out.write_all(&buf[..n]);
                    let _ = out.flush();
                    quiet_since = None;
                }
                Err(_) => {
                    if stop.load(std::sync::atomic::Ordering::Acquire) {
                        let since = *quiet_since.get_or_insert_with(Instant::now);
                        if since.elapsed() >= Duration::from_secs(2) {
                            break;
                        }
                    }
                }
            }
        }
    });
    let stdin = std::io::stdin();
    let mut line = Vec::new();
    let mut input = stdin.lock();
    loop {
        line.clear();
        match input.read_until(b'\n', &mut line) {
            Ok(0) | Err(_) => break,
            Ok(_) => {
                if s.send(&line).is_err() {
                    break;
                }
            }
        }
    }
    done.store(true, std::sync::atomic::Ordering::Release);
    let _ = reader.join();
}
