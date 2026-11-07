//! Domenova jmena (RFC 1035 3.1, kanonicke poradi RFC 4034 6.1).
//!
//! Jmeno je seznam navesti (bez korene). Porovnani nezavisi na velikosti
//! pismen ASCII; kanonicky tvar (podpisy DNSSEC) ma mala pismena.

use alloc::string::String;
use alloc::vec::Vec;
use core::cmp::Ordering;

use crate::{Error, Result};

/// Nejdelsi jmeno na drate (vcetne delek navesti a korene).
pub const MAX_WIRE: usize = 255;

#[derive(Debug, Clone, Default, Eq)]
pub struct Name {
    /// Navesti od nejlevejsiho (`www`, `example`, `com`).
    pub labels: Vec<Vec<u8>>,
}

impl PartialEq for Name {
    fn eq(&self, other: &Self) -> bool {
        self.labels.len() == other.labels.len()
            && self.labels.iter().zip(&other.labels).all(|(a, b)| a.eq_ignore_ascii_case(b))
    }
}

impl Name {
    pub fn root() -> Self {
        Self::default()
    }

    /// Z textu (`www.example.com`, `example.com.`, `.` = koren).
    pub fn parse(s: &str) -> Result<Self> {
        let s = s.strip_suffix('.').unwrap_or(s);
        if s.is_empty() {
            return Ok(Self::root());
        }
        let mut labels = Vec::new();
        for l in s.split('.') {
            if l.is_empty() || l.len() > 63 {
                return Err(Error::Malformed("neplatne navesti"));
            }
            labels.push(l.as_bytes().to_vec());
        }
        let n = Self { labels };
        if n.wire_len() > MAX_WIRE {
            return Err(Error::Malformed("prilis dlouhe jmeno"));
        }
        Ok(n)
    }

    pub fn is_root(&self) -> bool {
        self.labels.is_empty()
    }

    pub fn wire_len(&self) -> usize {
        self.labels.iter().map(|l| l.len() + 1).sum::<usize>() + 1
    }

    /// Bez komprese; `canonical` = mala pismena.
    pub fn write(&self, out: &mut Vec<u8>, canonical: bool) {
        for l in &self.labels {
            out.push(l.len() as u8);
            if canonical {
                out.extend(l.iter().map(u8::to_ascii_lowercase));
            } else {
                out.extend_from_slice(l);
            }
        }
        out.push(0);
    }

    pub fn to_wire(&self, canonical: bool) -> Vec<u8> {
        let mut v = Vec::with_capacity(self.wire_len());
        self.write(&mut v, canonical);
        v
    }

    /// Rodic (bez nejlevejsiho navesti); koren nema rodice.
    pub fn parent(&self) -> Option<Self> {
        (!self.is_root()).then(|| Self {
            labels: self.labels[1..].to_vec(),
        })
    }

    /// Je `self` rovno `zone` nebo v ni?
    pub fn is_in(&self, zone: &Name) -> bool {
        self.labels.len() >= zone.labels.len()
            && self.labels[self.labels.len() - zone.labels.len()..]
                .iter()
                .zip(&zone.labels)
                .all(|(a, b)| a.eq_ignore_ascii_case(b))
    }

    /// Poslednich `n` navesti (predek na urovni `n`).
    pub fn suffix(&self, n: usize) -> Self {
        Self {
            labels: self.labels[self.labels.len() - n.min(self.labels.len())..].to_vec(),
        }
    }

    /// `*.<self>`.
    pub fn wildcard_child(&self) -> Self {
        let mut labels = Vec::from([b"*".to_vec()]);
        labels.extend(self.labels.iter().cloned());
        Self { labels }
    }

    pub fn is_wildcard(&self) -> bool {
        self.labels.first().is_some_and(|l| l == b"*")
    }

    pub fn to_text(&self) -> String {
        if self.is_root() {
            return String::from(".");
        }
        let mut s = String::new();
        for (i, l) in self.labels.iter().enumerate() {
            if i > 0 {
                s.push('.');
            }
            s.push_str(&String::from_utf8_lossy(l));
        }
        s
    }

    /// Kanonicke poradi (RFC 4034 6.1): po navestich zprava, kazde jako
    /// retezec bajtu malymi pismeny.
    pub fn canonical_cmp(&self, other: &Self) -> Ordering {
        let a = self.labels.iter().rev();
        let b = other.labels.iter().rev();
        for (x, y) in a.zip(b) {
            let x: Vec<u8> = x.iter().map(u8::to_ascii_lowercase).collect();
            let y: Vec<u8> = y.iter().map(u8::to_ascii_lowercase).collect();
            match x.cmp(&y) {
                Ordering::Equal => {}
                o => return o,
            }
        }
        self.labels.len().cmp(&other.labels.len())
    }
}

/// Precte jmeno na pozici `pos` zpravy `msg` (s kompresi). Vraci jmeno
/// a pozici za nim (za prvnim ukazatelem, je-li).
pub fn read(msg: &[u8], mut pos: usize) -> Result<(Name, usize)> {
    let mut labels = Vec::new();
    let mut end = None;
    let mut jumps = 0;
    let mut len = 1;
    loop {
        let b = *msg.get(pos).ok_or(Error::Malformed("jmeno za koncem"))?;
        match b & 0xC0 {
            0x00 => {
                if b == 0 {
                    break;
                }
                let l = b as usize;
                let label = msg.get(pos + 1..pos + 1 + l).ok_or(Error::Malformed("navesti za koncem"))?;
                len += l + 1;
                if len > MAX_WIRE {
                    return Err(Error::Malformed("prilis dlouhe jmeno"));
                }
                labels.push(label.to_vec());
                pos += 1 + l;
            }
            0xC0 => {
                let target = ((b as usize & 0x3F) << 8) | *msg.get(pos + 1).ok_or(Error::Malformed("ukazatel"))? as usize;
                if end.is_none() {
                    end = Some(pos + 2);
                }
                jumps += 1;
                // Ukazatele jen dozadu a omezene (smycky).
                if target >= pos || jumps > 64 {
                    return Err(Error::Malformed("neplatny ukazatel komprese"));
                }
                pos = target;
            }
            _ => return Err(Error::Malformed("neznamy typ navesti")),
        }
    }
    Ok((Name { labels }, end.unwrap_or(pos + 1)))
}
