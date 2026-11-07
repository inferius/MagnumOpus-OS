//! NTS - Network Time Security (RFC 8915): overene NTP.
//!
//! 1. **NTS-KE** (TLS 1.3, port 4460, ALPN `ntske/1`): klient posle
//!    [`ke_request`], server odpovi zaznamy ([`parse_ke_reply`]) se sadou
//!    cookies a pripadne jinym serverem/portem pro NTP. Klice NTP (C2S, S2C)
//!    se spocitaji z TLS exportem ([`EXPORTER_LABEL`], [`key_context`]).
//! 2. **NTP s rozsirenimi** (UDP 123): pozadavek nese jednorazovy
//!    identifikator, jednu cookie, zastupce pro dalsi cookies a autentizator
//!    AES-SIV-CMAC-256 pres hlavicku a predchozi rozsireni ([`build_request`]).
//!    Odpoved se overi klicem S2C a v sifrovane casti nese nove cookies
//!    ([`parse_reply`]).
//!
//! Cookies jsou pro klienta neprusvitne (server v nich ma zasifrovane
//! klice); kazda se pouzije jen jednou (soukromi: pozadavky nejdou spojit).
//! Serverova strana ([`server`]) slouzi testum a pomocnemu serveru v xtask.

use alloc::string::String;
use alloc::vec::Vec;

use aes_siv::Aes128SivAead;
use aes_siv::aead::{Aead, KeyInit, Payload};

use crate::ntp;
use abi::time::WallTime;

pub const KE_PORT: u16 = 4460;
pub const ALPN: &[u8] = b"ntske/1";
pub const EXPORTER_LABEL: &[u8] = b"EXPORTER-network-time-security";
/// AEAD_AES_SIV_CMAC_256 (IANA AEAD id 15): klic 32 B, nonce 16 B, tag 16 B.
pub const AEAD_AES_SIV_CMAC_256: u16 = 15;
pub const PROTOCOL_NTPV4: u16 = 0;
pub const KEY_LEN: usize = 32;
/// Kolik cookies klient drzi (RFC 8915 doporucuje 8).
pub const COOKIES: usize = 8;
/// Nejvic bajtu odpovedi NTS-KE (ochrana pameti).
pub const MAX_KE_REPLY: usize = 64 * 1024;

/// Typy zaznamu NTS-KE (horni bit = kriticky).
pub mod record {
    pub const END: u16 = 0;
    pub const NEXT_PROTOCOL: u16 = 1;
    pub const ERROR: u16 = 2;
    pub const WARNING: u16 = 3;
    pub const AEAD: u16 = 4;
    pub const COOKIE: u16 = 5;
    pub const SERVER: u16 = 6;
    pub const PORT: u16 = 7;
    pub const CRITICAL: u16 = 0x8000;
}

/// Typy rozsireni NTP (RFC 8915 5.7).
pub mod ef {
    pub const UNIQUE_ID: u16 = 0x0104;
    pub const COOKIE: u16 = 0x0204;
    pub const PLACEHOLDER: u16 = 0x0304;
    pub const AUTH: u16 = 0x0404;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// Server NTS-KE hlasi chybu (kod RFC 8915 4.1.3).
    Server(u16),
    /// Porusena struktura nebo nepodporovana volba.
    Protocol(&'static str),
    /// Odpoved NTP neni nase (jiny identifikator, rezim, cas odeslani).
    NotOurs,
    /// Odpoved NTP neprosla overenim (autentizator, chybi pole).
    Unauthenticated,
    /// Server odmitl cookie (NTSN "kiss of death"): nove NTS-KE.
    CookieRejected,
    /// Server neni synchronizovany.
    Unsynchronized,
}

fn push_record(out: &mut Vec<u8>, kind: u16, body: &[u8]) {
    out.extend_from_slice(&kind.to_be_bytes());
    out.extend_from_slice(&(body.len() as u16).to_be_bytes());
    out.extend_from_slice(body);
}

/// Pozadavek klienta NTS-KE: NTPv4 a AES-SIV-CMAC-256.
pub fn ke_request() -> Vec<u8> {
    let mut out = Vec::new();
    push_record(&mut out, record::CRITICAL | record::NEXT_PROTOCOL, &PROTOCOL_NTPV4.to_be_bytes());
    push_record(&mut out, record::AEAD, &AEAD_AES_SIV_CMAC_256.to_be_bytes());
    push_record(&mut out, record::CRITICAL | record::END, &[]);
    out
}

/// Kontext exportu klice: protokol, AEAD, smer (0 = C2S, 1 = S2C).
pub fn key_context(s2c: bool) -> [u8; 5] {
    let mut c = [0u8; 5];
    c[0..2].copy_from_slice(&PROTOCOL_NTPV4.to_be_bytes());
    c[2..4].copy_from_slice(&AEAD_AES_SIV_CMAC_256.to_be_bytes());
    c[4] = s2c as u8;
    c
}

/// Vysledek NTS-KE.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct KeReply {
    pub cookies: Vec<Vec<u8>>,
    /// Server NTP, kdyz neni stejny jako server NTS-KE.
    pub server: Option<String>,
    /// Port NTP, kdyz neni 123.
    pub port: Option<u16>,
}

