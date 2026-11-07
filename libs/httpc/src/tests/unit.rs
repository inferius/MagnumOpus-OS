//! Jednotkove testy bez site.

use std::string::{String, ToString};
use std::vec::Vec;
use std::{format, vec};

use super::Script;
use crate::h2::{self, flags, frame, hpack};
use crate::{Error, Limits, Method, Request, Url, Version};

fn hex(s: &str) -> Vec<u8> {
    let s: String = s.chars().filter(|c| !c.is_whitespace()).collect();
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
}

#[test]
fn url_parse() {
    let u = Url::parse("HTTPS://Example.COM/a/b?c=d#frag").unwrap();
    assert_eq!((u.scheme, u.host.as_str(), u.port, u.path.as_str()), (crate::Scheme::Https, "example.com", 443, "/a/b?c=d"));
    assert_eq!(u.authority(), "example.com");
    let u = Url::parse("http://10.0.2.2:8080").unwrap();
    assert_eq!((u.port, u.path.as_str(), u.authority().as_str()), (8080, "/", "10.0.2.2:8080"));
    let u = Url::parse("http://[::1]:81/x y?q").unwrap();
    assert_eq!((u.host.as_str(), u.path.as_str(), u.authority().as_str()), ("::1", "/x%20y?q", "[::1]:81"));
    assert_eq!(Url::parse("http://h?q=1").unwrap().path, "/?q=1");
    assert_eq!(Url::parse("http://h/č").unwrap().path, "/%C4%8D");
    for bad in [
        "ftp://h/",
        "example.com",
        "http://user:pw@h/",
        "http://h:0/",
        "http://h:99999/",
        "http://[::1/",
        "http://h\u{0}/",
        "http:///x",
        "http://h/a\nb",
        "http://žluť.cz/",
    ] {
        assert!(Url::parse(bad).is_err(), "{bad}");
    }
}

#[test]
fn url_join() {
    let base = Url::parse("https://a.cz/x/y/z?q").unwrap();
    let j = |l: &str| base.join(l).unwrap().to_string();
    assert_eq!(j("http://b.cz/p"), "http://b.cz/p");
    assert_eq!(j("//c.cz/p"), "https://c.cz/p");
    assert_eq!(j("/abs?k"), "https://a.cz/abs?k");
    assert_eq!(j("rel"), "https://a.cz/x/y/rel");
    assert_eq!(j("../up"), "https://a.cz/x/up");
    assert_eq!(j("../../../../top"), "https://a.cz/top");
    assert_eq!(j("./"), "https://a.cz/x/y/");
    assert_eq!(j("?new"), "https://a.cz/x/y/z?new");
    assert_eq!(j(""), "https://a.cz/x/y/z?q");
    assert_eq!(j("a/./b/../c"), "https://a.cz/x/y/a/c");
    assert!(base.join("mailto:x@y").is_err());
}

#[test]
fn hpack_integers() {
    // RFC 7541 C.1
    let mut out = Vec::new();
    hpack::encode_int(&mut out, 0, 5, 10);
    assert_eq!(out, [10]);
    out.clear();
    hpack::encode_int(&mut out, 0, 5, 1337);
    assert_eq!(out, [31, 154, 10]);
    assert_eq!(hpack::decode_int(&[31, 154, 10], 5).unwrap(), (1337, 3));
    assert_eq!(hpack::decode_int(&[42], 8).unwrap(), (42, 1));
    assert!(hpack::decode_int(&[31, 0x80, 0x80], 5).is_err());
    assert!(hpack::decode_int(&[31, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x01], 5).is_err());
}

#[test]
fn hpack_huffman() {
    // RFC 7541 C.4.1: "www.example.com"
    assert_eq!(hpack::huffman_decode(&hex("f1e3 c2e5 f23a 6ba0 ab90 f4ff")).unwrap(), b"www.example.com");
    assert_eq!(hpack::huffman_encode(b"www.example.com"), hex("f1e3 c2e5 f23a 6ba0 ab90 f4ff"));
    // Vsechny bajty tam a zpet.
    let all: Vec<u8> = (0..=255u8).chain((0..=255u8).rev()).collect();
    assert_eq!(hpack::huffman_decode(&hpack::huffman_encode(&all)).unwrap(), all);
    // Vypln z nul, vypln > 7 bitu, EOS.
    assert!(hpack::huffman_decode(&[0xf1, 0xe3, 0xc2, 0x00]).is_err());
    assert!(hpack::huffman_decode(&[0xff, 0xff]).is_err());
    assert!(hpack::huffman_decode(&[0xff, 0xff, 0xff, 0xfc]).is_err());
}

