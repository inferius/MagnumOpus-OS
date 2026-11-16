//! HTTP/1.1 (RFC 9112): pozadavek, odpoved, telo podle `Content-Length`,
//! `Transfer-Encoding: chunked` nebo do konce spojeni.
//!
//! Prisne cteni hlavicek (ochrana proti "request smuggling" a zmatku mezi
//! proxy a klientem): pokracovaci radky (obs-fold), mezera pred dvojteckou
//! a ruzne hodnoty `Content-Length` = chyba.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use netproto::io::{Read, Write};

use crate::buf::Input;
use crate::sink::{Sink, VecSink};
use crate::{Error, Header, Limits, Method, Request, Response, Result, Version, protocol};

/// Nejvic bajtu tela predanych [`Sink`] naraz.
const CHUNK: usize = 64 * 1024;

/// Zapise pozadavek (hlavicky a telo) do jednoho bufferu.
pub fn encode_request(req: &Request) -> Vec<u8> {
    let mut out = String::new();
    out.push_str(req.method.as_str());
    out.push(' ');
    out.push_str(&req.url.path);
    out.push_str(" HTTP/1.1\r\nHost: ");
    out.push_str(&req.url.authority());
    out.push_str("\r\n");
    let has = |n: &str| req.headers.iter().any(|(h, _)| h == n);
    if !has("user-agent") {
        out.push_str("User-Agent: ");
        out.push_str(crate::USER_AGENT);
        out.push_str("\r\n");
    }
    if !has("accept-encoding") {
        out.push_str("Accept-Encoding: identity\r\n");
    }
    for (name, value) in &req.headers {
        out.push_str(name);
        out.push_str(": ");
        out.push_str(value);
        out.push_str("\r\n");
    }
    // Telo vzdy s delkou (i nulovou u metod s telem: nektere servery bez
    // ni POST odmitnou kodem 411).
    if !req.body.is_empty() || matches!(req.method, Method::Post | Method::Put | Method::Patch) {
        out.push_str(&format!("Content-Length: {}\r\n", req.body.len()));
    }
    out.push_str("Connection: close\r\n\r\n");
    let mut bytes = out.into_bytes();
    bytes.extend_from_slice(&req.body);
    bytes
}

fn parse_status(line: &[u8]) -> Result<u16> {
    let line = core::str::from_utf8(line).map_err(|_| protocol("stavovy radek neni text"))?;
    let rest = line.strip_prefix("HTTP/1.").ok_or_else(|| protocol(format!("neni odpoved HTTP/1.x: {line:.40}")))?;
    let mut parts = rest.splitn(3, ' ');
    let minor = parts.next().unwrap_or("");
    let code = parts.next().unwrap_or("");
    if !(minor == "0" || minor == "1") || code.len() != 3 || !code.bytes().all(|b| b.is_ascii_digit()) {
        return Err(protocol(format!("neplatny stavovy radek: {line:.40}")));
    }
    let code: u16 = code.parse().unwrap();
    if !(100..=599).contains(&code) {
        return Err(protocol("stavovy kod mimo 100-599"));
    }
    Ok(code)
}

/// Precte hlavicky az po prazdny radek (spolecne pro odpoved a trailery).
fn read_headers<S: Read>(input: &mut Input<S>, limits: &Limits, used: &mut usize, out: &mut Vec<Header>) -> Result<()> {
    loop {
        let remaining = limits.max_header_bytes.saturating_sub(*used);
        let line = input.line(remaining)?.ok_or_else(|| protocol("spojeni skoncilo v hlavickach"))?;
        *used += line.len() + 2;
        if *used > limits.max_header_bytes {
            return Err(Error::TooLarge("hlavicky odpovedi"));
        }
        if line.is_empty() {
            return Ok(());
        }
        if line[0] == b' ' || line[0] == b'\t' {
            return Err(protocol("pokracovaci radek hlavicky (obs-fold)"));
        }
        let colon = line.iter().position(|&b| b == b':').ok_or_else(|| protocol("hlavicka bez dvojtecky"))?;
        let name = core::str::from_utf8(&line[..colon]).map_err(|_| protocol("jmeno hlavicky neni ASCII"))?;
        if !crate::is_token(name) {
            return Err(protocol(format!("neplatne jmeno hlavicky {name:.40}")));
        }
        let value = String::from_utf8_lossy(&line[colon + 1..]);
        let value = value.trim_matches([' ', '\t']);
        if value.bytes().any(|b| b == 0) {
            return Err(protocol("NUL v hlavicce"));
        }
        if out.len() >= limits.max_headers {
            return Err(Error::TooLarge("pocet hlavicek"));
        }
        out.push((name.to_ascii_lowercase(), String::from(value)));
    }
}

enum Framing {
    None,
    Length(usize),
    Chunked,
    Close,
}

