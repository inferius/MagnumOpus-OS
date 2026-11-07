//! Spojeni pro programy: TCP (a pro `https` TLS) pres relaci site,
//! [`httpc::Connector`] nad nim a nacteni korenovych CA ze systemoveho
//! uloziste (`/certs`, relace FS jen pro cteni).
//!
//! Pouzivaji ho `fetch`, `revocd` (stahovani CRL a OneCRL) a dalsi
//! sluzby, ktere mluvi HTTP(S) nebo TLS.
//!
//! ```ignore
//! let roots = web::load_roots(fs.as_ref());
//! let tls = tls::Client::new(&tls::LIBOS, &roots, httpc::ALPN)?;
//! let mut client = httpc::Client::new(web::Web::new(&net, &tls), httpc::Limits::default());
//! let resp = client.get("https://example.com/")?;
//! ```

#![no_std]

extern crate alloc;

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::net::{IpAddr, SocketAddr};
use core::time::Duration;

use fsproto::FsClient;
use httpc::{Connector, Url, Version};
use libos::{Error, println};
use netproto::io::{Read, Write};
use netproto::{Net, TcpStream};
use tls::TlsStream;
use tls::roots::Roots;

/// Nejvetsi soubor uloziste certifikatu (sada Mozilla ma ~200 kB).
pub const MAX_CERT_FILE: u64 = 4 << 20;

/// Nacte vsechny soubory z `/certs` (`None` = bez uloziste = zadna CA).
pub fn load_roots(fs: Option<&FsClient>) -> Roots {
    let Some(fs) = fs else {
        return Roots::empty();
    };
    let entries = match fs.list("/certs") {
        Ok(e) => e,
        Err(e) => {
            println!("web: uloziste certifikatu: {e}");
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
            Err(err) => println!("web: {path}: {err}"),
        }
    }
    Roots::from_files(files.iter().map(|(n, d)| (n.as_str(), d.as_slice())))
}

/// Spojeni k serveru: holy TCP nebo TLS nad nim.
pub enum Conn {
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

/// Spojeni TCP na jmeno nebo adresu s casovym limitem (i pro cteni).
pub fn tcp(net: &Net, host: &str, port: u16, timeout: Duration) -> libos::Result<TcpStream> {
    let mut s = match host.parse::<IpAddr>() {
        Ok(ip) => net.tcp_connect(SocketAddr::new(ip, port), Some(timeout)),
        Err(_) => net.tcp_connect_host(host, port, Some(timeout)),
    }?;
    s.set_read_timeout(Some(timeout));
    Ok(s)
}

/// [`httpc::Connector`] nad relaci site a klientem TLS.
pub struct Web<'a> {
    pub net: &'a Net,
    pub tls: &'a tls::Client,
    pub timeout: Duration,
}

impl<'a> Web<'a> {
    pub fn new(net: &'a Net, tls: &'a tls::Client) -> Self {
        Self {
            net,
            tls,
            timeout: Duration::from_secs(30),
        }
    }
}

impl Connector for Web<'_> {
    type Stream = Conn;

    fn connect(&mut self, url: &Url) -> httpc::Result<(Conn, Version)> {
        let tcp = tcp(self.net, &url.host, url.port, self.timeout)
            .map_err(|e| httpc::Error::Transport(e, format!("{}:{}", url.host, url.port)))?;
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