fn headers(list: &[(&str, &str)]) -> Vec<(String, String)> {
    list.iter().map(|(n, v)| (String::from(*n), String::from(*v))).collect()
}

#[test]
fn hpack_rfc_requests() {
    // RFC 7541 C.4: tri pozadavky s Huffmanem a dynamickou tabulkou.
    let mut d = hpack::Decoder::new(4096);
    let r1 = d.decode(&hex("8286 8441 8cf1 e3c2 e5f2 3a6b a0ab 90f4 ff"), 4096, 100).unwrap();
    assert_eq!(r1, headers(&[(":method", "GET"), (":scheme", "http"), (":path", "/"), (":authority", "www.example.com")]));
    let r2 = d.decode(&hex("8286 84be 5886 a8eb 1064 9cbf"), 4096, 100).unwrap();
    assert_eq!(
        r2,
        headers(&[(":method", "GET"), (":scheme", "http"), (":path", "/"), (":authority", "www.example.com"), ("cache-control", "no-cache")])
    );
    let r3 = d
        .decode(&hex("8287 85bf 4088 25a8 49e9 5ba9 7d7f 8925 a849 e95b b8e8 b4bf"), 4096, 100)
        .unwrap();
    assert_eq!(
        r3,
        headers(&[
            (":method", "GET"),
            (":scheme", "https"),
            (":path", "/index.html"),
            (":authority", "www.example.com"),
            ("custom-key", "custom-value"),
        ])
    );
}

#[test]
fn hpack_rfc_responses_eviction() {
    // RFC 7541 C.5: odpovedi bez Huffmana, tabulka 256 B (vyrazovani).
    let mut d = hpack::Decoder::new(256);
    let r1 = d
        .decode(
            &hex(
                "4803 3330 3258 0770 7269 7661 7465 611d 4d6f 6e2c 2032 3120 4f63 7420 3230 3133 2032 303a 3133 3a32 3120 474d 546e 1768 7474 7073 3a2f 2f77 7777 2e65 7861 6d70 6c65 2e63 6f6d",
            ),
            4096,
            100,
        )
        .unwrap();
    assert_eq!(
        r1,
        headers(&[
            (":status", "302"),
            ("cache-control", "private"),
            ("date", "Mon, 21 Oct 2013 20:13:21 GMT"),
            ("location", "https://www.example.com"),
        ])
    );
    let r2 = d.decode(&hex("4803 3330 37c1 c0bf"), 4096, 100).unwrap();
    assert_eq!(
        r2,
        headers(&[
            (":status", "307"),
            ("cache-control", "private"),
            ("date", "Mon, 21 Oct 2013 20:13:21 GMT"),
            ("location", "https://www.example.com"),
        ])
    );
}

#[test]
fn hpack_limits() {
    // Odkaz na velkou polozku tabulky opakovany = limit dekodovanych dat.
    let mut block = vec![0x40, 0x01, b'x', 0x7f, 0x81, 0x01];
    block.extend(std::iter::repeat_n(b'a', 200));
    block.extend(std::iter::repeat_n(0xbe, 100));
    let mut d = hpack::Decoder::new(4096);
    assert_eq!(d.decode(&block, 4096, 1000), Err(Error::TooLarge("hlavicky odpovedi")));
    // Index mimo tabulku, index 0, zmena tabulky nad povolenou, zmena
    // uprostred bloku, retezec za koncem.
    let mut d = hpack::Decoder::new(4096);
    for bad in [&[0xc0][..], &[0x80], &[0x3f, 0xe2, 0x1f], &[0x82, 0x20], &[0x00, 0x05, b'a']] {
        assert!(d.decode(bad, 4096, 100).is_err(), "{bad:?}");
    }
    // Zmena velikosti na zacatku je povolena a vyprazdni tabulku.
    let mut d = hpack::Decoder::new(4096);
    d.decode(&[0x40, 0x01, b'k', 0x01, b'v'], 4096, 100).unwrap();
    assert_eq!(d.decode(&[0xbe], 4096, 100).unwrap(), headers(&[("k", "v")]));
    assert!(d.decode(&[0x20, 0xbe], 4096, 100).is_err());
}

