//! Klient proti serveru hyper (nezavisla implementace HTTP/1.1 a HTTP/2).
//!
//! Cesty serveru:
//! - `/hello` - kratky text s delkou,
//! - `/stream?n=N` - N bajtu vzoru po kusech bez delky (HTTP/1.1 chunked,
//!   HTTP/2 vic ramcu DATA a rizeni toku),
//! - `/echo` - vrati telo pozadavku, hlavicka `x-method` = metoda,
//! - `/headers` - vrati hlavicky pozadavku jako text,
//! - `/many` - 60 hlavicek (HPACK s dynamickou tabulkou a Huffmanem),
//! - `/redirect?to=URL&code=C` - presmerovani,
//! - `/loop` - presmerovani samo na sebe.

use std::convert::Infallible;
use std::net::TcpStream;
use std::pin::Pin;
use std::string::{String, ToString};
use std::task::{Context, Poll};
use std::vec::Vec;
use std::{format, thread};

use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::body::{Body, Frame, Incoming};
use hyper::service::service_fn;
use hyper_util::rt::{TokioExecutor, TokioIo};
use netproto::io::{Read, Write};

use crate::{Client, Connector, Error, Limits, Method, Request, Url, Version};

fn pattern(i: usize) -> u8 {
    (i.wrapping_mul(7) ^ (i >> 9)) as u8
}

/// Telo po kusech bez znamé delky (HTTP/1.1 pak pouzije chunked).
struct Chunks {
    left: usize,
    sent: usize,
}

impl Body for Chunks {
    type Data = Bytes;
    type Error = Infallible;

    fn poll_frame(mut self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Option<Result<Frame<Bytes>, Infallible>>> {
        if self.left == 0 {
            return Poll::Ready(None);
        }
        let n = self.left.min(7000);
        let start = self.sent;
        let data: Vec<u8> = (start..start + n).map(pattern).collect();
        self.left -= n;
        self.sent += n;
        Poll::Ready(Some(Ok(Frame::data(Bytes::from(data)))))
    }
}

type Resp = hyper::Response<http_body_util::combinators::BoxBody<Bytes, Infallible>>;

fn full(status: u16, body: impl Into<Bytes>) -> Resp {
    hyper::Response::builder()
        .status(status)
        .body(Full::new(body.into()).boxed())
        .unwrap()
}

fn query<'a>(uri: &'a hyper::Uri, key: &str) -> Option<&'a str> {
    uri.query()?.split('&').find_map(|kv| kv.strip_prefix(key)?.strip_prefix('='))
}

async fn handle(req: hyper::Request<Incoming>) -> Result<Resp, Infallible> {
    let uri = req.uri().clone();
    Ok(match uri.path() {
        "/hello" => full(200, "ahoj svete"),
        "/stream" => {
            let n = query(&uri, "n").and_then(|n| n.parse().ok()).unwrap_or(0);
            hyper::Response::new(Chunks { left: n, sent: 0 }.boxed())
        }
        "/echo" => {
            let method = req.method().to_string();
            let body = req.into_body().collect().await.unwrap().to_bytes();
            let mut r = full(200, body);
            r.headers_mut().insert("x-method", method.parse().unwrap());
            r
        }
        "/headers" => {
            let mut text = String::new();
            for (n, v) in req.headers() {
                text.push_str(&format!("{}: {}\n", n, v.to_str().unwrap_or("?")));
            }
            full(200, text)
        }
        "/many" => {
            let mut r = full(200, "hodne hlavicek");
            for i in 0..60 {
                r.headers_mut().append(
                    hyper::header::HeaderName::from_bytes(format!("x-header-{i}").as_bytes()).unwrap(),
                    format!("hodnota cislo {i} s textem").parse().unwrap(),
                );
            }
            r
        }
        "/redirect" => {
            let to = query(&uri, "to").unwrap_or("/hello").replace("%3F", "?").replace("%3D", "=").replace("%26", "&");
            let code = query(&uri, "code").and_then(|c| c.parse().ok()).unwrap_or(302);
            let mut r = full(code, "");
            r.headers_mut().insert("location", to.parse().unwrap());
            r
        }
        "/loop" => {
            let mut r = full(302, "");
            r.headers_mut().insert("location", "/loop".parse().unwrap());
            r
        }
        _ => full(404, "nenalezeno"),
    })
}

