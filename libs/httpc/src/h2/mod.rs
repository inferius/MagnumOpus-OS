//! HTTP/2 (RFC 9113): jeden pozadavek na spojeni (proud 1).
//!
//! - Uvod spojeni, SETTINGS (push vypnuty, okno 1 MiB, limit hlavicek),
//!   PING, WINDOW_UPDATE, GOAWAY, RST_STREAM.
//! - Rizeni toku v obou smerech: telo pozadavku jen do okna serveru, prijata
//!   data se hned potvrzuji (WINDOW_UPDATE proudu i spojeni).
//! - Kontroly odpovedi podle RFC 9113 8.1-8.3: pseudo-hlavicky, velka
//!   pismena, hlavicky spojeni, `content-length` proti delce dat.
//!
//! Chyba protokolu = GOAWAY s kodem chyby a konec spojeni.

pub mod huffman;
pub mod hpack;

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use netproto::io::{Read, Write};

use crate::buf::Input;
use crate::{Error, Header, Limits, Method, Request, Response, Result, Version, protocol};

pub const PREFACE: &[u8] = b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n";

pub mod frame {
    pub const DATA: u8 = 0;
    pub const HEADERS: u8 = 1;
    pub const PRIORITY: u8 = 2;
    pub const RST_STREAM: u8 = 3;
    pub const SETTINGS: u8 = 4;
    pub const PUSH_PROMISE: u8 = 5;
    pub const PING: u8 = 6;
    pub const GOAWAY: u8 = 7;
    pub const WINDOW_UPDATE: u8 = 8;
    pub const CONTINUATION: u8 = 9;
}

pub mod flags {
    pub const END_STREAM: u8 = 0x1;
    pub const ACK: u8 = 0x1;
    pub const END_HEADERS: u8 = 0x4;
    pub const PADDED: u8 = 0x8;
    pub const PRIORITY: u8 = 0x20;
}

pub mod settings {
    pub const HEADER_TABLE_SIZE: u16 = 1;
    pub const ENABLE_PUSH: u16 = 2;
    pub const MAX_CONCURRENT_STREAMS: u16 = 3;
    pub const INITIAL_WINDOW_SIZE: u16 = 4;
    pub const MAX_FRAME_SIZE: u16 = 5;
    pub const MAX_HEADER_LIST_SIZE: u16 = 6;
}

/// Kody chyb (RFC 9113 7).
pub mod code {
    pub const NO_ERROR: u32 = 0;
    pub const PROTOCOL_ERROR: u32 = 1;
    pub const FLOW_CONTROL_ERROR: u32 = 3;
    pub const FRAME_SIZE_ERROR: u32 = 6;
    pub const REFUSED_STREAM: u32 = 7;
    pub const COMPRESSION_ERROR: u32 = 9;
}

/// Nase nejvetsi velikost ramce (vychozi, nemenime ji).
const MAX_FRAME: usize = 16_384;
/// Nase okno pro prijem (proud i spojeni).
const RECV_WINDOW: u32 = 1 << 20;
/// Velikost dynamicke tabulky HPACK, kterou serveru povolime.
const TABLE_SIZE: usize = 4096;
const MAX_WINDOW: i64 = (1 << 31) - 1;
/// Proud naseho pozadavku.
const STREAM: u32 = 1;

pub fn frame_header(out: &mut Vec<u8>, len: usize, kind: u8, flags: u8, stream: u32) {
    out.extend_from_slice(&(len as u32).to_be_bytes()[1..]);
    out.push(kind);
    out.push(flags);
    out.extend_from_slice(&(stream & 0x7FFF_FFFF).to_be_bytes());
}

struct Frame {
    kind: u8,
    flags: u8,
    stream: u32,
    payload: Vec<u8>,
}

/// Chyba spojeni: kod pro GOAWAY a popis.
struct ConnError(u32, Error);

fn conn_err(code: u32, msg: impl Into<String>) -> ConnError {
    ConnError(code, protocol(msg))
}

impl From<Error> for ConnError {
    fn from(e: Error) -> Self {
        Self(code::PROTOCOL_ERROR, e)
    }
}