fn get(url: &str) -> Request {
    Request::new(Method::Get, Url::parse(url).unwrap())
}

fn h1(response: &[u8]) -> crate::Result<crate::Response> {
    crate::send(Script::new(response), Version::Http11, &get("http://h/p"), &Limits::default())
}

#[test]
fn h1_request_format() {
    let mut s = Script::new(b"HTTP/1.1 204 No Content\r\n\r\n");
    let req = Request::new(Method::Post, Url::parse("http://h:8080/a?b").unwrap())
        .header("X-Test", "1");
    let req = Request { body: b"data".to_vec(), ..req };
    crate::h1::send(&mut s, &req, &Limits::default()).unwrap();
    let text = String::from_utf8(s.written).unwrap();
    assert_eq!(
        text,
        format!(
            "POST /a?b HTTP/1.1\r\nHost: h:8080\r\nUser-Agent: {}\r\nAccept-Encoding: identity\r\nx-test: 1\r\nContent-Length: 4\r\nConnection: close\r\n\r\ndata",
            crate::USER_AGENT
        )
    );
}

#[test]
fn h1_bodies() {
    let r = h1(b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\nX-A:  b \r\n\r\nahoj!navic").unwrap();
    assert_eq!((r.status, r.body.as_slice(), r.header("x-a")), (200, &b"ahoj!"[..], Some("b")));
    let r = h1(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n4;ext=1\r\nahoj\r\nA\r\n0123456789\r\n0\r\nTrailer: x\r\n\r\n").unwrap();
    assert_eq!(r.body, b"ahoj0123456789");
    // TE ma prednost pred Content-Length.
    let r = h1(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nTransfer-Encoding: chunked\r\n\r\n3\r\nabc\r\n0\r\n\r\n").unwrap();
    assert_eq!(r.body, b"abc");
    // Do konce spojeni; 1xx se preskoci; LF bez CR se toleruje.
    let r = h1(b"HTTP/1.1 100 Continue\r\n\r\nHTTP/1.0 200 OK\nServer: x\n\nvse do konce").unwrap();
    assert_eq!((r.status, r.body.as_slice()), (200, &b"vse do konce"[..]));
    // 204 a 304 bez tela i s Content-Length.
    let r = h1(b"HTTP/1.1 304 Not Modified\r\nContent-Length: 10\r\n\r\n").unwrap();
    assert!(r.body.is_empty());
    // Data po kouskach po jednom bajtu.
    let mut s = Script::new(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n2\r\nab\r\n0\r\n\r\n");
    s.step = 1;
    let r = crate::h1::send(s, &get("http://h/"), &Limits::default()).unwrap();
    assert_eq!(r.body, b"ab");
}

#[test]
fn h1_rejects() {
    for bad in [
        &b"HTTP/2 200 OK\r\n\r\n"[..],
        b"HTTP/1.1 2000 OK\r\n\r\n",
        b"HTTP/1.1 099 x\r\n\r\n",
        b"HTTP/1.1 200 OK\r\nX: a\r\n folded\r\n\r\n",
        b"HTTP/1.1 200 OK\r\nBad Name: x\r\n\r\n",
        b"HTTP/1.1 200 OK\r\nX : a\r\n\r\n",
        b"HTTP/1.1 200 OK\r\nContent-Length: 1\r\nContent-Length: 2\r\n\r\nab",
        b"HTTP/1.1 200 OK\r\nContent-Length: -1\r\n\r\n",
        b"HTTP/1.1 200 OK\r\nContent-Length: 10\r\n\r\nkratke",
        b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\nzz\r\n",
        b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n2\r\nabX\r\n",
        b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n2\r\nab\r\n",
        b"HTTP/1.1 101 Switching Protocols\r\n\r\n",
        b"",
        b"HTTP/1.1 200 OK\r\nX: a",
    ] {
        assert!(h1(bad).is_err(), "{}", String::from_utf8_lossy(bad));
    }
}

#[test]
fn h1_limits() {
    let limits = Limits {
        max_header_bytes: 100,
        max_body: 10,
        max_headers: 3,
        ..Limits::default()
    };
    let send = |r: &[u8]| crate::h1::send(Script::new(r), &get("http://h/"), &limits);
    let long = format!("HTTP/1.1 200 OK\r\nX: {}\r\n\r\n", "a".repeat(200));
    assert_eq!(send(long.as_bytes()), Err(Error::TooLarge("radek hlavicky")));
    assert_eq!(send(b"HTTP/1.1 200 OK\r\na: 1\r\nb: 1\r\nc: 1\r\nd: 1\r\n\r\n"), Err(Error::TooLarge("pocet hlavicek")));
    assert_eq!(send(b"HTTP/1.1 200 OK\r\nContent-Length: 11\r\n\r\n"), Err(Error::TooLarge("telo odpovedi")));
    assert_eq!(send(b"HTTP/1.1 200 OK\r\n\r\n0123456789A"), Err(Error::TooLarge("telo odpovedi")));
    assert_eq!(send(b"HTTP/1.1 200 OK\r\n\r\n0123456789").unwrap().body.len(), 10);
    assert_eq!(
        send(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n6\r\n012345\r\n6\r\n012345\r\n0\r\n\r\n"),
        Err(Error::TooLarge("telo odpovedi"))
    );
}

#[test]
fn request_header_injection() {
    for (n, v) in [("x", "a\r\nInjected: 1"), ("bad name", "v"), ("host", "evil"), ("content-length", "1"), ("X-Upper", "v")] {
        let req = Request {
            headers: vec![(String::from(n), String::from(v))],
            ..get("http://h/")
        };
        let r = crate::send(Script::new(b""), Version::Http11, &req, &Limits::default());
        assert!(matches!(r, Err(Error::Request(_))), "{n}: {r:?}");
    }
}

/// Ramec HTTP/2 pro skript serveru.
fn fr(kind: u8, fl: u8, stream: u32, payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    h2::frame_header(&mut out, payload.len(), kind, fl, stream);
    out.extend_from_slice(payload);
    out
}

/// Odpoved serveru: SETTINGS, ACK nasich SETTINGS a dalsi ramce.
fn h2_script(frames: &[Vec<u8>]) -> Vec<u8> {
    let mut s = fr(frame::SETTINGS, 0, 0, &[]);
    s.extend(fr(frame::SETTINGS, flags::ACK, 0, &[]));
    for f in frames {
        s.extend_from_slice(f);
    }
    s
}

fn h2(frames: &[Vec<u8>]) -> crate::Result<crate::Response> {
    crate::send(Script::new(&h2_script(frames)), Version::Http2, &get("https://h/p"), &Limits::default())
}

fn status_block(code: &str, extra: &[(&str, &str)]) -> Vec<u8> {
    let mut list = vec![(":status", code)];
    list.extend_from_slice(extra);
    hpack::encode(&list)
}

#[test]
fn h2_basic_and_request() {
    let mut script = Script::new(&h2_script(&[
        fr(frame::PING, 0, 0, b"12345678"),
        fr(frame::HEADERS, flags::END_HEADERS, 1, &status_block("200", &[("content-length", "4")])),
        // DATA s vyplni (PADDED: delka 3).
        fr(frame::DATA, flags::PADDED, 1, &[3, b'a', b'h', 0, 0, 0]),
        fr(frame::DATA, flags::END_STREAM, 1, b"oj"),
    ]));
    let r = crate::h2::send(&mut script, &get("https://h:444/p?q"), &Limits::default()).unwrap();
    assert_eq!((r.status, r.version, r.body.as_slice(), r.header("content-length")), (200, Version::Http2, &b"ahoj"[..], Some("4")));
    // Klient: preface, SETTINGS, WINDOW_UPDATE, HEADERS s pseudo-hlavickami.
    let w = &script.written;
    assert!(w.starts_with(h2::PREFACE));
    let mut pos = h2::PREFACE.len();
    let mut kinds = Vec::new();
    let mut block = None;
    while pos + 9 <= w.len() {
        let len = u32::from_be_bytes([0, w[pos], w[pos + 1], w[pos + 2]]) as usize;
        kinds.push((w[pos + 3], w[pos + 4]));
        if w[pos + 3] == frame::HEADERS {
            block = Some(w[pos + 9..pos + 9 + len].to_vec());
        }
        pos += 9 + len;
    }
    assert_eq!(pos, w.len());
    assert_eq!(kinds[0], (frame::SETTINGS, 0));
    assert_eq!(kinds[1], (frame::WINDOW_UPDATE, 0));
    assert_eq!(kinds[2], (frame::HEADERS, flags::END_HEADERS | flags::END_STREAM));
    assert!(kinds.contains(&(frame::PING, flags::ACK)));
    assert!(kinds.contains(&(frame::SETTINGS, flags::ACK)));
    assert_eq!(kinds.last(), Some(&(frame::GOAWAY, 0)));
    let list = hpack::Decoder::new(4096).decode(&block.unwrap(), 4096, 100).unwrap();
    assert_eq!(&list[..4], &headers(&[(":method", "GET"), (":scheme", "https"), (":authority", "h:444"), (":path", "/p?q")])[..]);
}

#[test]
fn h2_continuation_trailers_and_1xx() {
    let block = status_block("200", &[("x-long", &"v".repeat(100))]);
    let (a, b) = block.split_at(10);
    let r = h2(&[
        fr(frame::HEADERS, flags::END_HEADERS, 1, &status_block("103", &[("link", "</a>")])),
        fr(frame::HEADERS, 0, 1, a),
        fr(frame::CONTINUATION, flags::END_HEADERS, 1, b),
        fr(frame::DATA, 0, 1, b"telo"),
        fr(frame::HEADERS, flags::END_HEADERS | flags::END_STREAM, 1, &hpack::encode(&[("x-trailer", "1")])),
    ])
    .unwrap();
    assert_eq!((r.status, r.body.as_slice(), r.header("x-long").map(str::len)), (200, &b"telo"[..], Some(100)));
}

#[test]
fn h2_rejects() {
    let ok = || fr(frame::HEADERS, flags::END_HEADERS, 1, &status_block("200", &[]));
    let cases: Vec<(&str, Vec<Vec<u8>>)> = vec![
        ("push", vec![fr(frame::PUSH_PROMISE, flags::END_HEADERS, 1, &[0, 0, 0, 2])]),
        ("data pred hlavickami", vec![fr(frame::DATA, flags::END_STREAM, 1, b"x")]),
        ("cizi proud", vec![fr(frame::HEADERS, flags::END_HEADERS, 3, &status_block("200", &[]))]),
        ("bez :status", vec![fr(frame::HEADERS, flags::END_HEADERS | flags::END_STREAM, 1, &hpack::encode(&[("a", "b")]))]),
        ("velka pismena", vec![fr(frame::HEADERS, flags::END_HEADERS | flags::END_STREAM, 1, &status_block("200", &[("X-A", "b")]))]),
        ("hlavicka spojeni", vec![fr(frame::HEADERS, flags::END_HEADERS | flags::END_STREAM, 1, &status_block("200", &[("connection", "close")]))]),
        ("pseudo po obycejne", vec![fr(frame::HEADERS, flags::END_HEADERS | flags::END_STREAM, 1, &hpack::encode(&[("a", "b"), (":status", "200")]))]),
        ("content-length", vec![fr(frame::HEADERS, flags::END_HEADERS, 1, &status_block("200", &[("content-length", "5")])), fr(frame::DATA, flags::END_STREAM, 1, b"abc")]),
        ("vypln", vec![ok(), fr(frame::DATA, flags::PADDED | flags::END_STREAM, 1, &[5, b'a'])]),
        ("ramec nad limit", vec![fr(frame::DATA, 0, 1, &vec![0; 16_385])]),
        ("rst", vec![fr(frame::RST_STREAM, 0, 1, &7u32.to_be_bytes())]),
        ("goaway pred proudem", vec![fr(frame::GOAWAY, 0, 0, &[0, 0, 0, 0, 0, 0, 0, 0])]),
        ("continuation bez headers", vec![fr(frame::CONTINUATION, flags::END_HEADERS, 1, &[0x88])]),
        ("konec spojeni", vec![ok()]),
        ("window update 0", vec![fr(frame::WINDOW_UPDATE, 0, 0, &[0, 0, 0, 0])]),
        ("settings na proudu", vec![fr(frame::SETTINGS, 0, 1, &[])]),
        ("hpack", vec![fr(frame::HEADERS, flags::END_HEADERS, 1, &[0xff, 0xff, 0xff, 0xff, 0x0f])]),
    ];
    for (name, frames) in cases {
        assert!(h2(&frames).is_err(), "{name}");
    }
    assert_eq!(h2(&[fr(frame::RST_STREAM, 0, 1, &7u32.to_be_bytes())]), Err(Error::Reset(7)));
}

#[test]
fn h2_flow_control_request_body() {
    // Telo 70000 B: nejdriv vychozi okno 65535 B (ramce po 16 KiB), pak jen
    // tolik, kolik server povoli (WINDOW_UPDATE spojeni i proudu).
    let mut input = fr(frame::SETTINGS, 0, 0, &[]);
    input.extend(fr(frame::WINDOW_UPDATE, 0, 0, &5000u32.to_be_bytes()));
    input.extend(fr(frame::WINDOW_UPDATE, 0, 1, &3000u32.to_be_bytes()));
    input.extend(fr(frame::WINDOW_UPDATE, 0, 1, &10000u32.to_be_bytes()));
    input.extend(fr(frame::HEADERS, flags::END_HEADERS | flags::END_STREAM, 1, &status_block("201", &[])));
    let mut script = Script::new(&input);
    let req = Request {
        body: vec![b'x'; 70_000],
        ..Request::new(Method::Post, Url::parse("http://h/").unwrap())
    };
    let r = crate::h2::send(&mut script, &req, &Limits::default()).unwrap();
    assert_eq!(r.status, 201);
    let w = &script.written[h2::PREFACE.len()..];
    let mut pos = 0;
    let mut data = Vec::new();
    while pos + 9 <= w.len() {
        let len = u32::from_be_bytes([0, w[pos], w[pos + 1], w[pos + 2]]) as usize;
        if w[pos + 3] == frame::DATA {
            data.push((len, w[pos + 4]));
        }
        pos += 9 + len;
    }
    assert_eq!(data, [(16384, 0), (16384, 0), (16384, 0), (16383, 0), (3000, 0), (1465, flags::END_STREAM)]);
}

#[test]
fn h2_settings_shrink_window() {
    // SETTINGS zmensi pocatecni okno pod uz poslana data: okno je zaporne
    // a klient ceka (RFC 9113 6.9.2), server odpovi drive = chyba.
    let mut settings = Vec::new();
    settings.extend_from_slice(&h2::settings::INITIAL_WINDOW_SIZE.to_be_bytes());
    settings.extend_from_slice(&10u32.to_be_bytes());
    let mut input = fr(frame::SETTINGS, 0, 0, &settings);
    input.extend(fr(frame::HEADERS, flags::END_HEADERS | flags::END_STREAM, 1, &status_block("413", &[])));
    let req = Request {
        body: vec![b'x'; 70_000],
        ..Request::new(Method::Post, Url::parse("http://h/").unwrap())
    };
    assert!(crate::h2::send(Script::new(&input), &req, &Limits::default()).is_err());
}

#[test]
fn h2_body_limit() {
    let limits = Limits { max_body: 3, ..Limits::default() };
    let script = Script::new(&h2_script(&[
        fr(frame::HEADERS, flags::END_HEADERS, 1, &status_block("200", &[])),
        fr(frame::DATA, flags::END_STREAM, 1, b"ahoj"),
    ]));
    assert_eq!(crate::h2::send(script, &get("http://h/"), &limits), Err(Error::TooLarge("telo odpovedi")));
}
