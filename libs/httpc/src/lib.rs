//! Klient HTTP/1.1 a HTTP/2 pro programy (bez `std`).
//!
//! Knihovna nezavisi na TCP ani na TLS: spojeni otevira [`Connector`]
//! programu (TCP z `netproto`, pro `https` navic TLS z `libs/tls`) a vraci
//! proud `netproto::io::Read + Write` a vyjednany protokol (ALPN `h2` nebo
//! `http/1.1`). [`Client`] k tomu pridava presmerovani a limity.
//!
//! ```ignore
//! let mut client = httpc::Client::new(MyConnector { net, tls }, httpc::Limits::default());
//! let resp = client.get("https://example.com/")?;
//! println!("{} {} B", resp.status, resp.body.len());
//! ```
//!
//! Zamerne jednoduche: jeden pozadavek na spojeni (HTTP/1.1
//! `Connection: close`, HTTP/2 jeden proud), cele telo v pameti (do
//! [`Limits::max_body`]), bez komprese (`Accept-Encoding: identity`), bez
//! cookies a bez cache.

#![no_std]

extern crate alloc;
#[cfg(test)]
extern crate std;

mod buf;
pub mod h1;
pub mod h2;
pub mod url;

#[cfg(test)]
mod tests;

use alloc::string::String;
use alloc::vec::Vec;

use netproto::io::{Read, Write};

pub use url::{Scheme, Url};

/// Chyba pozadavku.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// Chyba proudu (TCP, TLS).
    Io(abi::Error),
    /// Spojeni se nepodarilo navazat (kod a popis od [`Connector`], napr.
    /// neplatny certifikat serveru).
    Transport(abi::Error, String),
    /// Neplatna adresa (popis).
    Url(&'static str),
    /// Neplatny pozadavek programu (napr. hlavicka s CR/LF).
    Request(&'static str),
    /// Server porusil protokol (popis).
    Protocol(String),
    /// Prekrocen limit (ktery).
    TooLarge(&'static str),
    /// Vic presmerovani nez [`Limits::max_redirects`].
    TooManyRedirects,
    /// Presmerovani z `https` na `http` (odmitnuto).
    InsecureRedirect,
    /// HTTP/2: server zrusil proud nebo spojeni (kod chyby HTTP/2).
    Reset(u32),
}

impl core::fmt::Display for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "chyba spojeni: {e}"),
            Self::Transport(_, m) => write!(f, "spojeni nelze navazat: {m}"),
            Self::Url(m) => write!(f, "neplatna adresa: {m}"),
            Self::Request(m) => write!(f, "neplatny pozadavek: {m}"),
            Self::Protocol(m) => write!(f, "chyba protokolu HTTP: {m}"),
            Self::TooLarge(m) => write!(f, "prekrocen limit: {m}"),
            Self::TooManyRedirects => f.write_str("prilis mnoho presmerovani"),
            Self::InsecureRedirect => f.write_str("presmerovani z https na http odmitnuto"),
            Self::Reset(c) => write!(f, "server zrusil pozadavek (HTTP/2 kod {c})"),
        }
    }
}

impl From<abi::Error> for Error {
    fn from(e: abi::Error) -> Self {
        Self::Io(e)
    }
}

impl Error {
    /// Kod chyby systemu (pro odpovedi `err:<JMENO>`).
    pub fn code(&self) -> abi::Error {
        match self {
            Self::Io(e) | Self::Transport(e, _) => *e,
            Self::Url(_) | Self::Request(_) => abi::Error::InvalidArgs,
            Self::Protocol(_) | Self::Reset(_) => abi::Error::Corrupted,
            Self::TooLarge(_) => abi::Error::OutOfRange,
            Self::TooManyRedirects => abi::Error::OutOfRange,
            Self::InsecureRedirect => abi::Error::AccessDenied,
        }
    }
}

pub(crate) fn protocol(msg: impl Into<String>) -> Error {
    Error::Protocol(msg.into())
}