struct Conn<S> {
    input: Input<S>,
    /// Okno serveru pro nase data (spojeni a proud).
    send_conn: i64,
    send_stream: i64,
    /// Pocatecni okno proudu podle SETTINGS serveru.
    peer_initial: i64,
    peer_max_frame: usize,
    decoder: hpack::Decoder,
}

/// Odstrani vypln (PADDED): prvni bajt = delka vyplne na konci.
fn unpad(payload: &[u8], flags: u8) -> core::result::Result<&[u8], ConnError> {
    if flags & self::flags::PADDED == 0 {
        return Ok(payload);
    }
    let pad = *payload.first().ok_or_else(|| conn_err(code::PROTOCOL_ERROR, "PADDED bez delky"))? as usize;
    if pad >= payload.len() {
        return Err(conn_err(code::PROTOCOL_ERROR, "vypln delsi nez ramec"));
    }
    Ok(&payload[1..payload.len() - pad])
}

impl<S: Read + Write> Conn<S> {
    fn write(&mut self, data: &[u8]) -> core::result::Result<(), ConnError> {
        let s = &mut self.input.stream;
        s.write_all(data).and_then(|_| s.flush()).map_err(|e| ConnError(code::NO_ERROR, Error::Io(e)))
    }

    fn read_frame(&mut self) -> core::result::Result<Frame, ConnError> {
        self.input.need(9)?;
        let h = self.input.buffered();
        let len = u32::from_be_bytes([0, h[0], h[1], h[2]]) as usize;
        let (kind, flags) = (h[3], h[4]);
        let stream = u32::from_be_bytes([h[5], h[6], h[7], h[8]]) & 0x7FFF_FFFF;
        if len > MAX_FRAME {
            return Err(conn_err(code::FRAME_SIZE_ERROR, format!("ramec {len} B nad limit")));
        }
        self.input.need(9 + len)?;
        let payload = self.input.buffered()[9..9 + len].to_vec();
        self.input.consume(9 + len);
        Ok(Frame { kind, flags, stream, payload })
    }

    fn apply_settings(&mut self, payload: &[u8]) -> core::result::Result<(), ConnError> {
        if payload.len() % 6 != 0 {
            return Err(conn_err(code::FRAME_SIZE_ERROR, "SETTINGS neni nasobek 6"));
        }
        for item in payload.chunks(6) {
            let id = u16::from_be_bytes([item[0], item[1]]);
            let value = u32::from_be_bytes([item[2], item[3], item[4], item[5]]);
            match id {
                settings::INITIAL_WINDOW_SIZE => {
                    if i64::from(value) > MAX_WINDOW {
                        return Err(conn_err(code::FLOW_CONTROL_ERROR, "INITIAL_WINDOW_SIZE nad 2^31-1"));
                    }
                    // Zmena se promitne i do okna otevreneho proudu (6.9.2).
                    self.send_stream += i64::from(value) - self.peer_initial;
                    self.peer_initial = i64::from(value);
                    if self.send_stream > MAX_WINDOW {
                        return Err(conn_err(code::FLOW_CONTROL_ERROR, "okno proudu nad 2^31-1"));
                    }
                }
                settings::MAX_FRAME_SIZE => {
                    if !(16_384..=16_777_215).contains(&value) {
                        return Err(conn_err(code::PROTOCOL_ERROR, "neplatne MAX_FRAME_SIZE"));
                    }
                    self.peer_max_frame = value as usize;
                }
                settings::ENABLE_PUSH if value > 1 => {
                    return Err(conn_err(code::PROTOCOL_ERROR, "neplatne ENABLE_PUSH"));
                }
                // Ostatni (velikost tabulky koderu - nepouzivame ji; pocet
                // proudu - mame jeden; neznama nastaveni) se ignoruji.
                _ => {}
            }
        }
        let mut ack = Vec::new();
        frame_header(&mut ack, 0, frame::SETTINGS, flags::ACK, 0);
        self.write(&ack)
    }

