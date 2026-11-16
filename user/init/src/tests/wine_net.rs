//! Sit z programu Windows (W4b, `docs/cs/windows-vrstva.md`, kapitola W4b;
//! sada `wine`, beh s `--wine`): Winsock -> wineserver (sockety vytvari
//! server a fd posila klientovi pres `SCM_RIGHTS`) -> netd.
//!
//! - `wintest tcp` proti echo sluzbe v QEMU (loopback) a proti echo
//!   serveru hostitele (cesta ven): spojeni, data obema smery,
//! - PuTTY raw a telnet proti echo serveru testu na loopbacku: pozdrav
//!   serveru je v terminalu videt, text psany z QEMU dojde na server a jeho
//!   ozvena je taky videt.

use alloc::format;
use alloc::string::String;
use core::net::{IpAddr, Ipv4Addr, SocketAddr};
use core::time::Duration;

use libos::{Error, Startup, println, time};
use netproto::{Buffers, Deadline};

use super::TestResult;
use super::fail;
use super::gui::{pixel, settle};
use super::wine::TESTS_APP;
use super::wine_gui::{W3, Run, finish, launch, wait_for, wait_window, with};
use crate::check;

/// Znacky pro fyzicky vstup z QEMU (musi sedet s `xtask/src/qmp.rs`, `wine`).
const QEMU_PUTTY_RAW: &str = "init: test wine: klik do PuTTY raw na ";
const QEMU_PUTTY_TELNET: &str = "init: test wine: klik do PuTTY telnet na ";

/// Winsock z `wintest tcp`: spojeni na loopback, "ping" od Windows,
/// odpoved od nas.
pub fn winsock_loopback(s: &Startup) -> TestResult {
    with(s, |w| {
        let net = super::net::net()?;
        let l = net
            .tcp_listen_addr(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0), 2, Buffers::DEFAULT)
            .map_err(|e| fail("naslouchani", e))?;
        let port = l.local_addr().map_err(|e| fail("adresa", e))?.port();
        let exe = format!("{TESTS_APP}/wintest.exe");
        let ports = format!("{port}");
        let mut run = launch(s, w, &[&exe, "tcp", "127.0.0.1", &ports])?;
        // Prijeti (bez blokovani: supervisor testu musi bezet kvuli
        // listkum procesu Wine).
        let conn = wait_for(s, w, &mut run, "spojeni z Winsocku", |_, _| match l.accept_ctx(Deadline::TRY, None) {
            Ok((c, _)) => Ok(Some(c)),
            Err(Error::ShouldWait) => Ok(None),
            Err(e) => Err(fail("accept", e)),
        })?;
        let mut got = alloc::vec::Vec::new();
        let mut buf = [0u8; 64];
        let end = time::deadline_after(Duration::from_secs(30));
        while !got.ends_with(b"\n") && time::now() < end {
            match conn.recv_ctx(&mut buf, Deadline::after(Some(Duration::from_millis(100))), None) {
                Ok(0) => break,
                Ok(n) => got.extend_from_slice(&buf[..n]),
                Err(Error::TimedOut) => {
                    w.sup.step(s, Some(time::deadline_after(Duration::from_millis(20))));
                }
                Err(e) => return Err(fail("cteni z Winsocku", e)),
            }
        }
        check!(got == b"ping-z-windows\n", format!("od Windows: {:?}", String::from_utf8_lossy(&got)));
        conn.send_ctx(b"pong-z-magnumopus\n", Deadline::NONE).map_err(|e| fail("odpoved", e))?;
        wait_for(s, w, &mut run, "odpoved ve Winsocku", |_, r| Ok(String::from_utf8_lossy(&r.stdout).contains("tcp: ok pong-z-magnumopus").then_some(())))?;
        println!("init: test wine: Winsock pres loopback ok (port {port})");
        let _ = run.process.wait_exit(Some(time::deadline_after(Duration::from_secs(10))));
        Ok(())
    })
}

/// Naslouchani programu Windows: na loopbacku ano (spojeni sam se sebou),
/// na siti ne (manifest Wine `listen_from = "loopback"`, `WSAEACCES`).
pub fn winsock_listen(s: &Startup) -> TestResult {
    with(s, |w| {
        let exe = format!("{TESTS_APP}/wintest.exe");
        for (ip, want) in [("127.0.0.1", "listen: ok "), ("0.0.0.0", "10013")] {
            let mut run = launch(s, w, &[&exe, "listen", ip])?;
            let out = wait_for(s, w, &mut run, "vysledek naslouchani", |_, r| {
                let t = String::from_utf8_lossy(&r.stdout);
                Ok(t.lines().find(|l| l.starts_with("listen: ")).map(String::from))
            })?;
            check!(out.contains(want), format!("{ip}: {out} (cekano {want})"));
            println!("init: test wine: naslouchani na {ip}: {out}");
        }
        Ok(())
    })
}

/// Winsock k echo serveru hostitele (cesta ven pres slirp).
pub fn winsock_host(s: &Startup) -> TestResult {
    let Some(p) = super::net::ports(s, "wine winsock hostitel")? else {
        return Ok(());
    };
    with(s, |w| {
        let exe = format!("{TESTS_APP}/wintest.exe");
        let ip = format!("{}", super::net::HOST);
        let port = format!("{}", p.echo);
        let mut run = launch(s, w, &[&exe, "tcp", &ip, &port])?;
        wait_for(s, w, &mut run, "ozvena hostitele ve Winsocku", |_, r| {
            Ok(String::from_utf8_lossy(&r.stdout).contains("tcp: ok ping-z-windows").then_some(()))
        })?;
        println!("init: test wine: Winsock k hostiteli {ip}:{port} ok");
        Ok(())
    })
}

