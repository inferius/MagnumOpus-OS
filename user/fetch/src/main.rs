//! fetch: klient HTTP(S) - stahne adresu pres HTTP/1.1 nebo HTTP/2.
//!
//! Dostane od init (podle manifestu `fetch.manifest.toml`):
//! 1. relaci site (jen TCP na porty 80 a 443 a DNS; ostatni = dotaz),
//! 2. relaci FS s jedinym pripojenim `/certs` jen pro cteni (systemove
//!    uloziste korenovych certifikatu). Bez ni se nenavaze zadne spojeni
//!    `https` (zadna duveryhodna CA).
//!
//! Prikazy na bootstrap kanalu (odpoved `ok:...` nebo `err:<JMENO> <popis>`):
//! - `get <url>`, `head <url>`, `post <url> <telo>` -
//!   `ok:<stav> <verze> <delka tela> <FNV-1a 64 tela hex> <konecna adresa>`,
//! - `body <url>` - `ok:<telo>` (text, do 4 KiB; pro testy a ladeni),
//! - `roots` - `ok:<pocet CA> rejected=<n> distrusted=<n>`.
//!
//! Konec kanalu u rodice = konec procesu.

#![no_std]
#![no_main]

extern crate alloc;

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::net::{IpAddr, SocketAddr};
use core::time::Duration;

use fsproto::FsClient;
use httpc::{Client, Connector, Method, Request, Url, Version};
use libos::{Channel, Error, Startup, println};
use netproto::io::{Read, Write};
use netproto::{Net, TcpStream};
use tls::TlsStream;
use tls::roots::Roots;

libos::entry!(main);

/// Nejvetsi soubor uloziste certifikatu (sada Mozilla ma ~250 kB).
const MAX_CERT_FILE: u64 = 4 << 20;
const TIMEOUT: Duration = Duration::from_secs(30);

/// Nacte vsechny soubory z `/certs`.
fn load_roots(fs: Option<&FsClient>) -> Roots {
    let Some(fs) = fs else {
        return Roots::empty();
    };
    let entries = match fs.list("/certs") {
        Ok(e) => e,
        Err(e) => {
            println!("fetch: uloziste certifikatu: {e}");
            return Roots::empty();
        }
    };
    let mut files: Vec<(String, Vec<u8>)> = Vec::new();
    for e in entries.iter().filter(|e| e.kind == fsproto::kind::FILE) {
        let path = format!("/certs/{}", e.name);
        let data = fs
            .stat(&path)
            .and_then(|st| if st.size > MAX_CERT_FILE { Err(Error::OutOfRange) } else { fs.read(&path, 0, st.size as usize) });
        match data {
            Ok(d) => files.push((e.name.clone(), d)),
            Err(err) => println!("fetch: {path}: {err}"),
        }
    }
    Roots::from_files(files.iter().map(|(n, d)| (n.as_str(), d.as_slice())))
}

/// Spojeni k serveru: holy TCP nebo TLS nad nim.
enum Conn {
    Plain(TcpStream),
    Tls(TlsStream<TcpStream>),
}

impl Read for Conn {
    fn read(&mut self, buf: &mut [u8]) -> libos::Result<usize> {
        match self {
            Self::Plain(s) => s.read(buf),
            Self::Tls(s) => s.read(buf),
        }
    }
}

impl Write for Conn {
    fn write(&mut self, buf: &[u8]) -> libos::Result<usize> {
        match self {
            Self::Plain(s) => s.write(buf),
            Self::Tls(s) => s.write(buf),
        }
    }

    fn flush(&mut self) -> libos::Result<()> {
        match self {
            Self::Plain(s) => s.flush(),
            Self::Tls(s) => s.flush(),
        }
    }
}

struct Web<'a> {
    net: &'a Net,
    tls: &'a tls::Client,
}

impl Connector for Web<'_> {
    type Stream = Conn;

    fn connect(&mut self, url: &Url) -> httpc::Result<(Conn, Version)> {
        let transport = |e: Error| httpc::Error::Transport(e, format!("{}:{}", url.host, url.port));
        let mut tcp = match url.host.parse::<IpAddr>() {
            Ok(ip) => self.net.tcp_connect(SocketAddr::new(ip, url.port), Some(TIMEOUT)),
            Err(_) => self.net.tcp_connect_host(&url.host, url.port, Some(TIMEOUT)),
        }
        .map_err(transport)?;
        tcp.set_read_timeout(Some(TIMEOUT));
        match url.scheme {
            httpc::Scheme::Http => Ok((Conn::Plain(tcp), Version::Http11)),
            httpc::Scheme::Https => {
                let s = self
                    .tls
                    .connect(&url.host, tcp)
                    .map_err(|e| httpc::Error::Transport(e.code(), format!("{e}")))?;
                let version = Version::from_alpn(s.alpn_protocol());
                Ok((Conn::Tls(s), version))
            }
        }
    }
}

/// FNV-1a 64 (kontrolni soucet tela pro testy).
fn fnv(data: &[u8]) -> u64 {
    data.iter().fold(0xcbf2_9ce4_8422_2325, |h, &b| (h ^ u64::from(b)).wrapping_mul(0x100_0000_01b3))
}

fn main(startup: Startup) -> i64 {
    let ch = startup.bootstrap;
    let mut handles = startup.handles.into_iter();
    let Some(net) = handles.next().map(|h| Net::new(Channel::from_handle(h))) else {
        let _ = ch.write(b"err:NOT_FOUND bez site", Vec::new());
        return 1;
    };
    let fs = handles.next().map(|h| FsClient::new(Channel::from_handle(h)));
    let roots = load_roots(fs.as_ref());
    let tls = match tls::Client::new(&tls::LIBOS, &roots, httpc::ALPN) {
        Ok(c) => c,
        Err(e) => {
            println!("fetch: TLS: {e}");
            return 2;
        }
    };
    let mut client = Client::new(Web { net: &net, tls: &tls }, httpc::Limits::default());
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
