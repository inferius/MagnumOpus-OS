//! TLS a HTTP: program `fetch` (libs/tls + libs/httpc) proti testovacim
//! serverum hostitele (rustls + hyper, `xtask/src/webhelper.rs`)
//! a systemove uloziste korenovych certifikatu.
//!
//! `fetch` dostane relaci site podle manifestu, jen s cili na pomocne
//! servery (TCP na 10.0.2.2 a jejich porty), a relaci FS jen s `/certs`
//! pro cteni. Testovaci CA je v ulozisti jako `test-ca.pem`.

use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use core::net::{IpAddr, Ipv4Addr};

use fsproto::kind;
use libos::initfs::Initfs;
use libos::spawn::StartHandle;
use libos::{Channel, Process, Startup, time};
use netproto::{Rule, Target, proto};

use super::{Test, TestResult, devices, fail};
use crate::{check, check_eq};

pub const TESTS: &[Test] = &[
    ("uloziste certifikatu: sada Mozilla, testovaci CA, jen cteni", store),
    ("fetch: korenove CA z uloziste", roots),
    ("HTTP/1.1 bez TLS", plain_http),
    ("HTTPS: HTTP/2 vyjednane pres ALPN", https_h2),
    ("HTTPS: HTTP/1.1, kdyz server h2 nenabizi", https_h1),
    ("HTTPS: velke telo (HTTP/2 rizeni toku, chunked)", https_big),
    ("HTTPS: POST s telem", https_post),
    ("HTTPS: cizi CA a spatne jmeno odmitnuty", https_rejects),
    ("presmerovani: v ramci https ano, na http ne", redirects),
    ("odvolani: certifikat v CRL (revocd stahne a ulozi)", revoked_crl),
    ("odvolani: OCSP stapling", revoked_ocsp),
    ("odvolani: mezilehla CA v OneCRL", revoked_onecrl),
];

struct Ports {
    http: u16,
    https: u16,
    https_h1: u16,
    untrusted: u16,
    wrong_name: u16,
    revoked: u16,
    ocsp_revoked: u16,
    onecrl: u16,
}

fn ports(s: &Startup) -> Result<Option<Ports>, String> {
    let fs = Initfs::open(s).map_err(|e| fail("initfs", e))?;
    let Some(cfg) = fs.find("net-test") else {
        libos::println!("[preskoceno] web: chybi pomocne servery (jen `cargo xtask test`)");
        return Ok(None);
    };
    let text = core::str::from_utf8(cfg).map_err(|_| String::from("net-test neni UTF-8"))?;
    let get = |key: &str| -> Result<u16, String> {
        text.lines()
            .filter_map(|l| l.split_once('='))
            .find(|(k, _)| k.trim() == key)
            .and_then(|(_, v)| v.trim().parse().ok())
            .ok_or_else(|| format!("net-test: chybi {key}"))
    };
    Ok(Some(Ports {
        http: get("http")?,
        https: get("https")?,
        https_h1: get("https_h1")?,
        untrusted: get("https_untrusted")?,
        wrong_name: get("https_wrongname")?,
        revoked: get("https_revoked")?,
        ocsp_revoked: get("https_ocsp_revoked")?,
        onecrl: get("https_onecrl")?,
    }))
}

const HOST: IpAddr = IpAddr::V4(Ipv4Addr::new(10, 0, 2, 2));

struct Fetch {
    _process: Process,
    ch: Channel,
}

static FETCH: spin::Once<Result<Fetch, String>> = spin::Once::new();

fn start_fetch(s: &Startup, p: &Ports) -> Result<Fetch, String> {
    let root = super::net::net()?;
    super::net::configured(root)?;
    let fs = Initfs::open(s).map_err(|e| fail("initfs", e))?;
    let m = crate::apps::manifest(&fs, "fetch")
        .map_err(|e| fail("manifest", e))?
        .ok_or("fetch nema manifest")?;
    let mut r = m.network.clone().ok_or("manifest fetch bez site")?;
    // Jen pomocne servery: TCP na 10.0.2.2 a jejich porty.
    let host = Target::Net { addr: HOST, prefix: 32 };
    r.policy.connect = Some(
        [p.http, p.https, p.https_h1, p.untrusted, p.wrong_name, p.revoked, p.ocsp_revoked, p.onecrl]
            .iter()
            .map(|&port| Rule::port(proto::TCP, host.clone(), port))
            .collect(),
    );
    let net = root.restrict_with(&r).map_err(|e| fail("relace site", e))?;
    let d = devices().ok_or("zarizeni nebezi")?;
    let certs = crate::certs::session(d).map_err(|e| fail("relace uloziste", e))?;
    let revocation = crate::revocation::client().ok_or("revocd nebezi")?;
    let app = crate::apps::spawn(
        s,
        "fetch",
        vec![
            StartHandle::user(net.into_channel().into_handle()),
            StartHandle::user(certs.into_channel().into_handle()),
            StartHandle::user(revocation.into_handle()),
        ],
    )
    .map_err(|e| fail("spusteni fetch", e))?;
    Ok(Fetch {
        _process: app.process,
        ch: app.control,
    })
}

