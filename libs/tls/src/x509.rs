//! Minimalni cteni DER (X.509 certifikaty, CRL, OCSP): jen pole, ktera
//! potrebuje kontrola odvolani. Overeni retezu a podpisu certifikatu dela
//! webpki; tady se nic neoveruje, jen cte.
//!
//! Vse je presne podle DER (delka v nejkratsim tvaru se nevynucuje -
//! cteni je tolerantni, rozhodnuti o duvere na tom nezavisi).

use alloc::string::String;
use alloc::vec::Vec;

/// Znacky ASN.1, ktere tu potrebujeme.
pub mod tag {
    pub const BOOLEAN: u8 = 0x01;
    pub const INTEGER: u8 = 0x02;
    pub const BIT_STRING: u8 = 0x03;
    pub const OCTET_STRING: u8 = 0x04;
    pub const NULL: u8 = 0x05;
    pub const OID: u8 = 0x06;
    pub const ENUMERATED: u8 = 0x0A;
    pub const UTC_TIME: u8 = 0x17;
    pub const GENERALIZED_TIME: u8 = 0x18;
    pub const SEQUENCE: u8 = 0x30;
    pub const SET: u8 = 0x31;
    /// `[n]` constructed (EXPLICIT nebo slozeny IMPLICIT).
    pub const fn ctx(n: u8) -> u8 {
        0xA0 | n
    }
    /// `[n]` primitive (IMPLICIT nad jednoduchym typem).
    pub const fn ctx_prim(n: u8) -> u8 {
        0x80 | n
    }
}

/// Chyba cteni (poskozena nebo neocekavana struktura).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Malformed;

pub type Result<T> = core::result::Result<T, Malformed>;

/// Jedna polozka TLV.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Tlv<'a> {
    pub tag: u8,
    /// Obsah (bez znacky a delky).
    pub value: &'a [u8],
    /// Cela polozka vcetne znacky a delky.
    pub raw: &'a [u8],
}

/// Ctecka posloupnosti polozek.
#[derive(Debug, Clone, Copy)]
pub struct Der<'a> {
    data: &'a [u8],
}

impl<'a> Der<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self { data }
    }

    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    /// Dalsi polozka bez kontroly znacky.
    pub fn any(&mut self) -> Result<Tlv<'a>> {
        let d = self.data;
        let tag = *d.first().ok_or(Malformed)?;
        // Vicebajtove znacky (tag 31) X.509 nepouziva.
        if tag & 0x1F == 0x1F {
            return Err(Malformed);
        }
        let first = *d.get(1).ok_or(Malformed)?;
        let (len, hdr) = if first < 0x80 {
            (first as usize, 2)
        } else {
            let n = (first & 0x7F) as usize;
            if n == 0 || n > 4 {
                return Err(Malformed);
            }
            let mut len = 0usize;
            for i in 0..n {
                len = len << 8 | *d.get(2 + i).ok_or(Malformed)? as usize;
            }
            (len, 2 + n)
        };
        let end = hdr.checked_add(len).ok_or(Malformed)?;
        if end > d.len() {
            return Err(Malformed);
        }
        self.data = &d[end..];
        Ok(Tlv {
            tag,
            value: &d[hdr..end],
            raw: &d[..end],
        })
    }

    /// Dalsi polozka se znackou `tag`.
    pub fn expect(&mut self, tag: u8) -> Result<Tlv<'a>> {
        let t = self.any()?;
        if t.tag != tag {
            return Err(Malformed);
        }
        Ok(t)
    }

    /// Obsah dalsi polozky se znackou `tag` jako nova ctecka.
    pub fn nested(&mut self, tag: u8) -> Result<Der<'a>> {
        Ok(Der::new(self.expect(tag)?.value))
    }

    /// Volitelna polozka: precte ji, jen kdyz ma znacku `tag`.
    pub fn optional(&mut self, tag: u8) -> Result<Option<Tlv<'a>>> {
        if self.data.first() == Some(&tag) {
            self.expect(tag).map(Some)
        } else {
            Ok(None)
        }
    }

    pub fn peek_tag(&self) -> Option<u8> {
        self.data.first().copied()
    }
}

