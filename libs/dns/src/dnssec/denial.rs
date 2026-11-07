//! Dukazy neexistence: NSEC (RFC 4035 5.4, RFC 6840 4.1) a NSEC3
//! (RFC 5155 8, RFC 9276).
//!
//! Funkce dostavaji jen zaznamy NSEC/NSEC3 s **overenym podpisem** (overuje
//! volajici, [`super::chain`]); tady se jen rozhoduje, co dokazuji.

use alloc::vec::Vec;
use core::cmp::Ordering;

use sha1::{Digest, Sha1};

use crate::message::Record;
use crate::name::{self, Name};
use crate::{Error, Result, rtype};

/// Nad tolik iteraci NSEC3 se zona bere jako nepodepsana (RFC 9276 3.2:
/// vysoke pocty jsou DoS na validator; doporuceni je 0).
pub const MAX_NSEC3_ITERATIONS: u16 = 100;

/// Je typ `t` v bitove mape typu (RFC 4034 4.1.2)?
pub fn type_in_bitmap(mut bitmap: &[u8], t: u16) -> bool {
    let (window, bit) = ((t >> 8) as u8, (t & 0xFF) as usize);
    while bitmap.len() >= 2 {
        let (w, len) = (bitmap[0], bitmap[1] as usize);
        let Some(bits) = bitmap.get(2..2 + len) else {
            return false;
        };
        if w == window {
            return bits.get(bit / 8).is_some_and(|b| b & (0x80 >> (bit % 8)) != 0);
        }
        bitmap = &bitmap[2 + len..];
    }
    false
}

#[derive(Debug, Clone)]
pub struct Nsec {
    pub owner: Name,
    pub next: Name,
    pub bitmap: Vec<u8>,
}

impl Nsec {
    pub fn parse(r: &Record) -> Result<Self> {
        if r.rtype != rtype::NSEC {
            return Err(Error::Malformed("NSEC"));
        }
        let (next, end) = name::read(&r.rdata, 0)?;
        Ok(Self {
            owner: r.name.clone(),
            next,
            bitmap: r.rdata[end..].to_vec(),
        })
    }

    pub fn has(&self, t: u16) -> bool {
        type_in_bitmap(&self.bitmap, t)
    }

    /// Lezi `n` mezi vlastnikem a dalsim jmenem (kanonicky, s prechodem
    /// pres konec zony)?
    pub fn covers(&self, n: &Name) -> bool {
        let after_owner = self.owner.canonical_cmp(n) == Ordering::Less;
        let before_next = n.canonical_cmp(&self.next) == Ordering::Less;
        if self.owner.canonical_cmp(&self.next) == Ordering::Less {
            after_owner && before_next
        } else {
            // Posledni NSEC zony (dalsi = apex).
            after_owner || before_next
        }
    }

    /// Delegace (NS bez SOA): zaznam je z nadrazene zony a nic pod nim
    /// nedokazuje (RFC 6840 4.1).
    fn is_delegation(&self) -> bool {
        self.has(rtype::NS) && !self.has(rtype::SOA)
    }
}

/// Nejdelsi spolecny predek.
fn common(a: &Name, b: &Name) -> usize {
    a.labels
        .iter()
        .rev()
        .zip(b.labels.iter().rev())
        .take_while(|(x, y)| x.eq_ignore_ascii_case(y))
        .count()
}

/// NSEC, ktery smi dokazovat neco o `qname` (ne delegace nad nim, ne DNAME).
fn usable_for(n: &Nsec, qname: &Name) -> bool {
    !(qname.is_in(&n.owner) && n.owner != *qname && (n.is_delegation() || n.has(rtype::DNAME)))
}

#[derive(Debug, Clone)]
pub struct Nsec3 {
    pub hash_alg: u8,
    pub flags: u8,
    pub iterations: u16,
    pub salt: Vec<u8>,
    pub owner_hash: Vec<u8>,
    pub next_hash: Vec<u8>,
    pub bitmap: Vec<u8>,
    /// Zona (vlastnik bez prvniho navesti).
    pub zone: Name,
}

pub const NSEC3_OPT_OUT: u8 = 1;

