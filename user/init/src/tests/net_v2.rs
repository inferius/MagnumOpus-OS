//! netproto v2: soubezne dotazy na jednom kanalu (cislo dotazu), termin
//! a zruseni. Proti echo serveru hostitele (`xtask/src/nethelper.rs`).
//! Zruseni connectu cekajiciho na dotaz firewallu je v `firewall.rs`
//! (potrebuje agenta uzivatele).

use alloc::format;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::time::Duration;

use libos::sync::Mutex;
use libos::{Error, Startup, time};
use netproto::{CancelToken, Deadline, TcpStream};

use super::net::{configured, host, net, ports};
use super::{Test, TestResult, fail};
use crate::{check, check_eq};

pub const TESTS: &[Test] = &[
    ("cteni a zapis jednoho spojeni ze dvou vlaken", read_write_threads),
    ("dva ctenari jednoho spojeni (fronta smeru)", two_readers),
    ("termin: jen zkusit (SHOULD_WAIT) a kratky limit", deadlines),
    ("mistni adresa naslouchani a socketu UDP", bind_addresses),
    ("zruseni cteni z jineho vlakna", cancel_recv),
];

/// Limit cekani na vlakno testu.
const JOIN: Duration = Duration::from_secs(20);

fn connect(s: &Startup, test: &str) -> Result<Option<TcpStream>, String> {
    let Some(p) = ports(s, test)? else {
        return Ok(None);
    };
    let net = net()?;
    configured(net)?;
    let stream = net
        .tcp_connect(host(p.echo), Some(Duration::from_secs(10)))
        .map_err(|e| fail("spojeni", e))?;
    stream.set_read_timeout(Some(Duration::from_secs(10)));
    stream.set_write_timeout(Some(Duration::from_secs(10)));
    Ok(Some(stream))
}

/// Precte presne `n` bajtu.
fn read_n(s: &TcpStream, n: usize) -> libos::Result<Vec<u8>> {
    let mut buf = alloc::vec![0u8; n];
    let mut got = 0;
    while got < n {
        match s.read(&mut buf[got..])? {
            0 => return Err(Error::ConnectionReset),
            k => got += k,
        }
    }
    Ok(buf)
}

/// Spusti cteni `n` bajtu ve vlakne; vysledek az po `join`.
fn reader(s: &TcpStream, n: usize) -> Result<(libos::thread::JoinHandle, Arc<Mutex<Option<libos::Result<Vec<u8>>>>>), String> {
    let r = s.try_clone().map_err(|e| fail("klon", e))?;
    let out = Arc::new(Mutex::new(None));
    let o = out.clone();
    let t = libos::thread::spawn("net-v2-cteni", move || {
        *o.lock() = Some(read_n(&r, n));
    })
    .map_err(|e| fail("vlakno", e))?;
    Ok((t, out))
}

fn join(t: libos::thread::JoinHandle) -> TestResult {
    t.join(Some(time::deadline_after(JOIN))).map_err(|e| fail("vlakno nedobehlo", e))
}

/// Pred v2: zapis cekal za blokovanym RECV (jeden dotaz na kanalu)
/// a vlakno cteni by dostalo odpoved na zapis.
fn read_write_threads(s: &Startup) -> TestResult {
    let Some(stream) = connect(s, "v2 cteni a zapis")? else {
        return Ok(());
    };
    let (t, out) = reader(&stream, 4)?;
    // Cteni ma byt ve sluzbe rozpracovane driv, nez prijde zapis.
    time::sleep(Duration::from_millis(200));
    let start = time::now();
    let n = stream.write(b"ping").map_err(|e| fail("zapis", e))?;
    check_eq!(n, 4, "zapsano");
    join(t)?;
    let got = out.lock().take().ok_or("vlakno cteni nic nevratilo")?;
    let got = got.map_err(|e| fail("cteni ve vlakne", e))?;
    check_eq!(got.as_slice(), b"ping".as_slice(), "echo do vlakna cteni");
    let ms = (time::now() - start) / 1_000_000;
    check!(ms < 5_000, format!("zapis a echo trvaly {ms} ms (zapis cekal za ctenim?)"));
    Ok(())
}

/// Dva soubezne RECV na jednom spojeni: druhy ceka ve fronte (FIFO),
/// nedostane chybu; oba dostanou sva data.
fn two_readers(s: &Startup) -> TestResult {
    let Some(stream) = connect(s, "v2 dva ctenari")? else {
        return Ok(());
    };
    let (t1, o1) = reader(&stream, 4)?;
    let (t2, o2) = reader(&stream, 4)?;
    time::sleep(Duration::from_millis(200));
    stream.write(b"AAAABBBB").map_err(|e| fail("zapis", e))?;
    join(t1)?;
    join(t2)?;
    let a = o1.lock().take().ok_or("ctenar 1")?.map_err(|e| fail("ctenar 1", e))?;
    let b = o2.lock().take().ok_or("ctenar 2")?.map_err(|e| fail("ctenar 2", e))?;
    let mut all = [a, b].concat();
    all.sort_unstable();
    check_eq!(all.as_slice(), b"AAAABBBB".as_slice(), "ctenari dostali vsechna data");
    Ok(())
}