    /// Zpracuje ramec spojeni (stream 0, PING, ...). `true` = zpracovan.
    fn control(&mut self, f: &Frame) -> core::result::Result<bool, ConnError> {
        match f.kind {
            frame::SETTINGS => {
                if f.stream != 0 {
                    return Err(conn_err(code::PROTOCOL_ERROR, "SETTINGS na proudu"));
                }
                if f.flags & flags::ACK != 0 {
                    if !f.payload.is_empty() {
                        return Err(conn_err(code::FRAME_SIZE_ERROR, "SETTINGS ACK s daty"));
                    }
                } else {
                    self.apply_settings(&f.payload)?;
                }
            }
            frame::PING => {
                if f.stream != 0 || f.payload.len() != 8 {
                    return Err(conn_err(code::PROTOCOL_ERROR, "neplatny PING"));
                }
                if f.flags & flags::ACK == 0 {
                    let mut pong = Vec::new();
                    frame_header(&mut pong, 8, frame::PING, flags::ACK, 0);
                    pong.extend_from_slice(&f.payload);
                    self.write(&pong)?;
                }
            }
            frame::WINDOW_UPDATE => {
                if f.payload.len() != 4 {
                    return Err(conn_err(code::FRAME_SIZE_ERROR, "WINDOW_UPDATE neni 4 B"));
                }
                let inc = i64::from(u32::from_be_bytes(f.payload[..4].try_into().unwrap()) & 0x7FFF_FFFF);
                if inc == 0 {
                    return Err(conn_err(code::PROTOCOL_ERROR, "WINDOW_UPDATE o 0"));
                }
                let window = match f.stream {
                    0 => &mut self.send_conn,
                    STREAM => &mut self.send_stream,
                    _ => return Ok(true),
                };
                *window += inc;
                if *window > MAX_WINDOW {
                    return Err(conn_err(code::FLOW_CONTROL_ERROR, "okno nad 2^31-1"));
                }
            }
            frame::GOAWAY => {
                if f.stream != 0 || f.payload.len() < 8 {
                    return Err(conn_err(code::PROTOCOL_ERROR, "neplatny GOAWAY"));
                }
                let last = u32::from_be_bytes(f.payload[..4].try_into().unwrap()) & 0x7FFF_FFFF;
                let err = u32::from_be_bytes(f.payload[4..8].try_into().unwrap());
                if last < STREAM {
                    // Nas pozadavek server nezpracoval (lze zopakovat).
                    return Err(ConnError(code::NO_ERROR, Error::Reset(if err == 0 { code::REFUSED_STREAM } else { err })));
                }
                // Nas proud server jeste dokonci; nic dalsiho nezaciname.
            }
            frame::PRIORITY => {
                if f.payload.len() != 5 {
                    return Err(conn_err(code::FRAME_SIZE_ERROR, "PRIORITY neni 5 B"));
                }
            }
            frame::PUSH_PROMISE => return Err(conn_err(code::PROTOCOL_ERROR, "PUSH_PROMISE pri vypnutem push")),
            frame::DATA | frame::HEADERS | frame::CONTINUATION | frame::RST_STREAM => return Ok(false),
            // Nezname typy ramcu se ignoruji (RFC 9113 4.1).
            _ => {}
        }
        Ok(true)
    }

    /// Ceka, az bude v okne serveru misto pro data.
    fn wait_window(&mut self) -> core::result::Result<(), ConnError> {
        while self.send_conn <= 0 || self.send_stream <= 0 {
            let f = self.read_frame()?;
            if !self.control(&f)? {
                // Odpoved pred koncem tela (napr. 413): RST nebo hlavicky.
                if f.kind == frame::RST_STREAM && f.stream == STREAM {
                    return Err(ConnError(code::NO_ERROR, Error::Reset(rst_code(&f)?)));
                }
                return Err(conn_err(code::PROTOCOL_ERROR, "server odpovida pred koncem tela pozadavku"));
            }
        }
        Ok(())
    }

    fn send_body(&mut self, body: &[u8]) -> core::result::Result<(), ConnError> {
        let mut rest = body;
        while !rest.is_empty() {
            self.wait_window()?;
            let n = rest.len().min(self.peer_max_frame).min(self.send_conn.min(self.send_stream) as usize);
            let last = n == rest.len();
            let mut out = Vec::with_capacity(9 + n);
            frame_header(&mut out, n, frame::DATA, if last { flags::END_STREAM } else { 0 }, STREAM);
            out.extend_from_slice(&rest[..n]);
            self.write(&out)?;
            self.send_conn -= n as i64;
            self.send_stream -= n as i64;
            rest = &rest[n..];
        }
        Ok(())
    }

