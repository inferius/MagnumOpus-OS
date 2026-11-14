//! Pripravenost socketu (`READINESS`, uzivatelske bity eventu): zaklad
//! backendu mio. Smycka jako v mio: registrace eventu na portu, udalost,
//! operace s terminem `TRY`, nova registrace.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::time::Duration;

use libos::{Error, Port, Signals, Startup, time};
use netproto::{Deadline, TcpStream, ready};

use super::net::{configured, host, net, ports};
use super::{Test, TestResult, fail};
use crate::{check, check_eq};

pub const TESTS: &[Test] = &[
    ("spojeni: zapis hned, cteni po datech, konec proudu", stream_readiness),
    ("4 spojeni na jednom portu (smycka mio)", port_loop),
    ("UDP a naslouchani", udp_and_listener),
];

const WAIT: Duration = Duration::from_secs(10);

fn deadline() -> Option<u64> {
    Some(time::deadline_after(WAIT))
}

/// Prectene vse, co je hned k dispozici (`TRY`), az do `n` bajtu.
fn read_ready(s: &TcpStream, n: usize) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    let mut buf = [0u8; 256];
    let until = time::deadline_after(WAIT);
    while out.len() < n {
        match s.recv_ctx(&mut buf, Deadline::TRY, None) {
            Ok(0) => break,
            Ok(k) => out.extend_from_slice(&buf[..k]),
            // Data jeste neprisla cela: kratce pockat (urovnovy stav).
            Err(Error::ShouldWait) if time::now() < until => time::sleep(Duration::from_millis(5)),
            Err(e) => return Err(fail("cteni TRY", e)),
        }
    }
    Ok(out)
}

/// Pocka, az bit `bit` zmizi (stav se prepocita v dalsim kole netd).
fn wait_clear(r: &netproto::Readiness, bit: Signals, what: &str) -> TestResult {
    let until = time::deadline_after(Duration::from_secs(2));
    while r.current().intersects(bit) {
        if time::now() > until {
            return Err(format!("{what}: bit {:#x} nezmizel do 2 s", bit.0));
        }
        time::sleep(Duration::from_millis(5));
    }
    Ok(())
}

fn connect(s: &Startup, test: &str) -> Result<Option<(TcpStream, u16)>, String> {
    let Some(p) = ports(s, test)? else {
        return Ok(None);
    };
    let net = net()?;
    configured(net)?;
    let st = net.tcp_connect(host(p.echo), Some(Duration::from_secs(10))).map_err(|e| fail("spojeni", e))?;
    Ok(Some((st, p.echo)))
}

fn stream_readiness(s: &Startup) -> TestResult {
    let Some((st, _)) = connect(s, "pripravenost spojeni")? else {
        return Ok(());
    };
    let r = st.readiness().map_err(|e| fail("READINESS", e))?;
    let now = r.current();
    check!(now.contains(ready::WRITABLE), format!("novy socket neni zapisovatelny ({:#x})", now.0));
    check!(!now.intersects(ready::READABLE | ready::HUP | ready::ERROR), format!("necekane bity {:#x}", now.0));
    // Klient bity zmenit nesmi (handle jen pro cekani).
    let ev = libos::Event::from_handle(r.handle().duplicate(None).map_err(|e| fail("duplikat", e))?);
    check_eq!(ev.update(ready::ALL, Signals::NONE).err(), Some(Error::AccessDenied), "zmena bitu klientem");
    // Data -> READABLE (pres port, jako mio).
    let port = Port::create().map_err(|e| fail("port", e))?;
    port.wait_async(r.handle(), 1, ready::READABLE).map_err(|e| fail("registrace", e))?;
    st.write(b"abc").map_err(|e| fail("zapis", e))?;
    let p = port.wait(deadline()).map_err(|e| fail("udalost READABLE", e))?;
    check_eq!(p.key, 1, "klic udalosti");
    check!(Signals(p.signals).contains(ready::READABLE), format!("signaly {:#x}", p.signals));
    let got = read_ready(&st, 3)?;
    check_eq!(got.as_slice(), b"abc".as_slice(), "data po udalosti");
    wait_clear(&r, ready::READABLE, "po precteni")?;
    // Konec zapisu: echo server spojeni zavre -> HUP a READABLE (cteni
    // vrati 0, neblokuje).
    st.shutdown_write().map_err(|e| fail("shutdown", e))?;
    let bits = r.wait(ready::HUP, Deadline::after(Some(WAIT))).map_err(|e| fail("cekani na HUP", e))?;
    check!(bits.contains(ready::READABLE), format!("HUP bez READABLE ({:#x})", bits.0));
    check!(!bits.contains(ready::ERROR), "konec proudu neni chyba");
    let mut buf = [0u8; 4];
    check_eq!(st.recv_ctx(&mut buf, Deadline::TRY, None).map_err(|e| fail("cteni konce", e))?, 0, "konec proudu");
    Ok(())
}

