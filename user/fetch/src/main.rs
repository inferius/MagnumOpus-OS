//! fetch: klient HTTP(S) - stahne adresu pres HTTP/1.1 nebo HTTP/2.
//!
//! Dostane od init (podle manifestu `fetch.manifest.toml`):
//! 1. relaci site (jen TCP na porty 80 a 443 a DNS; ostatni = dotaz),
//! 2. relaci FS s jedinym pripojenim `/certs` jen pro cteni (systemove
//!    uloziste korenovych certifikatu). Bez ni se nenavaze zadne spojeni
//!    `https` (zadna duveryhodna CA).
//! 3. kanal ke sluzbe `revocd` (kontrola odvolani: CRL, OneCRL); bez nej
//!    se kontroluje jen OCSP stapling.
//! 4. (stahovani do souboru, docs/cs/stahovani.md) relace FS jen se slozkou
//!    `/Stazene` (priznak `ORIGIN`),
//! 5. kanal producenta do prehledu uloh (`taskd`); oznameni = handle se
//!    stitkem `abi::params::tag::NOTIFY`.
//!
//! Prikazy na bootstrap kanalu (odpoved `ok:...` nebo `err:<JMENO> <popis>`):
//! - `get <url>`, `head <url>`, `post <url> <telo>` -
//!   `ok:<stav> <verze> <delka tela> <FNV-1a 64 tela hex> <konecna adresa>`,
//! - `body <url>` - `ok:<telo>` (text, do 4 KiB; pro testy a ladeni),
//! - `roots` - `ok:<pocet CA> rejected=<n> distrusted=<n>`,
//! - `save <url> [jmeno]` - stahne do `/Stazene` (jen s relaci 4):
//!   `ok:<jmeno> <bajtu>` nebo `err:<JMENO> <popis pro uzivatele>`;
//!   `save-bg <url>` totez bez odpovedi (vysledek v ulohach a oznameni).
//!
//! Konec kanalu u rodice = konec procesu.

#![no_std]
#![no_main]

extern crate alloc;

use alloc::format;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::time::Duration;

use fsproto::FsClient;
use httpc::{Client, Method, Request, Url};
use libos::{Channel, Error, Startup, println};
use netproto::Net;
use tls::revocation::{ChannelSource, RevocationSource};
use tls::roots::Roots;
use web::Web;

mod save;

libos::entry!(main);

/// FNV-1a 64 (kontrolni soucet tela pro testy).
fn fnv(data: &[u8]) -> u64 {
    data.iter().fold(0xcbf2_9ce4_8422_2325, |h, &b| (h ^ u64::from(b)).wrapping_mul(0x100_0000_01b3))
}

fn main(mut startup: Startup) -> i64 {
    let notify = startup.take_tagged(libos::abi::params::tag::NOTIFY).map(Channel::from_handle);
    let ch = startup.bootstrap;
    let mut handles = startup.handles.into_iter();
    let Some(net) = handles.next().map(|h| Net::new(Channel::from_handle(h))) else {
        let _ = ch.write(b"err:NOT_FOUND bez site", Vec::new());
        return 1;
    };
    let fs = handles.next().map(|h| FsClient::new(Channel::from_handle(h)));
    let roots = web::load_roots(fs.as_ref());
    // Kontrola odvolani (CRL, OneCRL) pres revocd; bez nej jen OCSP stapling.
    let revocation = handles
        .next()
        .map(|h| Arc::new(ChannelSource::new(Channel::from_handle(h), Duration::from_secs(20))) as Arc<dyn RevocationSource>);
    let tls = match tls::Client::with_revocation(&tls::LIBOS, &roots, httpc::ALPN, revocation) {
        Ok(c) => c,
        Err(e) => {
            println!("fetch: TLS: {e}");
            return 2;
        }
    };
    // Stahovani do souboru: relace FS se slozkou Stazene a prehled uloh.
    let mut downloads = handles.next().map(|h| {
        let fs = FsClient::new(Channel::from_handle(h));
        let tasks = handles.next().map(Channel::from_handle);
        save::Downloads::new(fs, tasks, notify)
    });
    let mut client = Client::new(Web::new(&net, &tls), httpc::Limits::default());
    loop {
        let msg = match ch.read(None) {
            Ok(m) => m,
            Err(Error::PeerClosed) => return 0,
            Err(e) => {
                println!("fetch: cteni: {e}");
                return 3;
            }
        };
        let cmd = core::str::from_utf8(&msg.data).unwrap_or("");
        // `save-bg`: bez odpovedi (Soubory; vysledek v ulohach a oznameni).
        let quiet = cmd.starts_with("save-bg ");
        if let Some(rest) = cmd.strip_prefix("save ").or_else(|| cmd.strip_prefix("save-bg ")) {
            let (url, name) = match rest.split_once(' ') {
                Some((u, n)) => (u, Some(n)),
                None => (rest, None),
            };
            let reply = match downloads.as_mut() {
                None => String::from("err:NOT_SUPPORTED stahovani do souboru neni povolene"),
                Some(d) => match save::save(&mut client, d, url, name) {
                    Ok((n, b)) => format!("ok:{n} {b}"),
                    Err((e, m)) => format!("err:{} {m}", e.name()),
                },
            };
            if !quiet && ch.write(reply.as_bytes(), Vec::new()).is_err() {
                return 4;
            }
            continue;
        }
        let reply = match run(&mut client, &roots, cmd) {
            Ok(s) => format!("ok:{s}"),
            Err(e) => format!("err:{} {e}", e.code().name()),
        };
        if ch.write(reply.as_bytes(), Vec::new()).is_err() {
            return 4;
        }
    }
}

fn run(client: &mut Client<Web<'_>>, roots: &Roots, cmd: &str) -> httpc::Result<String> {
    let (verb, rest) = cmd.split_once(' ').unwrap_or((cmd, ""));
    let (url, body) = rest.split_once(' ').unwrap_or((rest, ""));
    let method = match verb {
        "roots" => return Ok(format!("{} rejected={} distrusted={}", roots.len(), roots.rejected, roots.distrusted)),
        "get" | "body" => Method::Get,
        "head" => Method::Head,
        "post" => Method::Post,
        _ => return Err(httpc::Error::Request("neznamy prikaz")),
    };
    let req = Request {
        body: body.as_bytes().to_vec(),
        ..Request::new(method, Url::parse(url)?)
    };
    let resp = client.request(req)?;
    if verb == "body" {
        let text = String::from_utf8_lossy(&resp.body[..resp.body.len().min(4096)]).into_owned();
        return Ok(text);
    }
    Ok(format!(
        "{} {} {} {:016x} {}",
        resp.status,
        resp.version.as_str(),
        resp.body.len(),
        fnv(&resp.body),
        resp.url.as_ref().map(|u| format!("{u}")).unwrap_or_default()
    ))
}