/// Spusti server (HTTP/1.1 nebo HTTP/2 h2c) na nahodnem portu.
fn server(version: Version) -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    listener.set_nonblocking(true).unwrap();
    thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().unwrap();
        rt.block_on(async move {
            let listener = tokio::net::TcpListener::from_std(listener).unwrap();
            loop {
                let (tcp, _) = listener.accept().await.unwrap();
                let io = TokioIo::new(tcp);
                tokio::spawn(async move {
                    let svc = service_fn(handle);
                    let _ = match version {
                        Version::Http11 => hyper::server::conn::http1::Builder::new().serve_connection(io, svc).await,
                        Version::Http2 => {
                            hyper::server::conn::http2::Builder::new(TokioExecutor::new()).serve_connection(io, svc).await
                        }
                    };
                });
            }
        });
    });
    port
}

struct Sock(TcpStream);

impl Read for Sock {
    fn read(&mut self, buf: &mut [u8]) -> abi::Result<usize> {
        std::io::Read::read(&mut self.0, buf).map_err(|_| abi::Error::ConnectionReset)
    }
}

impl Write for Sock {
    fn write(&mut self, buf: &[u8]) -> abi::Result<usize> {
        std::io::Write::write(&mut self.0, buf).map_err(|_| abi::Error::ConnectionReset)
    }

    fn flush(&mut self) -> abi::Result<()> {
        Ok(())
    }
}

/// Spojeni na 127.0.0.1 s pevnou verzi (adresa v URL se ignoruje krome
/// portu - testy pouzivaji jmena pro presmerovani mezi "servery").
struct Local {
    version: Version,
    connects: usize,
}

impl Connector for Local {
    type Stream = Sock;

    fn connect(&mut self, url: &Url) -> crate::Result<(Sock, Version)> {
        self.connects += 1;
        let tcp = TcpStream::connect(("127.0.0.1", url.port)).map_err(|_| Error::Io(abi::Error::ConnectionRefused))?;
        Ok((Sock(tcp), self.version))
    }
}

fn client(version: Version) -> (Client<Local>, u16) {
    let port = server(version);
    (Client::new(Local { version, connects: 0 }, Limits::default()), port)
}

const VERSIONS: [Version; 2] = [Version::Http11, Version::Http2];

#[test]
fn hello_and_404() {
    for v in VERSIONS {
        let (mut c, port) = client(v);
        let r = c.get(&format!("http://localhost:{port}/hello")).unwrap();
        assert_eq!((r.status, r.version, r.body.as_slice()), (200, v, &b"ahoj svete"[..]));
        assert_eq!(r.header("content-length"), Some("10"));
        let r = c.get(&format!("http://localhost:{port}/nic")).unwrap();
        assert_eq!(r.status, 404);
    }
}

#[test]
fn streamed_body() {
    // 3 MB: HTTP/1.1 chunked, HTTP/2 nad okno 1 MiB (WINDOW_UPDATE).
    for v in VERSIONS {
        let (mut c, port) = client(v);
        let n = 3_000_000;
        let r = c.get(&format!("http://localhost:{port}/stream?n={n}")).unwrap();
        assert_eq!(r.body.len(), n, "{v:?}");
        assert!(r.body.iter().enumerate().all(|(i, &b)| b == pattern(i)), "{v:?}");
        if v == Version::Http11 {
            assert_eq!(r.header("transfer-encoding"), Some("chunked"));
        }
    }
}

#[test]
fn post_echo() {
    // Telo vetsi nez vychozi okno HTTP/2 (65535 B) i ramec (16 KiB).
    for v in VERSIONS {
        let (mut c, port) = client(v);
        let body: Vec<u8> = (0..200_000).map(|i| pattern(i * 3)).collect();
        let req = Request {
            body: body.clone(),
            ..Request::new(Method::Post, Url::parse(&format!("http://localhost:{port}/echo")).unwrap())
        };
        let r = c.request(req).unwrap();
        assert_eq!((r.status, r.header("x-method")), (200, Some("POST")), "{v:?}");
        assert!(r.body == body, "{v:?}");
    }
}

