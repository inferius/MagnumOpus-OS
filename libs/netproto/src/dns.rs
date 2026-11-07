//! Format zprav DNS (RFC 1035): dotaz, odpoved, kontrola jmen.
//!
//! Pouziva ho resolver ve sluzbe `netd` a pomocny server DNS v testech
//! (`xtask`). Jen zaznamy A a AAAA; CNAME v odpovedi se preskoci (adresy
//! ciloveho jmena jsou ve stejne odpovedi). Zkracena odpoved (bit TC) se
//! bere tak, jak je - dotaz pres TCP zatim neni.

use alloc::string::String;
use alloc::vec::Vec;
use core::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use abi::{Error, Result};

/// Port serveru DNS.
pub const PORT: u16 = 53;
/// Typy zaznamu.
pub const TYPE_A: u16 = 1;
pub const TYPE_CNAME: u16 = 5;
pub const TYPE_AAAA: u16 = 28;
const CLASS_IN: u16 = 1;
/// Nejdelsi zprava pres UDP bez EDNS.
pub const MAX_UDP: usize = 512;
/// Kody odpovedi (RCODE).
pub const RCODE_OK: u8 = 0;
pub const RCODE_SERVFAIL: u8 = 2;
pub const RCODE_NXDOMAIN: u8 = 3;

/// Je `name` platne jmeno hostitele? Navesti 1..=63 znaku z `a-z`, `A-Z`,
/// `0-9`, `-`, `_`, cele jmeno nejvys 253 znaku, tecka na konci smi byt.
pub fn valid_name(name: &str) -> bool {
    let name = name.strip_suffix('.').unwrap_or(name);
    !name.is_empty()
        && name.len() <= 253
        && name.split('.').all(|l| {
            !l.is_empty()
                && l.len() <= 63
                && !l.starts_with('-')
                && !l.ends_with('-')
                && l.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        })
}

fn put_name(out: &mut Vec<u8>, name: &str) {
    for label in name.trim_end_matches('.').split('.') {
        out.push(label.len() as u8);
        out.extend_from_slice(label.as_bytes());
    }
    out.push(0);
}

/// Dotaz na jmeno `name` typu `qtype` s identifikatorem `id` (rekurze
/// vyzadana).
pub fn build_query(id: u16, name: &str, qtype: u16) -> Result<Vec<u8>> {
    if !valid_name(name) {
        return Err(Error::InvalidArgs);
    }
    let mut q = Vec::with_capacity(18 + name.len());
    q.extend_from_slice(&id.to_be_bytes());
    q.extend_from_slice(&0x0100u16.to_be_bytes()); // RD
    q.extend_from_slice(&1u16.to_be_bytes()); // QDCOUNT
    q.extend_from_slice(&[0; 6]); // AN, NS, AR
    put_name(&mut q, name);
    q.extend_from_slice(&qtype.to_be_bytes());
    q.extend_from_slice(&CLASS_IN.to_be_bytes());
    Ok(q)
}

fn be16(d: &[u8], at: usize) -> Result<u16> {
    d.get(at..at + 2)
        .map(|b| u16::from_be_bytes([b[0], b[1]]))
        .ok_or(Error::Corrupted)
}

fn be32(d: &[u8], at: usize) -> Result<u32> {
    d.get(at..at + 4)
        .map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
        .ok_or(Error::Corrupted)
}

/// Precte jmeno od `at` (s kompresi ukazateli). Vraci jmeno a pozici za
/// jmenem v puvodnim miste. Smycky ukazatelu a prilis dlouha jmena =
/// `CORRUPTED`.
fn read_name(d: &[u8], mut at: usize) -> Result<(String, usize)> {
    let mut name = String::new();
    let mut end = None;
    let mut jumps = 0;
    loop {
        let len = *d.get(at).ok_or(Error::Corrupted)? as usize;
        match len & 0xC0 {
            0x00 if len == 0 => {
                return Ok((name, end.unwrap_or(at + 1)));
            }
            0x00 => {
                let label = d.get(at + 1..at + 1 + len).ok_or(Error::Corrupted)?;
                if !name.is_empty() {
                    name.push('.');
                }
                name.push_str(core::str::from_utf8(label).map_err(|_| Error::Corrupted)?);
                if name.len() > 253 {
                    return Err(Error::Corrupted);
                }
                at += 1 + len;
            }
            0xC0 => {
                let ptr = (be16(d, at)? & 0x3FFF) as usize;
                if end.is_none() {
                    end = Some(at + 2);
                }
                jumps += 1;
                if jumps > 32 {
                    return Err(Error::Corrupted);
                }
                at = ptr;
            }
            _ => return Err(Error::Corrupted),
        }
    }
}

/// Vysledek dotazu.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Answer {
    pub addrs: Vec<IpAddr>,
    /// Nejmensi TTL z odpovedi (s).
    pub ttl: u32,
}