/// Body terminalu PuTTY, ktere nejsou cerne (text) - obsah okna po 3 px.
fn ink(w: &W3, r: guiproto::Rect) -> Result<u32, String> {
    let mut n = 0;
    for y in (r.y + 2..r.y + r.h as i32 - 2).step_by(3) {
        for x in (r.x + 2..r.x + r.w as i32 - 2).step_by(3) {
            if pixel(&w.inspect, x, y)? & 0x00FF_FFFF > 0x0030_3030 {
                n += 1;
            }
        }
    }
    Ok(n)
}

/// Pocka, az bude v terminalu vic textu nez `than`.
fn wait_ink(s: &Startup, w: &mut W3, run: &mut Run, r: guiproto::Rect, than: u32, what: &str) -> Result<u32, String> {
    wait_for(s, w, run, what, |w, _| {
        let n = ink(w, r)?;
        Ok((n > than).then_some(n))
    })
}

/// Cte ze spojeni, dokud data (bez prikazu telnetu IAC) neobsahuji
/// `want`; supervisor testu bezi dal.
fn read_until(s: &Startup, w: &mut W3, run: &mut Run, conn: &netproto::TcpStream, want: &[u8]) -> Result<alloc::vec::Vec<u8>, String> {
    let mut got = alloc::vec::Vec::new();
    let mut buf = [0u8; 256];
    let end = time::deadline_after(Duration::from_secs(60));
    loop {
        // Prikazy telnetu (IAC = 0xFF a dva bajty) vynechat.
        let mut text = alloc::vec::Vec::new();
        let mut i = 0;
        while i < got.len() {
            if got[i] == 0xFF {
                i += 3;
            } else {
                text.push(got[i]);
                i += 1;
            }
        }
        if text.windows(want.len()).any(|x| x == want) {
            return Ok(text);
        }
        if time::now() > end {
            return Err(format!("od PuTTY prislo jen {:?}", String::from_utf8_lossy(&text)));
        }
        run.drain();
        match conn.recv_ctx(&mut buf, Deadline::after(Some(Duration::from_millis(50))), None) {
            Ok(0) => return Err(String::from("PuTTY spojeni zavrel")),
            Ok(n) => got.extend_from_slice(&buf[..n]),
            Err(Error::TimedOut) => {
                w.sup.step(s, Some(time::deadline_after(Duration::from_millis(20))));
            }
            Err(e) => return Err(fail("cteni od PuTTY", e)),
        }
    }
}

/// PuTTY (`-raw` nebo `-telnet`) proti echo serveru testu na loopbacku.
fn putty_session(s: &Startup, w: &mut W3, mode: &str, marker: &str) -> TestResult {
    let net = super::net::net()?;
    let l = net
        .tcp_listen_addr(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0), 2, Buffers::DEFAULT)
        .map_err(|e| fail("naslouchani", e))?;
    let port = format!("{}", l.local_addr().map_err(|e| fail("adresa", e))?.port());
    let exe = format!("{TESTS_APP}/putty.exe");
    let mut run = launch(s, w, &[&exe, mode, "-P", &port, "127.0.0.1"])?;
    let conn = wait_for(s, w, &mut run, "spojeni z PuTTY", |_, _| match l.accept_ctx(Deadline::TRY, None) {
        Ok((c, _)) => Ok(Some(c)),
        Err(Error::ShouldWait) => Ok(None),
        Err(e) => Err(fail("accept", e)),
    })?;
    let win = wait_window(s, w, &mut run, "PuTTY")?;
    settle(&w.inspect)?;
    let r = win.rect;
    let before = ink(w, r)?;
    // Pozdrav serveru: musi byt v terminalu videt.
    conn.send_ctx(b"ahoj-z-magnumopus\r\n", Deadline::NONE).map_err(|e| fail("pozdrav", e))?;
    let greeted = wait_ink(s, w, &mut run, r, before, "pozdrav v terminalu PuTTY")?;
    // Text z klavesnice (QEMU): dojde na server, ozvena je videt.
    println!("{marker}{} {}", r.x + r.w as i32 / 2, r.y + r.h as i32 / 2);
    read_until(s, w, &mut run, &conn, b"ahoj")?;
    conn.send_ctx(b"ozvena: ahoj\r\n", Deadline::NONE).map_err(|e| fail("ozvena", e))?;
    let echoed = wait_ink(s, w, &mut run, r, greeted, "ozvena v terminalu PuTTY")?;
    println!("init: test wine: PuTTY {mode} na 127.0.0.1:{port}: text {before} -> {greeted} -> {echoed} bodu");
    finish(s, w, run, "PuTTY")
}

pub fn putty_raw(s: &Startup) -> TestResult {
    with(s, |w| putty_session(s, w, "-raw", QEMU_PUTTY_RAW))
}

pub fn putty_telnet(s: &Startup) -> TestResult {
    with(s, |w| putty_session(s, w, "-telnet", QEMU_PUTTY_TELNET))
}
