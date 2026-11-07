//! revocd: sluzba odvolani certifikatu.
//!
//! Programy s TLS (pres kanal od init, `tls::revocation::ChannelSource`) se
//! ptaji:
//! - `crl <url>` - CRL z distribucniho bodu certifikatu. revocd ji vrati
//!   z mezipameti, nebo stahne (HTTP/HTTPS, nejvys 16 MiB) a ulozi do
//!   `/crl` (= `/system/config/certs/crl`). Odpoved `ok` + DER nebo `none`.
//!   Podpis CRL overuje az TLS (webpki) v programu.
//! - `ca` + dotaz - je CA na seznamu OneCRL? `revoked` / `ok`.
//!
//! Handly: 1. relace site (TCP 80/443, DNS), 2. relace FS s `/certs` (jen
//! cteni, korenove CA pro https) a `/crl` (mezipamet). Bootstrap: prvni
//! zprava = konfigurace TOML (`revocd.toml`), odpoved `ok`; potom `client`
//! + handle kanalu programu, `status`.
//!
//! Dotazy se vyrizuji postupne: stahovani CRL pro jeden program zdrzi
//! ostatni (nejvys casovy limit spojeni). OneCRL (megabajty) se ale
//! nestahuje uprostred dotazu: `ca` odpovi hned z posledniho seznamu
//! (z FS, pri prvnim startu prazdny = stav nezname) a stazeni probehne,
//! az se klienti [`IDLE`] neozvou - handshake programu (napr. NTS-KE
//! ntpd pri startu) tak na nej neceka.

#![no_std]
#![no_main]

extern crate alloc;

mod json;
mod onecrl;
mod store;

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::time::Duration;

use fsproto::FsClient;
use httpc::{Client, Limits};
use libos::{Channel, Error, Signals, Startup, println, time};
use netproto::Net;
use tls::revocation::proto;

use onecrl::OneCrl;
use store::Entry;

libos::entry!(main);

const HOUR: i64 = 3600;
/// Klid (zadny dotaz), po kterem se stahne OneCRL.
const IDLE: Duration = Duration::from_millis(500);
const ONECRL_PATH: &str = "/crl/onecrl.json";
/// Nejvetsi CRL (velke CA deli CRL na casti; jedna cast byva do MB).
const MAX_CRL: usize = 16 << 20;
/// OneCRL ma stovky zaznamu (~1 MB JSON).
const MAX_ONECRL: usize = 8 << 20;

struct Config {
    onecrl: Option<String>,
    onecrl_refresh: i64,
    crl_default: i64,
    retry: i64,
}

fn config(text: &str) -> Result<Config, String> {
    let t = conf::parse(text).map_err(|e| format!("{e}"))?;
    let hours = |k: &str, d: i64| t.get_int(k).unwrap_or(d);
    Ok(Config {
        onecrl: t.get_str("onecrl").map(String::from).filter(|s| !s.is_empty()),
        onecrl_refresh: hours("onecrl_refresh_hours", 24).max(1) * HOUR,
        crl_default: hours("crl_default_hours", 24).max(1) * HOUR,
        retry: t.get_int("retry_minutes").unwrap_or(10).max(1) * 60,
    })
}

struct Service<'a> {
    cfg: Config,
    fs: Option<FsClient>,
    http: Client<web::Web<'a>>,
    mem: Vec<Entry>,
    failed: Vec<(String, i64)>,
    onecrl: OneCrl,
    /// Kdy (realny cas) OneCRL znovu stahnout.
    onecrl_next: i64,
    /// Nekdo se ptal na OneCRL: stahovat (az v klidu, viz [`IDLE`]).
    onecrl_wanted: bool,
    fetched: u64,
}

fn wall() -> i64 {
    time::wall().map_or(0, |t| t.secs)
}