fn base32hex(s: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    let (mut acc, mut bits) = (0u32, 0);
    for &c in s {
        let v = match c.to_ascii_uppercase() {
            b'0'..=b'9' => c - b'0',
            b'A'..=b'V' => c.to_ascii_uppercase() - b'A' + 10,
            _ => return None,
        };
        acc = acc << 5 | v as u32;
        bits += 5;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    Some(out)
}

impl Nsec3 {
    pub fn parse(r: &Record) -> Result<Self> {
        let d = &r.rdata;
        let bad = Error::Malformed("NSEC3");
        if r.rtype != rtype::NSEC3 || d.len() < 5 {
            return Err(bad);
        }
        let salt_len = d[4] as usize;
        let salt = d.get(5..5 + salt_len).ok_or(bad)?.to_vec();
        let hl = *d.get(5 + salt_len).ok_or(bad)? as usize;
        let next_hash = d.get(6 + salt_len..6 + salt_len + hl).ok_or(bad)?.to_vec();
        let bitmap = d[6 + salt_len + hl..].to_vec();
        let owner_hash = base32hex(r.name.labels.first().ok_or(bad)?).ok_or(bad)?;
        Ok(Self {
            hash_alg: d[0],
            flags: d[1],
            iterations: u16::from_be_bytes([d[2], d[3]]),
            salt,
            owner_hash,
            next_hash,
            bitmap,
            zone: r.name.parent().ok_or(bad)?,
        })
    }

    pub fn has(&self, t: u16) -> bool {
        type_in_bitmap(&self.bitmap, t)
    }

    pub fn opt_out(&self) -> bool {
        self.flags & NSEC3_OPT_OUT != 0
    }

    /// Hash jmena se solí a iteracemi tohoto zaznamu (RFC 5155 5).
    pub fn hash(&self, n: &Name) -> Vec<u8> {
        let mut h = Sha1::new();
        h.update(n.to_wire(true));
        h.update(&self.salt);
        let mut x = h.finalize().to_vec();
        for _ in 0..self.iterations {
            let mut h = Sha1::new();
            h.update(&x);
            h.update(&self.salt);
            x = h.finalize().to_vec();
        }
        x
    }

    fn matches(&self, h: &[u8]) -> bool {
        self.owner_hash == h
    }

    fn covers(&self, h: &[u8]) -> bool {
        if self.owner_hash.as_slice() < self.next_hash.as_slice() {
            self.owner_hash.as_slice() < h && h < self.next_hash.as_slice()
        } else {
            h > self.owner_hash.as_slice() || h < self.next_hash.as_slice()
        }
    }
}

/// Overene zaznamy neexistence z autoritativni sekce.
#[derive(Debug, Clone, Default)]
pub struct Proofs {
    pub nsec: Vec<Nsec>,
    pub nsec3: Vec<Nsec3>,
}

/// Nejblizsi predek (RFC 5155 8.3): `(predek, dalsi blizsi jmeno)`, kdyz
/// NSEC3 dokazuji predka a pokryvaji dalsi blizsi jmeno.
fn closest_encloser<'a>(nsec3: &'a [Nsec3], qname: &Name) -> Option<(Name, Name, &'a Nsec3)> {
    let zone = &nsec3.first()?.zone;
    let mut next_closer = qname.clone();
    let mut candidate = qname.parent()?;
    loop {
        if !candidate.is_in(zone) {
            return None;
        }
        if nsec3.iter().any(|n| n.matches(&n.hash(&candidate))) {
            let cover = nsec3.iter().find(|n| n.covers(&n.hash(&next_closer)))?;
            return Some((candidate, next_closer, cover));
        }
        next_closer = candidate.clone();
        candidate = candidate.parent()?;
    }
}

/// Vysledek dukazu pro dotaz DS (hledani hranic zon).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DsAbsence {
    /// Jmeno existuje a neni hranici zony (pokracovat nize se stejnymi klici).
    NotDelegation,
    /// Nepodepsana delegace (NS bez DS, nebo NSEC3 opt-out): zona pod ni
    /// je nepodepsana.
    Insecure,
    /// Jmeno neexistuje.
    NxDomain,
}

impl Proofs {
    pub fn is_empty(&self) -> bool {
        self.nsec.is_empty() && self.nsec3.is_empty()
    }

    /// NSEC3 s prilis mnoha iteracemi nebo neznamym hashem: zona se bere
    /// jako nepodepsana.
    pub fn unusable_nsec3(&self) -> bool {
        self.nsec3.iter().any(|n| n.iterations > MAX_NSEC3_ITERATIONS || n.hash_alg != 1)
    }