/// Bezici `fetch` a porty (`None` = mimo `cargo xtask test`).
fn fetch(s: &Startup) -> Result<Option<(&'static Fetch, Ports)>, String> {
    let Some(p) = ports(s)? else {
        return Ok(None);
    };
    let f = FETCH.call_once(|| start_fetch(s, &p)).as_ref().map_err(|e| e.clone())?;
    Ok(Some((f, p)))
}

impl Fetch {
    fn run(&self, cmd: &str) -> Result<String, String> {
        self.ch.write(cmd.as_bytes(), Vec::new()).map_err(|e| fail("prikaz fetch", e))?;
        // TLS v QEMU bez akcelerace (TCG): handshake trva sekundy.
        let m = self
            .ch
            .read(Some(time::deadline_after(core::time::Duration::from_secs(120))))
            .map_err(|e| fail("odpoved fetch", e))?;
        String::from_utf8(m.data).map_err(|_| String::from("odpoved neni UTF-8"))
    }

    /// `ok:<stav> <verze> <delka> <fnv> <adresa>` rozlozene.
    fn get(&self, cmd: &str) -> Result<(u16, String, usize, String, String), String> {
        let r = self.run(cmd)?;
        let rest = r.strip_prefix("ok:").ok_or_else(|| format!("{cmd}: {r}"))?;
        let parts: Vec<&str> = rest.split(' ').collect();
        if parts.len() != 5 {
            return Err(format!("{cmd}: neocekavana odpoved {r}"));
        }
        Ok((
            parts[0].parse().map_err(|_| format!("stav {r}"))?,
            String::from(parts[1]),
            parts[2].parse().map_err(|_| format!("delka {r}"))?,
            String::from(parts[3]),
            String::from(parts[4]),
        ))
    }
}

fn fnv(data: impl IntoIterator<Item = u8>) -> String {
    let h = data
        .into_iter()
        .fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ u64::from(b)).wrapping_mul(0x100_0000_01b3));
    format!("{h:016x}")
}

/// Vzor tela `/stream` (stejny jako `xtask/src/webhelper.rs`).
fn pattern(i: usize) -> u8 {
    (i.wrapping_mul(7) ^ (i >> 9)) as u8
}

const HELLO: &[u8] = b"ahoj z hostitele";

fn store(s: &Startup) -> TestResult {
    let d = devices().ok_or("zarizeni nebezi")?;
    let fs = d.fs.as_ref().ok_or("FS nebezi")?;
    let initfs = Initfs::open(s).map_err(|e| fail("initfs", e))?;
    let bundle = initfs.find("certs.pem").ok_or("initfs bez certs.pem")?;
    let path = format!("{}/mozilla.pem", crate::certs::DIR);
    let st = fs.stat(&path).map_err(|e| fail("stat mozilla.pem", e))?;
    check_eq!(st.size, bundle.len() as u64, "velikost mozilla.pem");
    check!(fs.stat(&format!("{}/test-ca.pem", crate::certs::DIR)).is_ok(), "test-ca.pem v ulozisti");
    // Opakovane seed nic neprepise (stejny obsah).
    crate::certs::seed(s, d).map_err(|e| fail("seed", e))?;
    let st2 = fs.stat(&path).map_err(|e| fail("stat", e))?;
    check_eq!(st2.oid, st.oid, "stejny obsah = soubor zustane");

    // Relace programu: jen /certs, jen cteni.
    let app = crate::certs::session(d).map_err(|e| fail("relace", e))?;
    let names: Vec<String> = app.list("/certs").map_err(|e| fail("vypis /certs", e))?.into_iter().map(|e| e.name).collect();
    check!(names.iter().any(|n| n == "mozilla.pem"), "vypis obsahuje mozilla.pem");
    check!(app.read("/certs/mozilla.pem", 0, 27).is_ok_and(|d| d.starts_with(b"# Korenove")), "cteni");
    check!(app.write("/certs/mozilla.pem", 0, b"x").is_err(), "zapis do uloziste odmitnut");
    check!(app.create("/certs/vlastni.pem", kind::FILE).is_err(), "vytvoreni souboru odmitnuto");
    check!(app.stat("/system").is_err(), "mimo pripojeni nic");
    Ok(())
}