    fn window_update(&mut self, stream: u32, inc: usize) -> core::result::Result<(), ConnError> {
        let mut out = Vec::new();
        frame_header(&mut out, 4, frame::WINDOW_UPDATE, 0, stream);
        out.extend_from_slice(&(inc as u32).to_be_bytes());
        self.write(&out)
    }

    /// Precte cely blok hlavicek (HEADERS + CONTINUATION).
    fn header_block(&mut self, first: Frame, limits: &Limits) -> core::result::Result<(Vec<u8>, bool), ConnError> {
        let end_stream = first.flags & flags::END_STREAM != 0;
        let mut data = unpad(&first.payload, first.flags)?;
        if first.flags & flags::PRIORITY != 0 {
            data = data.get(5..).ok_or_else(|| conn_err(code::PROTOCOL_ERROR, "HEADERS s PRIORITY bez dat"))?;
        }
        let mut block = data.to_vec();
        let mut end = first.flags & flags::END_HEADERS != 0;
        while !end {
            let f = self.read_frame()?;
            if f.kind != frame::CONTINUATION || f.stream != STREAM {
                return Err(conn_err(code::PROTOCOL_ERROR, "ocekavano CONTINUATION"));
            }
            // Limit uz na zakodovany blok (jinak neomezene CONTINUATION).
            if block.len() + f.payload.len() > limits.max_header_bytes {
                return Err(ConnError(code::PROTOCOL_ERROR, Error::TooLarge("hlavicky odpovedi")));
            }
            block.extend_from_slice(&f.payload);
            end = f.flags & flags::END_HEADERS != 0;
        }
        Ok((block, end_stream))
    }

    fn decode(&mut self, block: &[u8], limits: &Limits) -> core::result::Result<Vec<Header>, ConnError> {
        self.decoder
            .decode(block, limits.max_header_bytes, limits.max_headers)
            .map_err(|e| match e {
                Error::TooLarge(_) => ConnError(code::PROTOCOL_ERROR, e),
                e => ConnError(code::COMPRESSION_ERROR, e),
            })
    }
}

fn rst_code(f: &Frame) -> core::result::Result<u32, ConnError> {
    if f.payload.len() != 4 {
        return Err(conn_err(code::FRAME_SIZE_ERROR, "RST_STREAM neni 4 B"));
    }
    Ok(u32::from_be_bytes(f.payload[..4].try_into().unwrap()))
}

/// Hlavicky spojeni, ktere HTTP/2 zakazuje (RFC 9113 8.2.2).
const CONNECTION_HEADERS: &[&str] = &["connection", "keep-alive", "proxy-connection", "transfer-encoding", "upgrade"];

/// Overi hlavicky odpovedi; vrati stav a obycejne hlavicky.
fn check_response(headers: Vec<Header>, trailers: bool) -> Result<(u16, Vec<Header>)> {
    let mut status = None;
    let mut regular = Vec::with_capacity(headers.len());
    for (name, value) in headers {
        if name.bytes().any(|b| b.is_ascii_uppercase()) || name.is_empty() {
            return Err(protocol("HTTP/2: jmeno hlavicky s velkymi pismeny"));
        }
        if value.bytes().any(|b| b == b'\r' || b == b'\n' || b == 0) {
            return Err(protocol("HTTP/2: hodnota s CR, LF nebo NUL"));
        }
        if let Some(pseudo) = name.strip_prefix(':') {
            if trailers || !regular.is_empty() || pseudo != "status" || status.is_some() {
                return Err(protocol(format!("HTTP/2: neplatna pseudo-hlavicka :{pseudo:.20}")));
            }
            let code = value.parse::<u16>().ok().filter(|c| (100..=599).contains(c) && value.len() == 3);
            status = Some(code.ok_or_else(|| protocol("HTTP/2: neplatny :status"))?);
        } else {
            if CONNECTION_HEADERS.contains(&name.as_str()) || (name == "te" && value != "trailers") {
                return Err(protocol(format!("HTTP/2: hlavicka spojeni {name}")));
            }
            regular.push((name, value));
        }
    }
    if trailers {
        return Ok((0, regular));
    }
    Ok((status.ok_or_else(|| protocol("HTTP/2: chybi :status"))?, regular))
}