/// OID (obsah polozky OID) z textoveho zapisu - pro porovnani.
pub fn oid(dotted: &[u64]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut push = |mut v: u64| {
        let mut tmp = [0u8; 10];
        let mut i = tmp.len();
        loop {
            i -= 1;
            tmp[i] = (v & 0x7F) as u8;
            v >>= 7;
            if v == 0 {
                break;
            }
        }
        for (k, b) in tmp[i..].iter().enumerate() {
            let last = i + k == tmp.len() - 1;
            out.push(if last { *b } else { *b | 0x80 });
        }
    };
    push(dotted[0] * 40 + dotted[1]);
    for &v in &dotted[2..] {
        push(v);
    }
    out
}

/// Cas z UTCTime nebo GeneralizedTime (jen tvar `...Z` podle RFC 5280)
/// v sekundach od 1970.
pub fn time(t: &Tlv<'_>) -> Result<i64> {
    let s = core::str::from_utf8(t.value).map_err(|_| Malformed)?;
    let s = s.strip_suffix('Z').ok_or(Malformed)?;
    let (year, rest) = match t.tag {
        tag::UTC_TIME if s.len() == 12 => {
            let y: i64 = s[..2].parse().map_err(|_| Malformed)?;
            (if y >= 50 { 1900 + y } else { 2000 + y }, &s[2..])
        }
        tag::GENERALIZED_TIME if s.len() == 14 => (s[..4].parse().map_err(|_| Malformed)?, &s[4..]),
        _ => return Err(Malformed),
    };
    if !rest.bytes().all(|b| b.is_ascii_digit()) {
        return Err(Malformed);
    }
    let n = |i: usize| -> i64 { rest[i..i + 2].parse().unwrap_or(0) };
    let (mo, d, h, mi, se) = (n(0), n(2), n(4), n(6), n(8));
    if !(1..=12).contains(&mo) || !(1..=31).contains(&d) || h > 23 || mi > 59 || se > 60 {
        return Err(Malformed);
    }
    Ok(days_from_civil(year, mo, d) * 86_400 + h * 3600 + mi * 60 + se)
}

/// Dny od 1970-01-01 (algoritmus H. Hinnanta).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Klic a jmeno podpisovatele (certifikat nebo korenova CA z uloziste).
#[derive(Debug, Clone, Copy)]
pub struct Key<'a> {
    /// Cela polozka Name.
    pub subject: &'a [u8],
    /// Obsah AlgorithmIdentifier verejneho klice.
    pub spki_alg: &'a [u8],
    /// Bity verejneho klice.
    pub public_key: &'a [u8],
}

/// Rozlozi obsah SubjectPublicKeyInfo (bez vnejsi znacky) na algoritmus
/// a bity klice.
pub fn spki_parts(spki_contents: &[u8]) -> Result<(&[u8], &[u8])> {
    let mut d = Der::new(spki_contents);
    let alg = d.expect(tag::SEQUENCE)?.value;
    let key = bits(&d.expect(tag::BIT_STRING)?)?;
    Ok((alg, key))
}