fn roots(s: &Startup) -> TestResult {
    let Some((f, _)) = fetch(s)? else {
        return Ok(());
    };
    let r = f.run("roots")?;
    let rest = r.strip_prefix("ok:").ok_or_else(|| format!("roots: {r}"))?;
    let count: usize = rest.split(' ').next().and_then(|n| n.parse().ok()).ok_or_else(|| format!("roots: {r}"))?;
    // Sada Mozilla (> 100 CA) a testovaci CA, nic odmitnuteho.
    check!(count > 100, format!("pocet CA {count}"));
    check!(rest.contains("rejected=0"), format!("odmitnute CA: {r}"));
    Ok(())
}

fn plain_http(s: &Startup) -> TestResult {
    let Some((f, p)) = fetch(s)? else {
        return Ok(());
    };
    let url = format!("http://10.0.2.2:{}/hello", p.http);
    let (status, version, len, hash, final_url) = f.get(&format!("get {url}"))?;
    check_eq!((status, version.as_str(), len), (200, "HTTP/1.1", HELLO.len()), "odpoved");
    check_eq!(hash, fnv(HELLO.iter().copied()), "telo");
    check_eq!(final_url, url, "adresa");
    Ok(())
}

fn https_h2(s: &Startup) -> TestResult {
    let Some((f, p)) = fetch(s)? else {
        return Ok(());
    };
    let (status, version, len, hash, _) = f.get(&format!("get https://10.0.2.2:{}/hello", p.https))?;
    check_eq!((status, version.as_str(), len), (200, "HTTP/2", HELLO.len()), "odpoved");
    check_eq!(hash, fnv(HELLO.iter().copied()), "telo");
    let r = f.run(&format!("body https://10.0.2.2:{}/hello", p.https))?;
    check_eq!(r.as_str(), "ok:ahoj z hostitele", "text tela");
    Ok(())
}

fn https_h1(s: &Startup) -> TestResult {
    let Some((f, p)) = fetch(s)? else {
        return Ok(());
    };
    let (status, version, len, _, _) = f.get(&format!("get https://10.0.2.2:{}/hello", p.https_h1))?;
    check_eq!((status, version.as_str(), len), (200, "HTTP/1.1", HELLO.len()), "odpoved");
    Ok(())
}

fn https_big(s: &Startup) -> TestResult {
    let Some((f, p)) = fetch(s)? else {
        return Ok(());
    };
    let n = 1_000_000;
    let expected = fnv((0..n).map(pattern));
    for (port, version) in [(p.https, "HTTP/2"), (p.https_h1, "HTTP/1.1")] {
        let (status, v, len, hash, _) = f.get(&format!("get https://10.0.2.2:{port}/stream?n={n}"))?;
        check_eq!((status, v.as_str(), len), (200, version, n), format!("odpoved {version}"));
        check_eq!(hash, expected, format!("telo {version}"));
    }
    Ok(())
}

fn https_post(s: &Startup) -> TestResult {
    let Some((f, p)) = fetch(s)? else {
        return Ok(());
    };
    let body = "data od klienta pres TLS";
    for port in [p.https, p.https_h1] {
        let (status, _, len, hash, _) = f.get(&format!("post https://10.0.2.2:{port}/echo {body}"))?;
        check_eq!((status, len), (200, body.len()), "odpoved");
        check_eq!(hash, fnv(body.bytes()), "vracene telo");
    }
    Ok(())
}