/// Ctyri spojeni, jeden port: udalosti prijdou s klicem spravneho
/// spojeni (backend mio: `Registry::register` = `wait_async`).
fn port_loop(s: &Startup) -> TestResult {
    let Some(p) = ports(s, "pripravenost smycka")? else {
        return Ok(());
    };
    let net = net()?;
    configured(net)?;
    let port = Port::create().map_err(|e| fail("port", e))?;
    let mut conns = Vec::new();
    for i in 0..4u64 {
        let st = net.tcp_connect(host(p.echo), Some(Duration::from_secs(10))).map_err(|e| fail("spojeni", e))?;
        let r = st.readiness().map_err(|e| fail("READINESS", e))?;
        port.wait_async(r.handle(), 100 + i, ready::READABLE).map_err(|e| fail("registrace", e))?;
        conns.push((st, r));
    }
    // Zapis v opacnem poradi; kazde spojeni dostane jinou zpravu.
    for (i, (st, _)) in conns.iter().enumerate().rev() {
        st.write(format!("m{i}").as_bytes()).map_err(|e| fail("zapis", e))?;
    }
    let mut seen = [false; 4];
    for _ in 0..4 {
        let pk = port.wait(deadline()).map_err(|e| fail("udalost", e))?;
        let i = pk.key.checked_sub(100).filter(|&i| i < 4).ok_or(format!("cizi klic {}", pk.key))? as usize;
        check!(!seen[i], format!("dve udalosti spojeni {i} (registrace je jednorazova)"));
        seen[i] = true;
        let got = read_ready(&conns[i].0, 2)?;
        let want = format!("m{i}");
        check_eq!(got.as_slice(), want.as_bytes(), format!("data spojeni {i}"));
    }
    // Zadna dalsi udalost bez nove registrace.
    check_eq!(port.wait(Some(0)).err(), Some(Error::TimedOut), "udalost navic");
    Ok(())
}

fn udp_and_listener(s: &Startup) -> TestResult {
    let Some(p) = ports(s, "pripravenost UDP")? else {
        return Ok(());
    };
    let net = net()?;
    configured(net)?;
    // UDP: datagram -> READABLE. UDP muze datagram ztratit: az 5 pokusu,
    // kazdy z noveho socketu (novy tok ve slirp).
    let mut ok = false;
    for _ in 0..5 {
        let u = net.udp_bind(0).map_err(|e| fail("bind", e))?;
        let r = u.readiness().map_err(|e| fail("READINESS UDP", e))?;
        check!(r.current().contains(ready::WRITABLE), "UDP neni zapisovatelny");
        u.send_to(b"ping", host(p.udp_echo)).map_err(|e| fail("send_to", e))?;
        if r.wait(ready::READABLE, Deadline::after(Some(Duration::from_secs(3)))).is_err() {
            continue;
        }
        let mut buf = [0u8; 16];
        let (n, from) = u.recv_from_ctx(&mut buf, Deadline::TRY, None).map_err(|e| fail("recv_from TRY", e))?;
        check_eq!((&buf[..n], from), (b"ping".as_slice(), host(p.udp_echo)), "datagram");
        ok = true;
        break;
    }
    check!(ok, "UDP: zadna odpoved za 5 pokusu");
    // Naslouchani: prichozi spojeni z hostitele -> READABLE.
    super::firewall::ensure_home()?;
    let l = net.tcp_listen(p.forwarded, 2).map_err(|e| fail("naslouchani", e))?;
    let r = l.readiness().map_err(|e| fail("READINESS naslouchani", e))?;
    check!(!r.current().contains(ready::READABLE), "naslouchani bez spojeni je READABLE");
    let ctl = net.tcp_connect(host(p.callback), Some(Duration::from_secs(10))).map_err(|e| fail("spojeni na callback", e))?;
    ctl.write(b"connect\n").map_err(|e| fail("zapis callback", e))?;
    r.wait(ready::READABLE, Deadline::after(Some(WAIT))).map_err(|e| fail("cekani na spojeni", e))?;
    let (conn, _) = l.accept_ctx(Deadline::TRY, None).map_err(|e| fail("accept TRY", e))?;
    drop(conn);
    wait_clear(&r, ready::READABLE, "po accept")?;
    Ok(())
}