/// Rozebere odpoved NTS-KE. `Ok(None)` = jeste neni cela (chybi END).
pub fn parse_ke_reply(data: &[u8]) -> Result<Option<KeReply>, Error> {
    let mut out = KeReply::default();
    let mut protocol_ok = false;
    let mut aead_ok = false;
    let mut d = data;
    loop {
        if d.len() < 4 {
            return Ok(None);
        }
        let kind = u16::from_be_bytes([d[0], d[1]]);
        let len = u16::from_be_bytes([d[2], d[3]]) as usize;
        if d.len() < 4 + len {
            return Ok(None);
        }
        let body = &d[4..4 + len];
        d = &d[4 + len..];
        let critical = kind & record::CRITICAL != 0;
        let u16_body = || (body.len() == 2).then(|| u16::from_be_bytes([body[0], body[1]]));
        match kind & !record::CRITICAL {
            record::END => break,
            record::NEXT_PROTOCOL => {
                protocol_ok = body.chunks(2).any(|c| c == PROTOCOL_NTPV4.to_be_bytes());
                if body.len() != 2 || !protocol_ok {
                    return Err(Error::Protocol("server nenabizi NTPv4"));
                }
            }
            record::ERROR => return Err(Error::Server(u16_body().unwrap_or(u16::MAX))),
            record::WARNING => {}
            record::AEAD => {
                if u16_body() != Some(AEAD_AES_SIV_CMAC_256) {
                    return Err(Error::Protocol("server vybral jiny AEAD"));
                }
                aead_ok = true;
            }
            record::COOKIE => {
                if body.is_empty() || body.len() > 1024 {
                    return Err(Error::Protocol("neplatna cookie"));
                }
                if out.cookies.len() < COOKIES {
                    out.cookies.push(body.to_vec());
                }
            }
            record::SERVER => {
                let s = core::str::from_utf8(body).map_err(|_| Error::Protocol("neplatny server"))?;
                if s.is_empty() || s.len() > 253 {
                    return Err(Error::Protocol("neplatny server"));
                }
                out.server = Some(String::from(s));
            }
            record::PORT => out.port = Some(u16_body().ok_or(Error::Protocol("neplatny port"))?),
            _ if critical => return Err(Error::Protocol("neznamy kriticky zaznam")),
            _ => {}
        }
    }
    if !protocol_ok || !aead_ok {
        return Err(Error::Protocol("chybi protokol nebo AEAD"));
    }
    if out.cookies.is_empty() {
        return Err(Error::Protocol("server neposlal cookie"));
    }
    Ok(Some(out))
}

fn push_ef(out: &mut Vec<u8>, kind: u16, body: &[u8]) {
    let len = (4 + body.len()).div_ceil(4) * 4;
    out.extend_from_slice(&kind.to_be_bytes());
    out.extend_from_slice(&(len as u16).to_be_bytes());
    out.extend_from_slice(body);
    out.resize(out.len() + len - 4 - body.len(), 0);
}

fn pad4(v: &mut Vec<u8>) {
    while v.len() % 4 != 0 {
        v.push(0);
    }
}

/// Autentizator: nonce a sifrovany text (zarovnane na 4 B).
fn push_auth(out: &mut Vec<u8>, key: &[u8; KEY_LEN], nonce: &[u8; 16], plaintext: &[u8]) {
    let cipher = Aes128SivAead::new(key.into());
    let ct = cipher
        .encrypt(nonce.into(), Payload { msg: plaintext, aad: out })
        .expect("AES-SIV sifruje vzdy");
    let mut body = Vec::new();
    body.extend_from_slice(&(nonce.len() as u16).to_be_bytes());
    body.extend_from_slice(&(ct.len() as u16).to_be_bytes());
    body.extend_from_slice(nonce);
    pad4(&mut body);
    body.extend_from_slice(&ct);
    pad4(&mut body);
    push_ef(out, ef::AUTH, &body);
}