pub type Result<T> = core::result::Result<T, Error>;

/// Limity jednoho pozadavku (ochrana pameti programu pred serverem).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// Hlavicky odpovedi celkem (HTTP/1.1 radky, HTTP/2 dekodovany seznam).
    pub max_header_bytes: usize,
    /// Pocet hlavicek odpovedi.
    pub max_headers: usize,
    /// Telo odpovedi.
    pub max_body: usize,
    /// Pocet presmerovani za sebou.
    pub max_redirects: u8,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_header_bytes: 64 * 1024,
            max_headers: 128,
            max_body: 16 * 1024 * 1024,
            max_redirects: 5,
        }
    }
}

/// Metoda pozadavku.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    Get,
    Head,
    Post,
    Put,
    Delete,
    Options,
    Patch,
}

impl Method {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Get => "GET",
            Self::Head => "HEAD",
            Self::Post => "POST",
            Self::Put => "PUT",
            Self::Delete => "DELETE",
            Self::Options => "OPTIONS",
            Self::Patch => "PATCH",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "GET" => Self::Get,
            "HEAD" => Self::Head,
            "POST" => Self::Post,
            "PUT" => Self::Put,
            "DELETE" => Self::Delete,
            "OPTIONS" => Self::Options,
            "PATCH" => Self::Patch,
            _ => return None,
        })
    }
}

/// Verze protokolu spojeni.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Version {
    Http11,
    Http2,
}

impl Version {
    /// Protokol podle ALPN (`h2` = HTTP/2, jinak a bez ALPN HTTP/1.1).
    pub fn from_alpn(alpn: Option<&[u8]>) -> Self {
        if alpn == Some(b"h2") { Self::Http2 } else { Self::Http11 }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Http11 => "HTTP/1.1",
            Self::Http2 => "HTTP/2",
        }
    }
}

/// Protokoly ALPN, ktere klient nabizi pres TLS (preferuje HTTP/2).
pub const ALPN: &[&[u8]] = &[b"h2", b"http/1.1"];

/// Hlavicka (jmeno malymi pismeny).
pub type Header = (String, String);

/// Pozadavek.
#[derive(Debug, Clone)]
pub struct Request {
    pub method: Method,
    pub url: Url,
    /// Dalsi hlavicky (`Host`, `Content-Length`, `Connection` doplni
    /// knihovna; programem zadane se odmitnou).
    pub headers: Vec<Header>,
    pub body: Vec<u8>,
}

impl Request {
    pub fn new(method: Method, url: Url) -> Self {
        Self {
            method,
            url,
            headers: Vec::new(),
            body: Vec::new(),
        }
    }

    pub fn header(mut self, name: &str, value: &str) -> Self {
        self.headers.push((name.to_ascii_lowercase(), String::from(value)));
        self
    }
}

/// Odpoved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Response {
    pub status: u16,
    pub version: Version,
    pub headers: Vec<Header>,
    pub body: Vec<u8>,
    /// Konecna adresa (po presmerovanich).
    pub url: Option<Url>,
}

impl Response {
    /// Prvni hlavicka daneho jmena (bez ohledu na velikost pismen).
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

/// Hlavicky, ktere doplnuje knihovna nebo ktere HTTP/2 zakazuje.
const RESERVED: &[&str] = &[
    "host",
    "content-length",
    "connection",
    "keep-alive",
    "proxy-connection",
    "transfer-encoding",
    "upgrade",
    "te",
];

fn is_token(s: &str) -> bool {
    !s.is_empty()
        && s.bytes().all(|b| {
            b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b)
        })
}

/// Overi hlavicky programu: jmeno je token, hodnota bez CR, LF a NUL
/// (jinak by slo vlozit dalsi hlavicky nebo pozadavek), zadna rezervovana.
pub(crate) fn check_headers(headers: &[Header]) -> Result<()> {
    for (name, value) in headers {
        if !is_token(name) || name.bytes().any(|b| b.is_ascii_uppercase()) {
            return Err(Error::Request("neplatne jmeno hlavicky"));
        }
        if value.bytes().any(|b| b == b'\r' || b == b'\n' || b == 0) {
            return Err(Error::Request("hlavicka obsahuje CR, LF nebo NUL"));
        }
        if RESERVED.contains(&name.as_str()) {
            return Err(Error::Request("hlavicku doplnuje knihovna"));
        }
    }
    Ok(())
}