fn content_length(headers: &[Header]) -> Result<Option<usize>> {
    let mut len = None;
    for (name, value) in headers.iter().filter(|(n, _)| n == "content-length") {
        let n = value.trim().parse::<usize>().map_err(|_| protocol("HTTP/2: neplatny content-length"))?;
        if len.is_some_and(|l| l != n) {
            return Err(protocol(format!("HTTP/2: ruzne {name}")));
        }
        len = Some(n);
    }
    Ok(len)
}

/// Zakoduje pozadavek: pseudo-hlavicky a hlavicky programu.
fn request_block(req: &Request) -> Vec<u8> {
    let authority = req.url.authority();
    let mut list: Vec<(&str, &str)> = alloc::vec![
        (":method", req.method.as_str()),
        (":scheme", req.url.scheme.as_str()),
        (":authority", &authority),
        (":path", &req.url.path),
    ];
    let has = |n: &str| req.headers.iter().any(|(h, _)| h == n);
    if !has("user-agent") {
        list.push(("user-agent", crate::USER_AGENT));
    }
    if !has("accept-encoding") {
        list.push(("accept-encoding", "identity"));
    }
    for (n, v) in &req.headers {
        list.push((n, v));
    }
    let len = alloc::format!("{}", req.body.len());
    if !req.body.is_empty() || matches!(req.method, Method::Post | Method::Put | Method::Patch) {
        list.push(("content-length", &len));
    }
    hpack::encode(&list)
}