/// Pozadavek NTP s NTS: `uid` jednorazovy identifikator (nahoda), `cookie`
/// jedna cookie, `placeholders` = kolik dalsich cookies si rict.
pub fn build_request(t1: WallTime, uid: &[u8; 32], cookie: &[u8], placeholders: usize, c2s: &[u8; KEY_LEN], nonce: &[u8; 16]) -> Vec<u8> {
    let mut out = Vec::from(ntp::build_request(t1));
    push_ef(&mut out, ef::UNIQUE_ID, uid);
    push_ef(&mut out, ef::COOKIE, cookie);
    let zeros = alloc::vec![0u8; cookie.len()];
    for _ in 0..placeholders {
        push_ef(&mut out, ef::PLACEHOLDER, &zeros);
    }
    push_auth(&mut out, c2s, nonce, &[]);
    out
}

/// Rozsireni zpravy: (typ, telo, zacatek v datech).
type Field<'a> = (u16, &'a [u8], usize);

fn fields(data: &[u8], from: usize) -> Result<Vec<Field<'_>>, Error> {
    let mut out = Vec::new();
    let mut pos = from;
    while pos < data.len() {
        let h = data.get(pos..pos + 4).ok_or(Error::Protocol("useknute rozsireni"))?;
        let kind = u16::from_be_bytes([h[0], h[1]]);
        let len = u16::from_be_bytes([h[2], h[3]]) as usize;
        if len < 4 || len % 4 != 0 || pos + len > data.len() {
            return Err(Error::Protocol("neplatna delka rozsireni"));
        }
        out.push((kind, &data[pos + 4..pos + len], pos));
        pos += len;
    }
    Ok(out)
}

/// Rozebere a desifruje autentizator. Vraci otevreny text.
fn open_auth(body: &[u8], aad: &[u8], key: &[u8; KEY_LEN]) -> Result<Vec<u8>, Error> {
    let bad = Error::Unauthenticated;
    let nonce_len = u16::from_be_bytes(body.get(0..2).ok_or(bad.clone())?.try_into().unwrap()) as usize;
    let ct_len = u16::from_be_bytes(body.get(2..4).ok_or(bad.clone())?.try_into().unwrap()) as usize;
    let nonce = body.get(4..4 + nonce_len).ok_or(bad.clone())?;
    let ct_start = 4 + nonce_len.div_ceil(4) * 4;
    let ct = body.get(ct_start..ct_start + ct_len).ok_or(bad.clone())?;
    if nonce_len != 16 {
        return Err(bad);
    }
    let cipher = Aes128SivAead::new(key.into());
    cipher
        .decrypt(nonce.into(), Payload { msg: ct, aad })
        .map_err(|_| Error::Unauthenticated)
}

/// Overena odpoved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reply {
    pub ntp: ntp::Reply,
    pub cookies: Vec<Vec<u8>>,
}

/// Overi odpoved na pozadavek s `t1` a `uid` klicem `s2c`.
pub fn parse_reply(t1: WallTime, uid: &[u8; 32], s2c: &[u8; KEY_LEN], data: &[u8]) -> Result<Reply, Error> {
    if data.len() < ntp::LEN {
        return Err(Error::NotOurs);
    }
    let efs = fields(data, ntp::LEN).map_err(|_| Error::NotOurs)?;
    if !efs.iter().any(|(k, b, _)| *k == ef::UNIQUE_ID && b.get(..32) == Some(uid)) {
        return Err(Error::NotOurs);
    }
    // Kiss of death NTSN (neovereny: stratum 0, reference "NTSN").
    if data[1] == 0 && &data[12..16] == b"NTSN" {
        return Err(Error::CookieRejected);
    }
    let (_, auth, start) = *efs.iter().find(|(k, _, _)| *k == ef::AUTH).ok_or(Error::Unauthenticated)?;
    let plain = open_auth(auth, &data[..start], s2c)?;
    // Identifikator musi byt v overene casti (pred autentizatorem).
    if !efs.iter().any(|(k, b, at)| *k == ef::UNIQUE_ID && *at < start && b.get(..32) == Some(uid)) {
        return Err(Error::Unauthenticated);
    }
    let reply = match ntp::parse_reply(t1, data) {
        Ok(r) => r,
        Err(abi::Error::BadState) => return Err(Error::Unsynchronized),
        Err(_) => return Err(Error::NotOurs),
    };
    let cookies = fields(&plain, 0)
        .map_err(|_| Error::Unauthenticated)?
        .into_iter()
        .filter(|(k, _, _)| *k == ef::COOKIE)
        .map(|(_, b, _)| b.to_vec())
        .collect();
    Ok(Reply { ntp: reply, cookies })
}

/// Serverova strana (testy a pomocny server v xtask).
pub mod server {
    use alloc::vec::Vec;

    use super::*;