impl Service<'_> {
    fn download(&mut self, url: &str, max: usize) -> Option<Vec<u8>> {
        self.http.limits.max_body = max;
        match self.http.get(url) {
            Ok(r) if r.status == 200 => Some(r.body),
            Ok(r) => {
                println!("revocd: {url}: stav {}", r.status);
                None
            }
            Err(e) => {
                println!("revocd: {url}: {e}");
                None
            }
        }
    }

    fn crl(&mut self, url: &str) -> Option<Vec<u8>> {
        let now = wall();
        if let Some(e) = self.mem.iter().find(|e| e.url == url && e.valid_until > now) {
            return Some(e.der.clone());
        }
        let path = store::crl_path(url);
        // Z FS (po restartu).
        if let Some(fs) = &self.fs
            && let Ok(Some((der, mtime))) = store::load(fs, &path)
            && let Some(until) = store::valid_until(&der, mtime, self.cfg.crl_default)
            && until > now
        {
            self.remember(url, der.clone(), until);
            return Some(der);
        }
        if self.failed.iter().any(|(u, t)| u == url && now < *t + self.cfg.retry) {
            return None;
        }
        if !(url.starts_with("http://") || url.starts_with("https://")) {
            return None;
        }
        let Some(der) = self.download(url, MAX_CRL) else {
            self.failed.retain(|(u, _)| u != url);
            self.failed.push((String::from(url), now));
            return None;
        };
        let Some(until) = store::valid_until(&der, now, self.cfg.crl_default) else {
            println!("revocd: {url}: neni CRL");
            self.failed.push((String::from(url), now));
            return None;
        };
        self.fetched += 1;
        if let Some(fs) = &self.fs
            && let Err(e) = store::save(fs, &path, &der)
        {
            println!("revocd: ulozeni {path}: {e}");
        }
        self.remember(url, der.clone(), until);
        // Propadla hned po stazeni (spatne hodiny serveru): nepouzit.
        (until > now).then_some(der)
    }

    fn remember(&mut self, url: &str, der: Vec<u8>, valid_until: i64) {
        self.mem.retain(|e| e.url != url);
        if self.mem.len() >= store::MEM_ENTRIES {
            self.mem.remove(0);
        }
        self.mem.push(Entry {
            url: String::from(url),
            der,
            valid_until,
        });
    }

    /// Nacte OneCRL z FS a pripadne stahne novy.
    fn refresh_onecrl(&mut self, force: bool) {
        let Some(url) = self.cfg.onecrl.clone() else {
            return;
        };
        let now = wall();
        if !force && now < self.onecrl_next {
            return;
        }
        if let Some(data) = self.download(&url, MAX_ONECRL) {
            match core::str::from_utf8(&data).ok().and_then(OneCrl::parse) {
                Some(list) => {
                    println!("revocd: OneCRL: {} zaznamu", list.len());
                    self.onecrl = list;
                    if let Some(fs) = &self.fs
                        && let Err(e) = store::save(fs, ONECRL_PATH, &data)
                    {
                        println!("revocd: ulozeni OneCRL: {e}");
                    }
                    self.onecrl_next = now + self.cfg.onecrl_refresh;
                    return;
                }
                None => println!("revocd: OneCRL: neplatny dokument"),
            }
        }
        // Neuspech: zkusit znovu za `retry`, do te doby plati predchozi.
        self.onecrl_next = now + self.cfg.retry;
    }

    fn handle(&mut self, msg: &[u8]) -> Vec<u8> {
        if let Some(url) = msg.strip_prefix(proto::CRL) {
            let Ok(url) = core::str::from_utf8(url) else {
                return Vec::from(&b"none"[..]);
            };
            return match self.crl(url) {
                Some(der) => {
                    let mut out = Vec::from(&b"ok"[..]);
                    out.extend_from_slice(&der);
                    out
                }
                None => Vec::from(&b"none"[..]),
            };
        }
        if let Some(q) = proto::decode_ca(msg) {
            self.onecrl_wanted = self.cfg.onecrl.is_some();
            return Vec::from(if self.onecrl.revoked(&q) { &b"revoked"[..] } else { &b"ok"[..] });
        }
        Vec::from(&b"err:neznamy dotaz"[..])
    }
}

