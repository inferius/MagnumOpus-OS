//! Sifrovane spojeni k resolverum: DoH (RFC 8484) a DoT (RFC 7858).
//!
//! Server se zadava i s adresou: dnsd jmena resolveru nepreklada (preklad
//! jde pres dnsd samotnou). Jmeno slouzi jen pro overeni certifikatu.
//!
//! - DoH: `POST <url>` s `application/dns-message`, HTTP/2 nebo HTTP/1.1
//!   podle ALPN; jedno spojeni na dotaz (httpc zatim spojeni neopakuje).
//! - DoT: jedno trvale spojeni TLS (zpravy s delkou u16); po chybe se
//!   jednou znovu pripoji.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::net::{IpAddr, SocketAddr};
use core::time::Duration;

use httpc::{Connector, Method, Request, Url, Version};
use netproto::io::{Read, Write};
use netproto::{Net, TcpStream};
use tls::TlsStream;

const TIMEOUT: Duration = Duration::from_secs(10);
/// Navazani TCP: kratce, at nefunkcni resolver nezdrzi prechod na dalsi.
const CONNECT: Duration = Duration::from_secs(3);
/// Nejvetsi odpoved DNS (DoT/DoH: az 64 KiB).
const MAX_MESSAGE: usize = 65_535;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Server {
    /// `doh:https://jmeno/cesta@adresa`
    Doh { url: String, addr: IpAddr },
    /// `dot:jmeno@adresa[:port]`
    Dot { name: String, addr: SocketAddr },
}

impl Server {
    pub fn parse(s: &str) -> Option<Self> {
        if let Some(rest) = s.strip_prefix("doh:") {
            let (url, addr) = rest.rsplit_once('@')?;
            Url::parse(url).ok().filter(|u| u.scheme == httpc::Scheme::Https)?;
            return Some(Self::Doh {
                url: String::from(url),
                addr: addr.parse().ok()?,
            });
        }
        let rest = s.strip_prefix("dot:")?;
        let (name, addr) = rest.rsplit_once('@')?;
        let addr = addr
            .parse::<SocketAddr>()
            .ok()
            .or_else(|| addr.parse::<IpAddr>().ok().map(|ip| SocketAddr::new(ip, 853)))?;
        (!name.is_empty()).then(|| Self::Dot {
            name: String::from(name),
            addr,
        })
    }

    pub fn label(&self) -> String {
        match self {
            Self::Doh { url, .. } => format!("DoH {url}"),
            Self::Dot { name, addr } => format!("DoT {name} ({addr})"),
        }
    }
}

/// Spojeni na pevnou adresu (bez prekladu jmena).
struct Fixed<'a> {
    net: &'a Net,
    tls: &'a tls::Client,
    addr: IpAddr,
}

impl Connector for Fixed<'_> {
    type Stream = TlsStream<TcpStream>;

    fn connect(&mut self, url: &Url) -> httpc::Result<(Self::Stream, Version)> {
        let mut tcp = self
            .net
            .tcp_connect(SocketAddr::new(self.addr, url.port), Some(CONNECT))
            .map_err(|e| httpc::Error::Transport(e, format!("{}", self.addr)))?;
        tcp.set_read_timeout(Some(TIMEOUT));
        let s = self
            .tls
            .connect(&url.host, tcp)
            .map_err(|e| httpc::Error::Transport(e.code(), format!("{e}")))?;
        let v = Version::from_alpn(s.alpn_protocol());
        Ok((s, v))
    }
}

pub struct Transport {
    pub server: Server,
    dot: Option<TlsStream<TcpStream>>,
}

impl Transport {
    pub fn new(server: Server) -> Self {
        Self { server, dot: None }
    }

    /// Posle zpravu DNS a vrati odpoved (bajty).
    pub fn exchange(&mut self, net: &Net, tls: &tls::Client, msg: &[u8]) -> Result<Vec<u8>, String> {
        match self.server.clone() {
            Server::Doh { url, addr } => {
                let limits = httpc::Limits {
                    max_body: MAX_MESSAGE,
                    max_redirects: 0,
                    ..httpc::Limits::default()
                };
                let mut client = httpc::Client::new(Fixed { net, tls, addr }, limits);
                let req = Request {
                    body: msg.to_vec(),
                    ..Request::new(Method::Post, Url::parse(&url).map_err(|e| format!("{e}"))?)
                }
                .header("content-type", "application/dns-message")
                .header("accept", "application/dns-message");
                let resp = client.send(&req).map_err(|e| format!("{e}"))?;
                if resp.status != 200 {
                    return Err(format!("stav HTTP {}", resp.status));
                }
                Ok(resp.body)
            }
            Server::Dot { name, addr } => {
                // Trvale spojeni; po chybe jeden novy pokus.
                for attempt in 0..2 {
                    if self.dot.is_none() {
                        let mut tcp = net.tcp_connect(addr, Some(CONNECT)).map_err(|e| format!("{addr}: {e}"))?;
                        tcp.set_read_timeout(Some(TIMEOUT));
                        self.dot = Some(tls.connect(&name, tcp).map_err(|e| format!("{e}"))?);
                    }
                    let s = self.dot.as_mut().unwrap();
                    match dot_exchange(s, msg) {
                        Ok(r) => return Ok(r),
                        Err(e) => {
                            self.dot = None;
                            if attempt == 1 {
                                return Err(e);
                            }
                        }
                    }
                }
                unreachable!("smycka vraci vzdy")
            }
        }
    }
}

fn dot_exchange(s: &mut TlsStream<TcpStream>, msg: &[u8]) -> Result<Vec<u8>, String> {
    let mut out = (msg.len() as u16).to_be_bytes().to_vec();
    out.extend_from_slice(msg);
    s.write_all(&out).map_err(|e| format!("{e}"))?;
    let mut len = [0u8; 2];
    s.read_exact(&mut len).map_err(|e| format!("{e}"))?;
    let mut body = alloc::vec![0u8; u16::from_be_bytes(len) as usize];
    s.read_exact(&mut body).map_err(|e| format!("{e}"))?;
    Ok(body)
}