fn https_rejects(s: &Startup) -> TestResult {
    let Some((f, p)) = fetch(s)? else {
        return Ok(());
    };
    let r = f.run(&format!("get https://10.0.2.2:{}/hello", p.untrusted))?;
    check!(r.starts_with("err:ACCESS_DENIED") && r.contains("UnknownIssuer"), format!("cizi CA: {r}"));
    let r = f.run(&format!("get https://10.0.2.2:{}/hello", p.wrong_name))?;
    check!(r.starts_with("err:ACCESS_DENIED") && r.contains("NotValidForName"), format!("spatne jmeno: {r}"));
    // TLS na port bez TLS: chyba protokolu, ne zaseknuti.
    let r = f.run(&format!("get https://10.0.2.2:{}/hello", p.http))?;
    check!(r.starts_with("err:"), format!("TLS na HTTP: {r}"));
    Ok(())
}

fn redirects(s: &Startup) -> TestResult {
    let Some((f, p)) = fetch(s)? else {
        return Ok(());
    };
    let (status, _, _, _, final_url) = f.get(&format!("get https://10.0.2.2:{}/redirect?to=/hello", p.https))?;
    check_eq!(status, 200, "presmerovani v ramci https");
    check_eq!(final_url, format!("https://10.0.2.2:{}/hello", p.https), "konecna adresa");
    let r = f.run(&format!(
        "get https://10.0.2.2:{}/redirect?to=http://10.0.2.2:{}/hello",
        p.https, p.http
    ))?;
    check!(r.starts_with("err:ACCESS_DENIED"), format!("https -> http: {r}"));
    Ok(())
}

fn expect_revoked(f: &Fetch, port: u16, what: &str) -> TestResult {
    let r = f.run(&format!("get https://10.0.2.2:{port}/hello"))?;
    check!(r.starts_with("err:ACCESS_DENIED") && r.contains("Revoked"), format!("{what}: {r}"));
    Ok(())
}

fn revoked_crl(s: &Startup) -> TestResult {
    let Some((f, p)) = fetch(s)? else {
        return Ok(());
    };
    expect_revoked(f, p.revoked, "certifikat v CRL")?;
    // CRL je v mezipameti revocd na FS (sdilena vsemi programy).
    let d = devices().ok_or("zarizeni nebezi")?;
    let fs = d.fs.as_ref().ok_or("FS nebezi")?;
    let files = fs.list(crate::revocation::CRL_DIR).map_err(|e| fail("vypis mezipameti CRL", e))?;
    check!(files.iter().any(|e| e.name.ends_with(".crl")), "CRL ulozena v mezipameti");
    // Neodvolany certifikat se stejnou CRL projde (jen z mezipameti).
    let (status, _, _, _, _) = f.get(&format!("get https://10.0.2.2:{}/hello", p.https))?;
    check_eq!(status, 200, "neodvolany certifikat");
    Ok(())
}

fn revoked_ocsp(s: &Startup) -> TestResult {
    let Some((f, p)) = fetch(s)? else {
        return Ok(());
    };
    expect_revoked(f, p.ocsp_revoked, "OCSP stapling")
}

fn revoked_onecrl(s: &Startup) -> TestResult {
    let Some((f, p)) = fetch(s)? else {
        return Ok(());
    };
    // revocd stahuje OneCRL az v klidu po prvnim dotazu `ca` (handshake na
    // nej neceka): prvni spojeni ho vyvola, pak pockat na seznam.
    let _ = f.run(&format!("get https://10.0.2.2:{}/hello", p.onecrl))?;
    let deadline = time::deadline_after(core::time::Duration::from_secs(30));
    let text = loop {
        let text = revocd_status()?;
        // Zaznam s "enabled": false se nepocita.
        if text.contains("onecrl=1") || time::now() > deadline {
            break text;
        }
        time::sleep(core::time::Duration::from_millis(100));
    };
    check!(text.contains("onecrl=1"), format!("stav revocd do 30 s: {text}"));
    expect_revoked(f, p.onecrl, "OneCRL")
}

fn revocd_status() -> Result<String, String> {
    let r = crate::system::REVOCD.get().ok_or("revocd nebezi")?;
    let _g = crate::system::REVOCD_LOCK.lock();
    r.control.write(b"status", Vec::new()).map_err(|e| fail("status", e))?;
    let m = r
        .control
        .read(Some(time::deadline_after(core::time::Duration::from_secs(5))))
        .map_err(|e| fail("status", e))?;
    Ok(String::from_utf8(m.data).unwrap_or_default())
}