    /// NODATA: jmeno existuje, typ `qtype` (ani CNAME) ne.
    pub fn nodata(&self, qname: &Name, qtype: u16) -> bool {
        let lacks = |has: &dyn Fn(u16) -> bool| !has(qtype) && !has(rtype::CNAME);
        if self.nsec.iter().any(|n| n.owner == *qname && lacks(&|t| n.has(t)) && (qtype == rtype::DS || !n.is_delegation())) {
            return true;
        }
        // Wildcard NODATA: jmeno pokryto a *.predek existuje bez typu.
        if let Some(cov) = self.nsec.iter().find(|n| n.covers(qname) && usable_for(n, qname)) {
            let ce = qname.suffix(common(qname, &cov.owner).max(common(qname, &cov.next)));
            let wc = ce.wildcard_child();
            if self.nsec.iter().any(|n| n.owner == wc && lacks(&|t| n.has(t))) {
                return true;
            }
        }
        if self.nsec3.iter().any(|n| n.matches(&n.hash(qname)) && lacks(&|t| n.has(t))) {
            return true;
        }
        // NSEC3 wildcard NODATA (RFC 5155 8.7).
        if let Some((ce, _, _)) = closest_encloser(&self.nsec3, qname) {
            let wc = ce.wildcard_child();
            if self.nsec3.iter().any(|n| n.matches(&n.hash(&wc)) && lacks(&|t| n.has(t))) {
                return true;
            }
        }
        false
    }

    /// NXDOMAIN: jmeno ani odpovidajici wildcard neexistuji.
    pub fn nxdomain(&self, qname: &Name) -> bool {
        if let Some(cov) = self.nsec.iter().find(|n| n.covers(qname) && usable_for(n, qname)) {
            let ce = qname.suffix(common(qname, &cov.owner).max(common(qname, &cov.next)));
            let wc = ce.wildcard_child();
            return self.nsec.iter().any(|n| n.covers(&wc) && usable_for(n, &wc));
        }
        if let Some((ce, _, _)) = closest_encloser(&self.nsec3, qname) {
            let wc = ce.wildcard_child();
            return self.nsec3.iter().any(|n| n.covers(&n.hash(&wc)));
        }
        false
    }

    /// Kladna odpoved z wildcardu `*.ce`: musi byt dokazano, ze presne
    /// jmeno neexistuje (RFC 4035 5.3.4, RFC 5155 8.8).
    pub fn wildcard_answer(&self, qname: &Name, wildcard: &Name) -> bool {
        let Some(ce) = wildcard.parent() else {
            return false;
        };
        if self.nsec.iter().any(|n| n.covers(qname) && usable_for(n, qname)) {
            return true;
        }
        // Dalsi blizsi jmeno: o navesti delsi nez predek, smerem ke qname.
        if qname.labels.len() <= ce.labels.len() {
            return false;
        }
        let next_closer = qname.suffix(ce.labels.len() + 1);
        self.nsec3.iter().any(|n| n.covers(&n.hash(&next_closer)))
    }

    /// Dukaz pro dotaz DS na `name` (bez DS v odpovedi).
    pub fn ds_absence(&self, name: &Name) -> Option<DsAbsence> {
        if let Some(n) = self.nsec.iter().find(|n| n.owner == *name) {
            if n.has(rtype::DS) || n.has(rtype::CNAME) {
                return None;
            }
            return Some(if n.has(rtype::NS) && !n.has(rtype::SOA) {
                DsAbsence::Insecure
            } else {
                DsAbsence::NotDelegation
            });
        }
        if self.nxdomain(name) {
            return Some(DsAbsence::NxDomain);
        }
        if let Some(n) = self.nsec3.iter().find(|n| n.matches(&n.hash(name))) {
            if n.has(rtype::DS) || n.has(rtype::CNAME) {
                return None;
            }
            return Some(if n.has(rtype::NS) && !n.has(rtype::SOA) {
                DsAbsence::Insecure
            } else {
                DsAbsence::NotDelegation
            });
        }
        // Jmeno vytvorene wildcardem bez DS (wildcard NODATA): neni hranice.
        if self.nodata(name, rtype::DS) {
            return Some(DsAbsence::NotDelegation);
        }
        // Opt-out (RFC 5155 8.6): dalsi blizsi jmeno pokryto opt-out NSEC3.
        if let Some((_, _, cover)) = closest_encloser(&self.nsec3, name)
            && cover.opt_out()
        {
            return Some(DsAbsence::Insecure);
        }
        None
    }
}