/// Pole certifikatu pro kontrolu odvolani.
#[derive(Debug, Clone)]
pub struct Cert<'a> {
    /// Cela TBSCertificate (podepsana cast).
    pub tbs: &'a [u8],
    /// Obsah INTEGER cisla certifikatu (jak je v DER, vcetne uvodni nuly).
    pub serial: &'a [u8],
    /// Cela polozka Name vystavitele.
    pub issuer: &'a [u8],
    /// Cela polozka Name subjektu.
    pub subject: &'a [u8],
    /// Cela polozka SubjectPublicKeyInfo.
    pub spki: &'a [u8],
    /// Obsah AlgorithmIdentifier verejneho klice (pro webpki).
    pub spki_alg: &'a [u8],
    /// Bity verejneho klice (obsah BIT STRING bez bajtu nevyuzitych bitu).
    pub public_key: &'a [u8],
    /// Obsah AlgorithmIdentifier podpisu certifikatu.
    pub sig_alg: &'a [u8],
    /// Podpis (obsah BIT STRING bez bajtu nevyuzitych bitu).
    pub signature: &'a [u8],
    pub not_before: i64,
    pub not_after: i64,
    /// Rozsireni: (OID, kriticke, hodnota).
    pub extensions: Vec<(&'a [u8], bool, &'a [u8])>,
}

fn bits<'a>(t: &Tlv<'a>) -> Result<&'a [u8]> {
    match t.value.split_first() {
        Some((0, rest)) => Ok(rest),
        _ => Err(Malformed),
    }
}

/// OID rozsireni a pouziti.
pub mod oids {
    use alloc::vec::Vec;

    pub fn crl_distribution_points() -> Vec<u8> {
        super::oid(&[2, 5, 29, 31])
    }
    pub fn extended_key_usage() -> Vec<u8> {
        super::oid(&[2, 5, 29, 37])
    }
    pub fn ocsp_signing() -> Vec<u8> {
        super::oid(&[1, 3, 6, 1, 5, 5, 7, 3, 9])
    }
    pub fn ocsp_basic() -> Vec<u8> {
        super::oid(&[1, 3, 6, 1, 5, 5, 7, 48, 1, 1])
    }
    pub fn sha1() -> Vec<u8> {
        super::oid(&[1, 3, 14, 3, 2, 26])
    }
    pub fn sha256() -> Vec<u8> {
        super::oid(&[2, 16, 840, 1, 101, 3, 4, 2, 1])
    }
}

impl<'a> Cert<'a> {
    pub fn parse(der: &'a [u8]) -> Result<Self> {
        let mut outer = Der::new(der);
        let mut cert = outer.nested(tag::SEQUENCE)?;
        if !outer.is_empty() {
            return Err(Malformed);
        }
        let tbs_tlv = cert.expect(tag::SEQUENCE)?;
        let sig_alg = cert.expect(tag::SEQUENCE)?.value;
        let signature = bits(&cert.expect(tag::BIT_STRING)?)?;
        let mut tbs = Der::new(tbs_tlv.value);
        tbs.optional(tag::ctx(0))?; // verze
        let serial = tbs.expect(tag::INTEGER)?.value;
        tbs.expect(tag::SEQUENCE)?; // algoritmus podpisu (znovu)
        let issuer = tbs.expect(tag::SEQUENCE)?.raw;
        let mut validity = tbs.nested(tag::SEQUENCE)?;
        let not_before = time(&validity.any()?)?;
        let not_after = time(&validity.any()?)?;
        let subject = tbs.expect(tag::SEQUENCE)?.raw;
        let spki_tlv = tbs.expect(tag::SEQUENCE)?;
        let mut spki = Der::new(spki_tlv.value);
        let spki_alg = spki.expect(tag::SEQUENCE)?.value;
        let public_key = bits(&spki.expect(tag::BIT_STRING)?)?;
        tbs.optional(tag::ctx_prim(1))?; // issuerUniqueID
        tbs.optional(tag::ctx_prim(2))?; // subjectUniqueID
        let mut extensions = Vec::new();
        if let Some(ext) = tbs.optional(tag::ctx(3))? {
            let mut list = Der::new(ext.value).nested(tag::SEQUENCE)?;
            while !list.is_empty() {
                let mut e = list.nested(tag::SEQUENCE)?;
                let id = e.expect(tag::OID)?.value;
                let critical = match e.optional(tag::BOOLEAN)? {
                    Some(b) => b.value == [0xFF],
                    None => false,
                };
                let value = e.expect(tag::OCTET_STRING)?.value;
                extensions.push((id, critical, value));
            }
        }
        Ok(Self {
            tbs: tbs_tlv.raw,
            serial,
            issuer,
            subject,
            spki: spki_tlv.raw,
            spki_alg,
            public_key,
            sig_alg,
            signature,
            not_before,
            not_after,
            extensions,
        })
    }

