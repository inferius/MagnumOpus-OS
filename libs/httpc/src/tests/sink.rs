//! Telo po castech (`Sink`): hlavicky pred daty, delka, chunked, HTTP/2,
//! presmerovani bez tela pro prijemce, chyba prijemce ukonci pozadavek.

use std::string::String;
use std::vec;
use std::vec::Vec;

use super::Script;
use crate::h2::{self, flags, frame, hpack};
use crate::{Connector, Error, Header, Limits, Method, Request, Result, Sink, Url, Version};

#[derive(Default)]
struct Rec {
    head: Vec<(u16, Option<u64>)>,
    data: Vec<u8>,
    chunks: usize,
    /// Chyba po tolika bajtech (plny disk).
    fail_after: Option<usize>,
}

impl Sink for Rec {
    fn head(&mut self, status: u16, _h: &[Header], len: Option<u64>) -> Result<()> {
        self.head.push((status, len));
        Ok(())
    }
    fn data(&mut self, chunk: &[u8]) -> Result<()> {
        if self.fail_after.is_some_and(|n| self.data.len() + chunk.len() > n) {
            return Err(Error::Io(abi::Error::NoSpace));
        }
        self.data.extend_from_slice(chunk);
        self.chunks += 1;
        Ok(())
    }
}

fn get(url: &str) -> Request {
    Request::new(Method::Get, Url::parse(url).unwrap())
}

#[test]
fn h1_streams_large_body_beyond_max_body() {
    let body: Vec<u8> = (0..300_000u32).map(|i| (i % 251) as u8).collect();
    let mut resp = std::format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n", body.len()).into_bytes();
    resp.extend_from_slice(&body);
    let mut rec = Rec::default();
    // max_body je mensi nez telo: limit hlida jen VecSink, ne Sink.
    let limits = Limits { max_body: 1000, ..Limits::default() };
    let r = crate::h1::send_to(Script::new(&resp), &get("http://h/"), &limits, &mut rec).unwrap();
    assert_eq!((r.status, r.body.len()), (200, 0));
    assert_eq!(rec.head, vec![(200, Some(body.len() as u64))]);
    assert_eq!(rec.data, body);
    assert!(rec.chunks >= 2, "po castech");
    // VecSink stejne odpovedi limit dodrzi.
    assert_eq!(crate::h1::send(Script::new(&resp), &get("http://h/"), &limits), Err(Error::TooLarge("telo odpovedi")));
    // Chunked a do konce spojeni: delka neznama.
    let mut rec = Rec::default();
    crate::h1::send_to(Script::new(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n3\r\nabc\r\n0\r\n\r\n"), &get("http://h/"), &limits, &mut rec).unwrap();
    assert_eq!((rec.head[0].1, rec.data.as_slice()), (None, &b"abc"[..]));
}

#[test]
fn sink_error_stops_request() {
    let mut resp = b"HTTP/1.1 200 OK\r\nContent-Length: 200000\r\n\r\n".to_vec();
    resp.extend(std::iter::repeat_n(7u8, 200_000));
    let mut rec = Rec { fail_after: Some(100_000), ..Rec::default() };
    let r = crate::h1::send_to(Script::new(&resp), &get("http://h/"), &Limits::default(), &mut rec);
    assert_eq!(r.map(|_| ()), Err(Error::Io(abi::Error::NoSpace)));
    assert_eq!(r_code(abi::Error::NoSpace), abi::Error::NoSpace);
}

fn r_code(e: abi::Error) -> abi::Error {
    Error::Io(e).code()
}

fn fr(kind: u8, fl: u8, stream: u32, payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    h2::frame_header(&mut out, payload.len(), kind, fl, stream);
    out.extend_from_slice(payload);
    out
}

#[test]
fn h2_streams_body() {
    let mut s = fr(frame::SETTINGS, 0, 0, &[]);
    s.extend(fr(frame::SETTINGS, flags::ACK, 0, &[]));
    s.extend(fr(frame::HEADERS, flags::END_HEADERS, 1, &hpack::encode(&[(":status", "200"), ("content-length", "8")])));
    s.extend(fr(frame::DATA, 0, 1, b"ahoj"));
    s.extend(fr(frame::DATA, flags::END_STREAM, 1, b"svet"));
    let mut rec = Rec::default();
    let limits = Limits { max_body: 2, ..Limits::default() };
    crate::h2::send_to(Script::new(&s), &get("https://h/"), &limits, &mut rec).unwrap();
    assert_eq!((rec.head.as_slice(), rec.data.as_slice(), rec.chunks), (&[(200, Some(8))][..], &b"ahojsvet"[..], 2));
}

/// Skriptovane odpovedi po spojenich (presmerovani).
struct Scripts(Vec<Vec<u8>>);

impl Connector for Scripts {
    type Stream = Script;
    fn connect(&mut self, _url: &Url) -> Result<(Script, Version)> {
        Ok((Script::new(&self.0.remove(0)), Version::Http11))
    }
}

#[test]
fn redirect_body_is_not_passed() {
    let first = b"HTTP/1.1 302 Found\r\nLocation: /cil\r\nContent-Length: 12\r\n\r\nzahodit tohle".to_vec();
    let second = b"HTTP/1.1 200 OK\r\nContent-Length: 3\r\n\r\nabc".to_vec();
    let mut client = crate::Client::new(Scripts(vec![first, second]), Limits::default());
    let mut rec = Rec::default();
    let r = client.request_to(get("http://h/start"), &mut rec).unwrap();
    assert_eq!(r.status, 200);
    assert_eq!(rec.head, vec![(200, Some(3))]);
    assert_eq!(rec.data, b"abc");
    assert_eq!(r.url.map(|u| u.path), Some(String::from("/cil")));
}