fn framing(method: Method, status: u16, headers: &[Header], limits: &Limits) -> Result<Framing> {
    if method == Method::Head || status == 204 || status == 304 || (100..200).contains(&status) {
        return Ok(Framing::None);
    }
    let mut te: Option<String> = None;
    let mut length: Option<usize> = None;
    for (name, value) in headers {
        match name.as_str() {
            "transfer-encoding" => {
                let v = te.get_or_insert_with(String::new);
                if !v.is_empty() {
                    v.push(',');
                }
                v.push_str(value);
            }
            "content-length" => {
                for part in value.split(',') {
                    let part = part.trim();
                    if part.is_empty() || !part.bytes().all(|b| b.is_ascii_digit()) {
                        return Err(protocol("neplatny Content-Length"));
                    }
                    let n: usize = part.parse().map_err(|_| Error::TooLarge("telo odpovedi"))?;
                    if length.is_some_and(|l| l != n) {
                        return Err(protocol("ruzne hodnoty Content-Length"));
                    }
                    length = Some(n);
                }
            }
            _ => {}
        }
    }
    if let Some(te) = te {
        // RFC 9112 6.3: Transfer-Encoding ma prednost; chunked musi byt
        // posledni, jinak telo konci se spojenim.
        let last = te.rsplit(',').next().unwrap_or("").trim();
        return Ok(if last.eq_ignore_ascii_case("chunked") { Framing::Chunked } else { Framing::Close });
    }
    // Limit velikosti tela hlida prijemce (`Sink::head`).
    let _ = limits;
    match length {
        Some(n) => Ok(Framing::Length(n)),
        None => Ok(Framing::Close),
    }
}

fn read_exact_body<S: Read>(input: &mut Input<S>, sink: &mut dyn Sink, mut n: usize) -> Result<()> {
    let mut tmp = Vec::new();
    while n > 0 {
        tmp.clear();
        let got = input.read_into(&mut tmp, n.min(CHUNK))?;
        if got == 0 {
            return Err(protocol("spojeni skoncilo pred koncem tela"));
        }
        sink.data(&tmp)?;
        n -= got;
    }
    Ok(())
}

fn read_chunked<S: Read>(input: &mut Input<S>, limits: &Limits, used: &mut usize, sink: &mut dyn Sink) -> Result<()> {
    loop {
        let line = input.line(1024)?.ok_or_else(|| protocol("spojeni skoncilo v chunked"))?;
        let line = core::str::from_utf8(&line).map_err(|_| protocol("neplatna velikost chunku"))?;
        let size = line.split(';').next().unwrap_or("").trim_end_matches([' ', '\t']);
        if size.is_empty() || size.len() > 16 || !size.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(protocol("neplatna velikost chunku"));
        }
        let size = usize::from_str_radix(size, 16).map_err(|_| Error::TooLarge("telo odpovedi"))?;
        if size == 0 {
            // Trailery se prectou (kvuli limitu) a zahodi.
            let mut trailers = Vec::new();
            return read_headers(input, limits, used, &mut trailers);
        }
        read_exact_body(input, sink, size)?;
        match input.line(2)? {
            Some(l) if l.is_empty() => {}
            _ => return Err(protocol("chunk bez CRLF")),
        }
    }
}

/// Posle pozadavek a precte odpoved (cele telo v pameti).
pub fn send<S: Read + Write>(stream: S, req: &Request, limits: &Limits) -> Result<Response> {
    let mut sink = VecSink::new(limits.max_body);
    let mut resp = send_to(stream, req, limits, &mut sink)?;
    resp.body = sink.body;
    Ok(resp)
}

/// Posle pozadavek, telo odpovedi preda `sink` po castech (`Response::body`
/// zustane prazdne).
pub fn send_to<S: Read + Write>(mut stream: S, req: &Request, limits: &Limits, sink: &mut dyn Sink) -> Result<Response> {
    stream.write_all(&encode_request(req)).map_err(Error::Io)?;
    stream.flush().map_err(Error::Io)?;
    let mut input = Input::new(stream);
    let mut used = 0;
    // Mezistavy 1xx (100 Continue, 103 Early Hints) se preskoci.
    let (status, mut headers) = loop {
        let line = input
            .line(limits.max_header_bytes)?
            .ok_or_else(|| protocol("server zavrel spojeni bez odpovedi"))?;
        used += line.len() + 2;
        let status = parse_status(&line)?;
        let mut headers = Vec::new();
        read_headers(&mut input, limits, &mut used, &mut headers)?;
        if status == 101 {
            return Err(protocol("neocekavane 101 Switching Protocols"));
        }
        if (100..200).contains(&status) {
            continue;
        }
        break (status, headers);
    };
    let fr = framing(req.method, status, &headers, limits)?;
    let len = match fr {
        Framing::None => Some(0),
        Framing::Length(n) => Some(n as u64),
        _ => None,
    };
    sink.head(status, &headers, len)?;
    match fr {
        Framing::None => {}
        Framing::Length(n) => read_exact_body(&mut input, sink, n)?,
        Framing::Chunked => read_chunked(&mut input, limits, &mut used, sink)?,
        Framing::Close => {
            let mut tmp = Vec::new();
            loop {
                tmp.clear();
                if input.read_into(&mut tmp, CHUNK)? == 0 {
                    break;
                }
                sink.data(&tmp)?;
            }
        }
    }
    headers.shrink_to_fit();
    Ok(Response {
        status,
        version: Version::Http11,
        headers,
        body: Vec::new(),
        url: None,
    })
}