    pub fn key(&self) -> Key<'a> {
        Key {
            subject: self.subject,
            spki_alg: self.spki_alg,
            public_key: self.public_key,
        }
    }

    pub fn extension(&self, id: &[u8]) -> Option<&'a [u8]> {
        self.extensions.iter().find(|(o, _, _)| *o == id).map(|(_, _, v)| *v)
    }

    /// Adresy `http://` z rozsireni CRL Distribution Points (jen
    /// `fullName` s URI; distribucni body s `reasons` nebo `cRLIssuer` se
    /// preskoci - webpki je neumi pouzit a jsou vzacne).
    pub fn crl_urls(&self) -> Vec<String> {
        let mut out = Vec::new();
        let Some(ext) = self.extension(&oids::crl_distribution_points()) else {
            return out;
        };
        let _ = (|| -> Result<()> {
            let mut points = Der::new(ext).nested(tag::SEQUENCE)?;
            while !points.is_empty() {
                let mut dp = points.nested(tag::SEQUENCE)?;
                let Some(name) = dp.optional(tag::ctx(0))? else {
                    continue;
                };
                if !dp.is_empty() {
                    continue; // reasons / cRLIssuer
                }
                let mut name = Der::new(name.value);
                // [0] fullName (GeneralNames, IMPLICIT)
                let Some(full) = name.optional(tag::ctx(0))? else {
                    continue;
                };
                let mut names = Der::new(full.value);
                while !names.is_empty() {
                    let gn = names.any()?;
                    // [6] uniformResourceIdentifier (IA5String, IMPLICIT)
                    if gn.tag == tag::ctx_prim(6)
                        && let Ok(uri) = core::str::from_utf8(gn.value)
                        && uri.starts_with("http://")
                        && uri.len() <= 2048
                    {
                        out.push(String::from(uri));
                    }
                }
            }
            Ok(())
        })();
        out
    }

    /// Ma rozsireni Extended Key Usage s ucelem `purpose`?
    pub fn has_eku(&self, purpose: &[u8]) -> bool {
        let Some(ext) = self.extension(&oids::extended_key_usage()) else {
            return false;
        };
        let Ok(mut list) = Der::new(ext).nested(tag::SEQUENCE) else {
            return false;
        };
        while let Ok(t) = list.expect(tag::OID) {
            if t.value == purpose {
                return true;
            }
        }
        false
    }
}

/// Casy CRL (thisUpdate, nextUpdate) a vystavitel.
#[derive(Debug, Clone, Copy)]
pub struct CrlInfo<'a> {
    pub issuer: &'a [u8],
    pub this_update: i64,
    pub next_update: Option<i64>,
}

pub fn crl_info(der: &[u8]) -> Result<CrlInfo<'_>> {
    let mut list = Der::new(der).nested(tag::SEQUENCE)?;
    let mut tbs = list.nested(tag::SEQUENCE)?;
    tbs.optional(tag::INTEGER)?; // verze
    tbs.expect(tag::SEQUENCE)?; // algoritmus
    let issuer = tbs.expect(tag::SEQUENCE)?.raw;
    let this_update = time(&tbs.any()?)?;
    let next_update = match tbs.peek_tag() {
        Some(tag::UTC_TIME | tag::GENERALIZED_TIME) => Some(time(&tbs.any()?)?),
        _ => None,
    };
    Ok(CrlInfo { issuer, this_update, next_update })
}
