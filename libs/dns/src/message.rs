//! Zpravy DNS (RFC 1035 4) s EDNS0 (RFC 6891).
//!
//! RDATA se uklada bez komprese: jmena uvnitr RDATA (NS, CNAME, SOA, PTR,
//! MX, SRV, DNAME) se pri cteni rozbali, aby slo RDATA pouzit
//! v kanonickem tvaru (podpisy DNSSEC) a porovnavat bez zpravy.

use alloc::vec::Vec;

use crate::name::{self, Name};
use crate::{Error, Result, rtype};

pub mod flags {
    pub const QR: u16 = 0x8000;
    pub const AA: u16 = 0x0400;
    pub const TC: u16 = 0x0200;
    pub const RD: u16 = 0x0100;
    pub const RA: u16 = 0x0080;
    /// Authentic Data (resolver overil DNSSEC).
    pub const AD: u16 = 0x0020;
    /// Checking Disabled (vratit i neoverena data, overi klient).
    pub const CD: u16 = 0x0010;
}

pub mod rcode {
    pub const NOERROR: u8 = 0;
    pub const SERVFAIL: u8 = 2;
    pub const NXDOMAIN: u8 = 3;
}

/// Bit DO (DNSSEC OK) v TTL zaznamu OPT.
pub const EDNS_DO: u32 = 0x8000;
pub const CLASS_IN: u16 = 1;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Question {
    pub name: Name,
    pub qtype: u16,
    pub qclass: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Record {
    pub name: Name,
    pub rtype: u16,
    pub class: u16,
    pub ttl: u32,
    /// Bez komprese (viz popis modulu).
    pub rdata: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Message {
    pub id: u16,
    pub flags: u16,
    pub questions: Vec<Question>,
    pub answers: Vec<Record>,
    pub authority: Vec<Record>,
    pub additional: Vec<Record>,
}

fn u16_at(d: &[u8], p: usize) -> Result<u16> {
    Ok(u16::from_be_bytes(d.get(p..p + 2).ok_or(Error::Malformed("zprava za koncem"))?.try_into().unwrap()))
}

fn u32_at(d: &[u8], p: usize) -> Result<u32> {
    Ok(u32::from_be_bytes(d.get(p..p + 4).ok_or(Error::Malformed("zprava za koncem"))?.try_into().unwrap()))
}

/// RDATA bez komprese: pevna cast `prefix` bajtu, pak `names` jmen,
/// zbytek beze zmeny.
fn expand(msg: &[u8], start: usize, end: usize, prefix: usize, names: usize) -> Result<Vec<u8>> {
    let mut out = msg.get(start..start + prefix).ok_or(Error::Malformed("RDATA"))?.to_vec();
    let mut pos = start + prefix;
    for _ in 0..names {
        let (n, next) = name::read(msg, pos)?;
        if next > end {
            return Err(Error::Malformed("jmeno za koncem RDATA"));
        }
        n.write(&mut out, false);
        pos = next;
    }
    out.extend_from_slice(msg.get(pos..end).ok_or(Error::Malformed("RDATA"))?);
    Ok(out)
}

fn read_record(msg: &[u8], pos: usize) -> Result<(Record, usize)> {
    let (name, p) = name::read(msg, pos)?;
    let rtype = u16_at(msg, p)?;
    let class = u16_at(msg, p + 2)?;
    let ttl = u32_at(msg, p + 4)?;
    let len = u16_at(msg, p + 8)? as usize;
    let start = p + 10;
    let end = start + len;
    if end > msg.len() {
        return Err(Error::Malformed("RDATA za koncem zpravy"));
    }
    let rdata = match rtype {
        rtype::NS | rtype::CNAME | rtype::PTR | rtype::DNAME => expand(msg, start, end, 0, 1)?,
        rtype::SOA => expand(msg, start, end, 0, 2)?,
        rtype::MX => expand(msg, start, end, 2, 1)?,
        rtype::SRV => expand(msg, start, end, 6, 1)?,
        _ => msg[start..end].to_vec(),
    };
    Ok((
        Record {
            name,
            rtype,
            class,
            ttl,
            rdata,
        },
        end,
    ))
}

impl Message {
    pub fn parse(msg: &[u8]) -> Result<Self> {
        if msg.len() < 12 {
            return Err(Error::Malformed("kratka zprava"));
        }
        let id = u16_at(msg, 0)?;
        let flags = u16_at(msg, 2)?;
        let counts = [u16_at(msg, 4)?, u16_at(msg, 6)?, u16_at(msg, 8)?, u16_at(msg, 10)?];
        // Kazda polozka ma aspon 5 B: proti obrovskym poctum.
        if counts.iter().map(|&c| c as usize).sum::<usize>() * 5 > msg.len() {
            return Err(Error::Malformed("pocty polozek nesedi s delkou"));
        }
        let mut pos = 12;
        let mut questions = Vec::new();
        for _ in 0..counts[0] {
            let (name, p) = name::read(msg, pos)?;
            questions.push(Question {
                name,
                qtype: u16_at(msg, p)?,
                qclass: u16_at(msg, p + 2)?,
            });
            pos = p + 4;
        }
        let mut sections: [Vec<Record>; 3] = Default::default();
        for (i, section) in sections.iter_mut().enumerate() {
            for _ in 0..counts[i + 1] {
                let (r, p) = read_record(msg, pos)?;
                section.push(r);
                pos = p;
            }
        }
        let [answers, authority, additional] = sections;
        Ok(Self {
            id,
            flags,
            questions,
            answers,
            authority,
            additional,
        })
    }

    pub fn rcode(&self) -> u8 {
        (self.flags & 0x000F) as u8
    }

    /// Bez komprese (pro DoH/DoT a testovaci server).
    pub fn to_wire(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&self.id.to_be_bytes());
        out.extend_from_slice(&self.flags.to_be_bytes());
        for n in [self.questions.len(), self.answers.len(), self.authority.len(), self.additional.len()] {
            out.extend_from_slice(&(n as u16).to_be_bytes());
        }
        for q in &self.questions {
            q.name.write(&mut out, false);
            out.extend_from_slice(&q.qtype.to_be_bytes());
            out.extend_from_slice(&q.qclass.to_be_bytes());
        }
        for r in self.answers.iter().chain(&self.authority).chain(&self.additional) {
            r.write(&mut out, false);
        }
        out
    }

    /// Dotaz s EDNS0 (UDP 1232 B); `dnssec` = bit DO (chci RRSIG a NSEC)
    /// a CD (resolver nema zahazovat neoverena data - overim sam).
    pub fn query(id: u16, name: &Name, qtype: u16, dnssec: bool) -> Self {
        let mut flags = flags::RD;
        if dnssec {
            flags |= flags::CD;
        }
        Self {
            id,
            flags,
            questions: alloc::vec![Question {
                name: name.clone(),
                qtype,
                qclass: CLASS_IN,
            }],
            answers: Vec::new(),
            authority: Vec::new(),
            additional: alloc::vec![Record {
                name: Name::root(),
                rtype: rtype::OPT,
                class: 1232,
                ttl: if dnssec { EDNS_DO } else { 0 },
                rdata: Vec::new(),
            }],
        }
    }
}

impl Record {
    /// Na drat bez komprese; `canonical` = jmeno vlastnika malymi pismeny
    /// (RDATA uz ma byt kanonicke, viz [`Record::canonical_rdata`]).
    pub fn write(&self, out: &mut Vec<u8>, canonical: bool) {
        self.name.write(out, canonical);
        out.extend_from_slice(&self.rtype.to_be_bytes());
        out.extend_from_slice(&self.class.to_be_bytes());
        out.extend_from_slice(&self.ttl.to_be_bytes());
        out.extend_from_slice(&(self.rdata.len() as u16).to_be_bytes());
        out.extend_from_slice(&self.rdata);
    }

    /// RDATA v kanonickem tvaru (RFC 4034 6.2, s opravou RFC 6840 5.1):
    /// jmena v NS, CNAME, SOA, PTR, MX, SRV, DNAME a RRSIG malymi pismeny.
    /// NSEC (dalsi jmeno) se nemeni.
    pub fn canonical_rdata(&self) -> Vec<u8> {
        let lower_names = |prefix: usize, names: usize| -> Vec<u8> {
            let mut out = self.rdata[..prefix.min(self.rdata.len())].to_vec();
            let mut pos = prefix;
            for _ in 0..names {
                let Ok((n, next)) = name::read(&self.rdata, pos) else {
                    return self.rdata.clone();
                };
                n.write(&mut out, true);
                pos = next;
            }
            out.extend_from_slice(self.rdata.get(pos..).unwrap_or(&[]));
            out
        };
        match self.rtype {
            rtype::NS | rtype::CNAME | rtype::PTR | rtype::DNAME => lower_names(0, 1),
            rtype::SOA => lower_names(0, 2),
            rtype::MX => lower_names(2, 1),
            rtype::SRV => lower_names(6, 1),
            rtype::RRSIG => lower_names(18, 1),
            _ => self.rdata.clone(),
        }
    }

    /// Jmeno v RDATA (CNAME, DNAME, NS).
    pub fn target(&self) -> Option<Name> {
        name::read(&self.rdata, 0).ok().map(|(n, _)| n)
    }
}