fn deadlines(s: &Startup) -> TestResult {
    let Some(stream) = connect(s, "v2 termin")? else {
        return Ok(());
    };
    let mut buf = [0u8; 16];
    // Jen zkusit (neblokujici socket): zadna data = SHOULD_WAIT hned.
    let start = time::now();
    let r = stream.recv_ctx(&mut buf, Deadline::TRY, None);
    check_eq!(r.err(), Some(Error::ShouldWait), "jen zkusit bez dat");
    let ms = (time::now() - start) / 1_000_000;
    check!(ms < 2_000, format!("jen zkusit trvalo {ms} ms"));
    // Kratky limit: vyprsi az po nem, ne o moc pozdeji.
    let start = time::now();
    let r = stream.recv_ctx(&mut buf, Deadline::after(Some(Duration::from_millis(300))), None);
    check_eq!(r.err(), Some(Error::TimedOut), "limit 300 ms bez dat");
    let ms = (time::now() - start) / 1_000_000;
    check!((250..5_000).contains(&ms), format!("limit 300 ms vyprsel za {ms} ms"));
    // Spojeni po vyprseni funguje dal.
    stream.write(b"dal").map_err(|e| fail("zapis po vyprseni", e))?;
    let back = read_n(&stream, 3).map_err(|e| fail("cteni", e))?;
    check_eq!(back.as_slice(), b"dal".as_slice(), "echo po vyprseni");
    Ok(())
}

fn cancel_recv(s: &Startup) -> TestResult {
    let Some(stream) = connect(s, "v2 zruseni")? else {
        return Ok(());
    };
    let token = CancelToken::new();
    let r = stream.try_clone().map_err(|e| fail("klon", e))?;
    let tk = token.clone();
    let out = Arc::new(Mutex::new(None));
    let o = out.clone();
    let t = libos::thread::spawn("net-v2-zruseni", move || {
        let mut buf = [0u8; 16];
        *o.lock() = Some(r.recv_ctx(&mut buf, Deadline::NONE, Some(&tk)));
    })
    .map_err(|e| fail("vlakno", e))?;
    time::sleep(Duration::from_millis(200));
    let start = time::now();
    token.cancel();
    join(t)?;
    let ms = (time::now() - start) / 1_000_000;
    let got = out.lock().take().ok_or("vlakno nic nevratilo")?;
    check_eq!(got.err(), Some(Error::Canceled), "zrusene cteni");
    check!(ms < 2_000, format!("zruseni trvalo {ms} ms"));
    // Zruseny token zrusi i dalsi dotaz hned.
    let mut buf = [0u8; 4];
    check_eq!(stream.recv_ctx(&mut buf, Deadline::NONE, Some(&token)).err(), Some(Error::Canceled), "dalsi dotaz se zrusenym tokenem");
    // Spojeni funguje dal.
    stream.write(b"ok").map_err(|e| fail("zapis po zruseni", e))?;
    let back = read_n(&stream, 2).map_err(|e| fail("cteni", e))?;
    check_eq!(back.as_slice(), b"ok".as_slice(), "echo po zruseni");
    Ok(())
}

fn bind_addresses(s: &Startup) -> TestResult {
    use core::net::SocketAddr;
    if ports(s, "v2 mistni adresa")?.is_none() {
        return Ok(());
    }
    let net = net()?;
    let info = configured(net).and_then(|_| net.info().map_err(|e| fail("info", e)))?;
    let (ip, _) = info.addr.ok_or("rozhrani nema adresu")?;
    let b = netproto::Buffers::DEFAULT;
    // Vsechna rozhrani a adresa rozhrani projdou.
    let l = net.tcp_listen_addr(SocketAddr::new(ip, 0), 1, b).map_err(|e| fail("naslouchani na adrese rozhrani", e))?;
    drop(l);
    let u = net.udp_bind_addr(SocketAddr::from(([0, 0, 0, 0], 0)), b).map_err(|e| fail("UDP na 0.0.0.0", e))?;
    drop(u);
    // Cizi adresa (loopback 127.x uz netd ma, testy v net_lo.rs).
    for a in [SocketAddr::from(([10, 0, 2, 99], 0))] {
        check_eq!(net.tcp_listen_addr(a, 1, b).err(), Some(Error::AddressNotAvailable), format!("naslouchani na {a}"));
        check_eq!(net.udp_bind_addr(a, b).err(), Some(Error::AddressNotAvailable), format!("UDP na {a}"));
    }
    // Neblokujici prijeti bez cekajiciho spojeni.
    let l = net.tcp_listen(0, 1).map_err(|e| fail("naslouchani", e))?;
    check_eq!(l.accept_ctx(Deadline::TRY, None).err(), Some(Error::ShouldWait), "accept jen zkusit");
    Ok(())
}