fn main(startup: Startup) -> i64 {
    let bootstrap = startup.bootstrap;
    let mut handles = startup.handles.into_iter();
    let Some(net) = handles.next().map(|h| Net::new(Channel::from_handle(h))) else {
        let _ = bootstrap.write(b"err:bez site", Vec::new());
        return 1;
    };
    let fs = handles.next().map(|h| FsClient::new(Channel::from_handle(h)));
    let cfg = match bootstrap.read(Some(time::deadline_after(Duration::from_secs(10)))) {
        Ok(m) => match core::str::from_utf8(&m.data).map_err(|_| String::from("neni UTF-8")).and_then(config) {
            Ok(c) => c,
            Err(e) => {
                println!("revocd: konfigurace: {e}");
                let _ = bootstrap.write(format!("err:{e}").as_bytes(), Vec::new());
                return 2;
            }
        },
        Err(e) => {
            println!("revocd: bez konfigurace: {e}");
            return 2;
        }
    };
    let roots = web::load_roots(fs.as_ref());
    // Sam sebe se na odvolani neptat (OneCRL pres https jen s OCSP).
    let tls = match tls::Client::new(&tls::LIBOS, &roots, httpc::ALPN) {
        Ok(c) => c,
        Err(e) => {
            println!("revocd: TLS: {e}");
            return 3;
        }
    };
    let mut web = web::Web::new(&net, &tls);
    web.timeout = Duration::from_secs(15);
    let limits = Limits {
        max_body: MAX_CRL,
        ..Limits::default()
    };
    let mut svc = Service {
        cfg,
        fs,
        http: Client::new(web, limits),
        mem: Vec::new(),
        failed: Vec::new(),
        onecrl: OneCrl::default(),
        onecrl_next: 0,
        onecrl_wanted: false,
        fetched: 0,
    };
    // Posledni ulozeny OneCRL hned (sit muze byt jeste bez adresy).
    if let Some(fs) = &svc.fs
        && let Ok(Some((data, mtime))) = store::load(fs, ONECRL_PATH)
        && let Some(list) = core::str::from_utf8(&data).ok().and_then(OneCrl::parse)
    {
        svc.onecrl = list;
        svc.onecrl_next = mtime + svc.cfg.onecrl_refresh;
    }
    if bootstrap.write(b"ok", Vec::new()).is_err() {
        return 4;
    }
    println!(
        "revocd: bezim (OneCRL {}, {} zaznamu, mezipamet {})",
        if svc.cfg.onecrl.is_some() { "ano" } else { "ne" },
        svc.onecrl.len(),
        if svc.fs.is_some() { "na FS" } else { "jen v pameti" }
    );

    let mut clients: Vec<Channel> = Vec::new();
    loop {
        let mut items: Vec<(&libos::Handle, Signals)> = Vec::with_capacity(1 + clients.len());
        items.push((bootstrap.handle(), Signals::READABLE | Signals::PEER_CLOSED));
        for c in &clients {
            items.push((c.handle(), Signals::READABLE | Signals::PEER_CLOSED));
        }
        // OneCRL v klidu: po IDLE bez dotazu, nejdriv v dobe obnovy.
        let deadline = svc.onecrl_wanted.then(|| {
            let wait = Duration::from_secs((svc.onecrl_next - wall()).max(0) as u64).max(IDLE);
            time::deadline_after(wait)
        });
        let observed = match libos::wait_many(&items, deadline) {
            Ok((_, o)) => o,
            Err(Error::TimedOut) => {
                drop(items);
                svc.refresh_onecrl(false);
                continue;
            }
            Err(e) => {
                println!("revocd: cekani: {e}");
                return 5;
            }
        };
        drop(items);
        for i in (0..clients.len()).rev() {
            let s = observed[i + 1];
            if s.contains(Signals::READABLE) {
                while let Ok(msg) = clients[i].try_read() {
                    let reply = svc.handle(&msg.data);
                    let _ = clients[i].write(&reply, Vec::new());
                }
            } else if s.contains(Signals::PEER_CLOSED) {
                clients.swap_remove(i);
            }
        }
        if !observed[0].intersects(Signals::READABLE | Signals::PEER_CLOSED) {
            continue;
        }
        let msg = match bootstrap.try_read() {
            Ok(m) => m,
            Err(Error::ShouldWait) => continue,
            Err(Error::PeerClosed) => return 0,
            Err(e) => {
                println!("revocd: cteni: {e}");
                return 6;
            }
        };
        let reply = if msg.data == b"client" {
            match msg.handles.into_iter().next() {
                Some(h) => {
                    clients.push(Channel::from_handle(h));
                    String::from("ok")
                }
                None => String::from("err:chybi handle"),
            }
        } else if msg.data == b"status" {
            format!(
                "ok:clients={} crls={} fetched={} onecrl={}",
                clients.len(),
                svc.mem.len(),
                svc.fetched,
                svc.onecrl.len()
            )
        } else if msg.data == b"refresh" {
            svc.refresh_onecrl(true);
            format!("ok:onecrl={}", svc.onecrl.len())
        } else {
            String::from("err:neznamy prikaz")
        };
        if bootstrap.write(reply.as_bytes(), Vec::new()).is_err() {
            return 0;
        }
    }
}