#[test]
fn request_headers() {
    for v in VERSIONS {
        let (mut c, port) = client(v);
        let req = Request::new(Method::Get, Url::parse(&format!("http://localhost:{port}/headers")).unwrap())
            .header("X-Custom", "hodnota");
        let r = c.request(req).unwrap();
        let text = String::from_utf8(r.body).unwrap();
        assert!(text.contains("x-custom: hodnota"), "{v:?}: {text}");
        assert!(text.contains(&format!("user-agent: {}", crate::USER_AGENT)), "{text}");
        assert!(text.contains("accept-encoding: identity"), "{text}");
        if v == Version::Http11 {
            assert!(text.contains(&format!("host: localhost:{port}")), "{text}");
        }
    }
}

#[test]
fn many_headers_hpack() {
    for v in VERSIONS {
        let (mut c, port) = client(v);
        // Dvakrat: podruhe jine spojeni (nova tabulka HPACK).
        for _ in 0..2 {
            let r = c.get(&format!("http://localhost:{port}/many")).unwrap();
            assert_eq!(r.header("x-header-59"), Some("hodnota cislo 59 s textem"), "{v:?}");
            assert_eq!(r.headers.iter().filter(|(n, _)| n.starts_with("x-header-")).count(), 60);
        }
    }
}

#[test]
fn head_has_no_body() {
    for v in VERSIONS {
        let (mut c, port) = client(v);
        let r = c.request(Request::new(Method::Head, Url::parse(&format!("http://localhost:{port}/hello")).unwrap())).unwrap();
        assert_eq!((r.status, r.body.len()), (200, 0), "{v:?}");
    }
}

#[test]
fn redirects() {
    for v in VERSIONS {
        let (mut c, port) = client(v);
        // Relativni a absolutni presmerovani; 303 z POST = GET bez tela.
        let r = c.get(&format!("http://localhost:{port}/redirect?to=/redirect%3Fto=hello")).unwrap();
        assert_eq!((r.status, r.body.as_slice()), (200, &b"ahoj svete"[..]), "{v:?}");
        assert_eq!(r.url.unwrap().path, "/hello");
        let to = format!("http://127.0.0.1:{port}/echo");
        let req = Request {
            body: b"telo".to_vec(),
            ..Request::new(Method::Post, Url::parse(&format!("http://localhost:{port}/redirect?to={to}&code=303")).unwrap())
        };
        let r = c.request(req).unwrap();
        assert_eq!((r.header("x-method"), r.body.len()), (Some("GET"), 0));
        // 307 zachova metodu i telo.
        let req = Request {
            body: b"telo".to_vec(),
            ..Request::new(Method::Post, Url::parse(&format!("http://localhost:{port}/redirect?to=/echo&code=307")).unwrap())
        };
        let r = c.request(req).unwrap();
        assert_eq!((r.header("x-method"), r.body.as_slice()), (Some("POST"), &b"telo"[..]));
        // Smycka: limit presmerovani (prvni pozadavek + 5 presmerovani).
        c.connector.connects = 0;
        assert_eq!(c.get(&format!("http://localhost:{port}/loop")), Err(Error::TooManyRedirects));
        assert_eq!(c.connector.connects, 6);
        // https -> http se odmitne (spojeni pro https tu je obycejne TCP).
        let r = c.get(&format!("https://localhost:{port}/redirect?to=http://localhost:{port}/hello"));
        if v == Version::Http2 {
            assert_eq!(r, Err(Error::InsecureRedirect));
        }
    }
}

#[test]
fn credentials_not_forwarded() {
    let (mut c, port) = client(Version::Http11);
    let port2 = server(Version::Http11);
    let to = format!("http://localhost:{port2}/headers");
    let req = Request::new(Method::Get, Url::parse(&format!("http://localhost:{port}/redirect?to={to}")).unwrap())
        .header("Authorization", "Bearer tajne")
        .header("X-Keep", "ano");
    let r = c.request(req).unwrap();
    let text = String::from_utf8(r.body).unwrap();
    assert!(!text.contains("authorization"), "{text}");
    assert!(text.contains("x-keep: ano"), "{text}");
    // Stejny server: hlavicka zustane.
    let req = Request::new(Method::Get, Url::parse(&format!("http://localhost:{port}/redirect?to=/headers")).unwrap())
        .header("Authorization", "Bearer tajne");
    let text = String::from_utf8(c.request(req).unwrap().body).unwrap();
    assert!(text.contains("authorization: Bearer tajne"), "{text}");
}