fn run<S: Read + Write>(conn: &mut Conn<S>, req: &Request, limits: &Limits) -> core::result::Result<Response, ConnError> {
    // Uvod: preface, SETTINGS, zvetseni okna spojeni.
    let mut out = Vec::from(PREFACE);
    let ours = [
        (settings::ENABLE_PUSH, 0),
        (settings::INITIAL_WINDOW_SIZE, RECV_WINDOW),
        (settings::MAX_HEADER_LIST_SIZE, limits.max_header_bytes as u32),
        (settings::HEADER_TABLE_SIZE, TABLE_SIZE as u32),
        (settings::MAX_CONCURRENT_STREAMS, 0),
    ];
    frame_header(&mut out, ours.len() * 6, frame::SETTINGS, 0, 0);
    for (id, v) in ours {
        out.extend_from_slice(&id.to_be_bytes());
        out.extend_from_slice(&v.to_be_bytes());
    }
    frame_header(&mut out, 4, frame::WINDOW_UPDATE, 0, 0);
    out.extend_from_slice(&(RECV_WINDOW - 65_535).to_be_bytes());

    // Hlavicky pozadavku (CONTINUATION, kdyz je blok vetsi nez ramec;
    // serverovo MAX_FRAME_SIZE jeste nezname, plati vychozi 16 KiB).
    let block = request_block(req);
    let body_follows = !req.body.is_empty();
    let mut chunks = block.chunks(MAX_FRAME).peekable();
    let mut first = true;
    while let Some(chunk) = chunks.next() {
        let last = chunks.peek().is_none();
        let mut fl = if last { flags::END_HEADERS } else { 0 };
        let kind = if first { frame::HEADERS } else { frame::CONTINUATION };
        if first && !body_follows {
            fl |= flags::END_STREAM;
        }
        frame_header(&mut out, chunk.len(), kind, fl, STREAM);
        out.extend_from_slice(chunk);
        first = false;
    }
    conn.write(&out)?;
    if body_follows {
        conn.send_body(&req.body)?;
    }

    // Odpoved.
    let mut status = None;
    let mut headers = Vec::new();
    let mut body: Vec<u8> = Vec::new();
    loop {
        let f = conn.read_frame()?;
        if conn.control(&f)? {
            continue;
        }
        if f.stream != STREAM {
            return Err(conn_err(code::PROTOCOL_ERROR, format!("ramec {} na proudu {}", f.kind, f.stream)));
        }
        match f.kind {
            frame::RST_STREAM => return Err(ConnError(code::NO_ERROR, Error::Reset(rst_code(&f)?))),
            frame::CONTINUATION => return Err(conn_err(code::PROTOCOL_ERROR, "CONTINUATION bez HEADERS")),
            frame::HEADERS => {
                let (block, end_stream) = conn.header_block(f, limits)?;
                let list = conn.decode(&block, limits)?;
                if status.is_none() {
                    let (code, list) = check_response(list, false)?;
                    if (100..200).contains(&code) {
                        // Informativni odpoved (1xx) se preskoci.
                        if end_stream {
                            return Err(conn_err(code::PROTOCOL_ERROR, "1xx s END_STREAM"));
                        }
                        continue;
                    }
                    status = Some(code);
                    headers = list;
                } else {
                    // Trailery: musi koncit proud; zahodi se.
                    check_response(list, true)?;
                    if !end_stream {
                        return Err(conn_err(code::PROTOCOL_ERROR, "trailery bez END_STREAM"));
                    }
                }
                if end_stream {
                    break;
                }
            }
            frame::DATA => {
                if status.is_none() {
                    return Err(conn_err(code::PROTOCOL_ERROR, "DATA pred hlavickami"));
                }
                let data = unpad(&f.payload, f.flags)?;
                if req.method != Method::Head && body.len() + data.len() > limits.max_body {
                    return Err(ConnError(code::NO_ERROR, Error::TooLarge("telo odpovedi")));
                }
                if req.method != Method::Head {
                    body.extend_from_slice(data);
                }
                let end = f.flags & flags::END_STREAM != 0;
                // Potvrdit cely ramec (i vypln) - spojeni vzdy, proud jen
                // kdyz jeste neskoncil.
                if !f.payload.is_empty() {
                    conn.window_update(0, f.payload.len())?;
                    if !end {
                        conn.window_update(STREAM, f.payload.len())?;
                    }
                }
                if end {
                    break;
                }
            }
            _ => unreachable!("control() zpracuje ostatni typy"),
        }
    }
    let status = status.expect("smycka konci az po hlavickach");
    if let Some(len) = content_length(&headers)?
        && req.method != Method::Head
        && status != 304
        && len != body.len()
    {
        return Err(conn_err(code::PROTOCOL_ERROR, "HTTP/2: content-length neodpovida datum"));
    }
    if req.method == Method::Head {
        // Odpoved na HEAD nema telo; nektere servery data presto poslou
        // (hyper) - zahodi se, jako v prohlizecich.
        body.clear();
    }
    Ok(Response {
        status,
        version: Version::Http2,
        headers,
        body,
        url: None,
    })
}

/// Posle pozadavek po spojeni HTTP/2 (TLS s ALPN `h2` nebo h2c).
pub fn send<S: Read + Write>(stream: S, req: &Request, limits: &Limits) -> Result<Response> {
    let mut conn = Conn {
        input: Input::new(stream),
        send_conn: 65_535,
        send_stream: 65_535,
        peer_initial: 65_535,
        peer_max_frame: MAX_FRAME,
        decoder: hpack::Decoder::new(TABLE_SIZE),
    };
    let result = run(&mut conn, req, limits);
    // Konec spojeni: GOAWAY (NO_ERROR nebo kod chyby protokolu), bez cekani.
    let code = match &result {
        Ok(_) => code::NO_ERROR,
        Err(ConnError(c, _)) => *c,
    };
    if !matches!(&result, Err(ConnError(_, Error::Io(_)))) {
        let mut out = Vec::new();
        frame_header(&mut out, 8, frame::GOAWAY, 0, 0);
        // Posledni proud od serveru: zadny (push je vypnuty).
        out.extend_from_slice(&0u32.to_be_bytes());
        out.extend_from_slice(&code.to_be_bytes());
        let _ = conn.write(&out);
    }
    result.map_err(|ConnError(_, e)| e)
}