pub(crate) const USER_AGENT: &str = "MagnumOpus-httpc/0.1";

/// Posle pozadavek po uz otevrenem spojeni a precte odpoved.
pub fn send<S: Read + Write>(stream: S, version: Version, req: &Request, limits: &Limits) -> Result<Response> {
    check_headers(&req.headers)?;
    match version {
        Version::Http11 => h1::send(stream, req, limits),
        Version::Http2 => h2::send(stream, req, limits),
    }
}

/// Otevira spojeni pro [`Client`].
pub trait Connector {
    type Stream: Read + Write;

    /// Spojeni k serveru `url` (pro `https` s TLS a ALPN [`ALPN`]). Vraci
    /// proud a protokol (podle ALPN, bez TLS HTTP/1.1).
    fn connect(&mut self, url: &Url) -> Result<(Self::Stream, Version)>;
}

/// Klient: spojeni pres [`Connector`], presmerovani, limity.
pub struct Client<C> {
    pub connector: C,
    pub limits: Limits,
}

/// Hlavicky, ktere se pri presmerovani na jiny server neposilaji dal.
const CREDENTIALS: &[&str] = &["authorization", "cookie", "proxy-authorization"];

impl<C: Connector> Client<C> {
    pub fn new(connector: C, limits: Limits) -> Self {
        Self { connector, limits }
    }

    /// Jeden pozadavek bez presmerovani.
    pub fn send(&mut self, req: &Request) -> Result<Response> {
        check_headers(&req.headers)?;
        let (stream, version) = self.connector.connect(&req.url)?;
        let mut resp = send(stream, version, req, &self.limits)?;
        resp.url = Some(req.url.clone());
        Ok(resp)
    }

    /// Pozadavek vcetne presmerovani (301, 302, 303, 307, 308).
    ///
    /// - 303 (a 301/302 u jine metody nez GET/HEAD) = dalsi pozadavek GET
    ///   bez tela, 307/308 = stejna metoda i telo.
    /// - `https` -> `http` se odmitne ([`Error::InsecureRedirect`]).
    /// - Na jiny server (schema, jmeno, port) se neposilaji prihlasovaci
    ///   hlavicky (`Authorization`, `Cookie`).
    pub fn request(&mut self, req: Request) -> Result<Response> {
        let mut req = req;
        let mut redirects = 0u8;
        loop {
            let resp = self.send(&req)?;
            let Some(location) = (match resp.status {
                301 | 302 | 303 | 307 | 308 => resp.header("location"),
                _ => None,
            }) else {
                return Ok(resp);
            };
            if redirects >= self.limits.max_redirects {
                return Err(Error::TooManyRedirects);
            }
            redirects += 1;
            let next = req.url.join(location)?;
            if req.url.scheme == Scheme::Https && next.scheme == Scheme::Http {
                return Err(Error::InsecureRedirect);
            }
            if !next.same_origin(&req.url) {
                req.headers.retain(|(n, _)| !CREDENTIALS.contains(&n.as_str()));
            }
            let to_get = resp.status == 303
                || (matches!(resp.status, 301 | 302) && !matches!(req.method, Method::Get | Method::Head));
            if to_get {
                if req.method != Method::Head {
                    req.method = Method::Get;
                }
                req.body.clear();
                req.headers.retain(|(n, _)| !n.starts_with("content-"));
            }
            req.url = next;
        }
    }

    /// GET s presmerovanim.
    pub fn get(&mut self, url: &str) -> Result<Response> {
        self.request(Request::new(Method::Get, Url::parse(url)?))
    }
}