    /// Odpoved NTS-KE s `cookies` (a pripadne portem NTP).
    pub fn ke_reply(cookies: &[Vec<u8>], port: Option<u16>) -> Vec<u8> {
        let mut out = Vec::new();
        push_record(&mut out, record::CRITICAL | record::NEXT_PROTOCOL, &PROTOCOL_NTPV4.to_be_bytes());
        push_record(&mut out, record::AEAD, &AEAD_AES_SIV_CMAC_256.to_be_bytes());
        for c in cookies {
            push_record(&mut out, record::COOKIE, c);
        }
        if let Some(p) = port {
            push_record(&mut out, record::PORT, &p.to_be_bytes());
        }
        push_record(&mut out, record::CRITICAL | record::END, &[]);
        out
    }

    /// Je pozadavek NTS-KE cely (konci zaznamem END)?
    pub fn ke_request_complete(data: &[u8]) -> bool {
        let mut d = data;
        while d.len() >= 4 {
            let kind = u16::from_be_bytes([d[0], d[1]]) & !record::CRITICAL;
            let len = u16::from_be_bytes([d[2], d[3]]) as usize;
            if d.len() < 4 + len {
                return false;
            }
            if kind == record::END {
                return true;
            }
            d = &d[4 + len..];
        }
        false
    }

    /// Cookie = nonce (16 B) || AES-SIV(hlavni klic, C2S || S2C): 96 B,
    /// nasobek 4 (v rozsireni NTP bez vyplne).
    pub fn make_cookie(master: &[u8; KEY_LEN], nonce: &[u8; 16], c2s: &[u8; KEY_LEN], s2c: &[u8; KEY_LEN]) -> Vec<u8> {
        let mut keys = Vec::from(&c2s[..]);
        keys.extend_from_slice(s2c);
        let ct = Aes128SivAead::new(master.into())
            .encrypt(nonce.into(), keys.as_slice())
            .expect("AES-SIV sifruje vzdy");
        let mut out = Vec::from(&nonce[..]);
        out.extend_from_slice(&ct);
        out
    }

    pub fn open_cookie(master: &[u8; KEY_LEN], cookie: &[u8]) -> Option<([u8; KEY_LEN], [u8; KEY_LEN])> {
        let (nonce, ct) = cookie.split_at_checked(16)?;
        let keys = Aes128SivAead::new(master.into()).decrypt(nonce.into(), ct).ok()?;
        Some((keys[..32].try_into().ok()?, keys[32..64].try_into().ok()?))
    }

    /// Overeny pozadavek NTS: identifikator, klice z cookie, pocet
    /// zastupcu.
    pub struct Request {
        pub uid: Vec<u8>,
        pub s2c: [u8; KEY_LEN],
        pub placeholders: usize,
    }

    /// Overi pozadavek (cookie a autentizator); `None` = odmitnout.
    pub fn verify_request(master: &[u8; KEY_LEN], data: &[u8]) -> Option<Request> {
        let efs = fields(data, ntp::LEN).ok()?;
        let uid = efs.iter().find(|(k, _, _)| *k == ef::UNIQUE_ID)?.1.to_vec();
        let cookie = efs.iter().find(|(k, _, _)| *k == ef::COOKIE)?.1;
        let (c2s, s2c) = open_cookie(master, cookie)?;
        let (_, auth, start) = *efs.iter().find(|(k, _, _)| *k == ef::AUTH)?;
        let plain = open_auth(auth, &data[..start], &c2s).ok()?;
        if !plain.is_empty() {
            return None;
        }
        let placeholders = efs.iter().filter(|(k, _, _)| *k == ef::PLACEHOLDER).count();
        Some(Request { uid, s2c, placeholders })
    }

    /// Odpoved NTS na overeny pozadavek (`ntp_reply` = 48 B hlavicka).
    pub fn reply(ntp_reply: &[u8; ntp::LEN], req: &Request, cookies: &[Vec<u8>], nonce: &[u8; 16]) -> Vec<u8> {
        let mut out = Vec::from(&ntp_reply[..]);
        push_ef(&mut out, ef::UNIQUE_ID, &req.uid);
        let mut plain = Vec::new();
        for c in cookies {
            push_ef(&mut plain, ef::COOKIE, c);
        }
        push_auth(&mut out, &req.s2c, nonce, &plain);
        out
    }

    /// Kiss of death NTSN (cookie neplatna).
    pub fn nak(request: &[u8], uid: &[u8]) -> Vec<u8> {
        let mut out = alloc::vec![0u8; ntp::LEN];
        out[0] = 0xE4; // leap 3, verze 4, rezim 4 (server)
        out[12..16].copy_from_slice(b"NTSN");
        out[24..32].copy_from_slice(&request[40..48]);
        push_ef(&mut out, ef::UNIQUE_ID, uid);
        out
    }
}