/// Rozebere odpoved na dotaz s identifikatorem `id`. Jina zprava (jine ID,
/// neni odpoved) = `INVALID_ARGS` (resolver ji ignoruje a ceka dal).
/// NXDOMAIN = `NOT_FOUND`, jina chyba serveru = `IO_ERROR`, odpoved bez
/// adres = prazdny seznam.
pub fn parse_response(id: u16, d: &[u8]) -> Result<Answer> {
    if d.len() < 12 {
        return Err(Error::InvalidArgs);
    }
    let flags = be16(d, 2)?;
    if be16(d, 0)? != id || flags & 0x8000 == 0 {
        return Err(Error::InvalidArgs);
    }
    match (flags & 0xF) as u8 {
        RCODE_OK => {}
        RCODE_NXDOMAIN => return Err(Error::NotFound),
        _ => return Err(Error::IoError),
    }
    let qd = be16(d, 4)?;
    let an = be16(d, 6)?;
    let mut at = 12;
    for _ in 0..qd {
        let (_, next) = read_name(d, at)?;
        at = next + 4;
    }
    let mut out = Answer {
        addrs: Vec::new(),
        ttl: u32::MAX,
    };
    for _ in 0..an {
        let (_, next) = read_name(d, at)?;
        let rtype = be16(d, next)?;
        let class = be16(d, next + 2)?;
        let ttl = be32(d, next + 4)?;
        let rdlen = be16(d, next + 8)? as usize;
        let rdata = d
            .get(next + 10..next + 10 + rdlen)
            .ok_or(Error::Corrupted)?;
        at = next + 10 + rdlen;
        if class != CLASS_IN {
            continue;
        }
        let addr = match (rtype, rdlen) {
            (TYPE_A, 4) => IpAddr::V4(Ipv4Addr::new(rdata[0], rdata[1], rdata[2], rdata[3])),
            (TYPE_AAAA, 16) => {
                let o: [u8; 16] = rdata.try_into().unwrap();
                IpAddr::V6(Ipv6Addr::from(o))
            }
            _ => continue,
        };
        out.ttl = out.ttl.min(ttl);
        if !out.addrs.contains(&addr) {
            out.addrs.push(addr);
        }
    }
    if out.addrs.is_empty() {
        out.ttl = 0;
    }
    Ok(out)
}

/// Rozebrany dotaz (pro server).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Query {
    pub id: u16,
    pub name: String,
    pub qtype: u16,
}

/// Rozebere dotaz s jednou otazkou.
pub fn parse_query(d: &[u8]) -> Result<Query> {
    if d.len() < 12 || be16(d, 2)? & 0x8000 != 0 || be16(d, 4)? != 1 {
        return Err(Error::InvalidArgs);
    }
    let (name, next) = read_name(d, 12)?;
    Ok(Query {
        id: be16(d, 0)?,
        name,
        qtype: be16(d, next)?,
    })
}

/// Odpoved serveru na dotaz `q`: zaznamy `addrs` (jen ty, ktere odpovidaji
/// typu dotazu) s TTL `ttl`, nebo chyba `rcode`.
pub fn build_response(q: &Query, addrs: &[IpAddr], ttl: u32, rcode: u8) -> Vec<u8> {
    let records: Vec<&IpAddr> = addrs
        .iter()
        .filter(|a| match a {
            IpAddr::V4(_) => q.qtype == TYPE_A,
            IpAddr::V6(_) => q.qtype == TYPE_AAAA,
        })
        .collect();
    let mut r = Vec::new();
    r.extend_from_slice(&q.id.to_be_bytes());
    // QR, RD, RA + rcode
    r.extend_from_slice(&(0x8180u16 | (rcode as u16 & 0xF)).to_be_bytes());
    r.extend_from_slice(&1u16.to_be_bytes());
    let an = if rcode == RCODE_OK { records.len() } else { 0 };
    r.extend_from_slice(&(an as u16).to_be_bytes());
    r.extend_from_slice(&[0; 4]);
    put_name(&mut r, &q.name);
    r.extend_from_slice(&q.qtype.to_be_bytes());
    r.extend_from_slice(&CLASS_IN.to_be_bytes());
    for a in records.iter().take(an) {
        r.extend_from_slice(&0xC00Cu16.to_be_bytes()); // jmeno = otazka
        match a {
            IpAddr::V4(v4) => {
                r.extend_from_slice(&TYPE_A.to_be_bytes());
                r.extend_from_slice(&CLASS_IN.to_be_bytes());
                r.extend_from_slice(&ttl.to_be_bytes());
                r.extend_from_slice(&4u16.to_be_bytes());
                r.extend_from_slice(&v4.octets());
            }
            IpAddr::V6(v6) => {
                r.extend_from_slice(&TYPE_AAAA.to_be_bytes());
                r.extend_from_slice(&CLASS_IN.to_be_bytes());
                r.extend_from_slice(&ttl.to_be_bytes());
                r.extend_from_slice(&16u16.to_be_bytes());
                r.extend_from_slice(&v6.octets());
            }
        }
    }
    r
}
