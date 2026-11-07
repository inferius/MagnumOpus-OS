//! CRL, ktere webpki neprecte: rozsah "jen certifikaty CA".
//!
//! webpki (rustls) odmita CRL s kritickym rozsirenim Issuing Distribution
//! Point, ktere ma `onlyContainsCACerts` (chyba
//! `UnsupportedCrlIssuingDistributionPoint`). Presne takove CRL vydavaji
//! korenove CA Let's Encrypt (ISRG Root X1, Root YR) pro odvolane mezilehle
//! CA. Tady se ctou a overuji samy: podpis vydavatele CRL, platnost
//! (`nextUpdate`) a cisla odvolanych certifikatu.
//!
//! Cokoli dalsiho neznameho (delta CRL, neprima CRL, `onlySomeReasons`,
//! jine kriticke rozsireni CRL nebo polozky) = CRL se nepouzije a stav
//! odvolani je neznamy (mekke selhani, viz [`crate::revocation`]).

use alloc::vec::Vec;

use crate::x509::{Der, Malformed, oid, tag, time};

/// Precteny CRL s rozsahem jen pro certifikaty CA.
#[derive(Debug, Clone)]
pub struct CaCrl<'a> {
    /// Cela TBSCertList (podepsana cast).
    pub tbs: &'a [u8],
    /// Obsah AlgorithmIdentifier podpisu.
    pub sig_alg: &'a [u8],
    /// Podpis (obsah BIT STRING bez bajtu nevyuzitych bitu).
    pub signature: &'a [u8],
    /// Cela polozka Name vydavatele.
    pub issuer: &'a [u8],
    pub this_update: i64,
    pub next_update: Option<i64>,
    /// Obsahy INTEGER odvolanych cisel (jak jsou v DER).
    pub revoked: Vec<&'a [u8]>,
}

/// Proc CRL nejde pouzit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CrlError {
    /// Poskozena struktura.
    Malformed,
    /// Neni to CRL jen pro CA (tu zpracuje webpki, nebo nikdo).
    NotCaOnly,
    /// Rozsireni, ktere tu neumime (delta, neprima CRL, duvody, ...).
    Unsupported,
}

impl From<Malformed> for CrlError {
    fn from(_: Malformed) -> Self {
        Self::Malformed
    }
}

fn issuing_distribution_point() -> Vec<u8> {
    oid(&[2, 5, 29, 28])
}

/// Rozsireni CRL, ktera nic nemeni (AKI, cislo CRL).
fn harmless(id: &[u8]) -> bool {
    id == oid(&[2, 5, 29, 35]).as_slice() || id == oid(&[2, 5, 29, 20]).as_slice()
}

/// Je IDP "jen certifikaty CA" bez dalsich omezeni?
fn idp_ca_only(value: &[u8]) -> core::result::Result<bool, CrlError> {
    let mut idp = Der::new(value).nested(tag::SEQUENCE)?;
    let mut ca_only = false;
    while !idp.is_empty() {
        let t = idp.any()?;
        let set = t.value == [0xFF];
        match t.tag {
            // distributionPoint: CRL jsme stahli z adresy v certifikatu.
            0xA0 => {}
            // onlyContainsUserCerts: tu zpracuje webpki.
            0x81 if set => return Ok(false),
            0x82 => ca_only = set,
            // onlySomeReasons, indirectCRL, onlyContainsAttributeCerts.
            0x83 => return Err(CrlError::Unsupported),
            0x84 | 0x85 if set => return Err(CrlError::Unsupported),
            0x81 | 0x84 | 0x85 => {}
            _ => return Err(CrlError::Malformed),
        }
    }
    Ok(ca_only)
}

/// Precte CRL "jen certifikaty CA" (RFC 5280, kap. 5).
pub fn parse_ca_only(der: &[u8]) -> core::result::Result<CaCrl<'_>, CrlError> {
    let mut outer = Der::new(der);
    let mut list = outer.nested(tag::SEQUENCE)?;
    if !outer.is_empty() {
        return Err(CrlError::Malformed);
    }
    let tbs_tlv = list.expect(tag::SEQUENCE)?;
    let sig_alg = list.expect(tag::SEQUENCE)?.value;
    let signature = match list.expect(tag::BIT_STRING)?.value.split_first() {
        Some((0, rest)) => rest,
        _ => return Err(CrlError::Malformed),
    };
    let mut tbs = Der::new(tbs_tlv.value);
    // Rozsireni (a tedy IDP) ma jen verze 2.
    if tbs.optional(tag::INTEGER)?.map(|v| v.value) != Some(&[1]) {
        return Err(CrlError::NotCaOnly);
    }
    let inner_alg = tbs.expect(tag::SEQUENCE)?.value;
    if inner_alg != sig_alg {
        return Err(CrlError::Malformed);
    }
    let issuer = tbs.expect(tag::SEQUENCE)?.raw;
    let this_update = time(&tbs.any()?)?;
    let next_update = match tbs.peek_tag() {
        Some(tag::UTC_TIME | tag::GENERALIZED_TIME) => Some(time(&tbs.any()?)?),
        _ => None,
    };
    let mut revoked = Vec::new();
    if tbs.peek_tag() == Some(tag::SEQUENCE) {
        let mut entries = tbs.nested(tag::SEQUENCE)?;
        while !entries.is_empty() {
            let mut e = entries.nested(tag::SEQUENCE)?;
            revoked.push(e.expect(tag::INTEGER)?.value);
            time(&e.any()?)?;
            // Kriticke rozsireni polozky (napr. certificateIssuer u neprime
            // CRL) neumime.
            if let Some(ext) = e.optional(tag::SEQUENCE)? {
                let mut exts = Der::new(ext.value);
                while !exts.is_empty() {
                    let mut x = exts.nested(tag::SEQUENCE)?;
                    x.expect(tag::OID)?;
                    if x.optional(tag::BOOLEAN)?.is_some_and(|b| b.value == [0xFF]) {
                        return Err(CrlError::Unsupported);
                    }
                }
            }
        }
    }
    let mut ca_only = false;
    if let Some(ext) = tbs.optional(tag::ctx(0))? {
        let mut exts = Der::new(ext.value).nested(tag::SEQUENCE)?;
        while !exts.is_empty() {
            let mut x = exts.nested(tag::SEQUENCE)?;
            let id = x.expect(tag::OID)?.value;
            let critical = x.optional(tag::BOOLEAN)?.is_some_and(|b| b.value == [0xFF]);
            let value = x.expect(tag::OCTET_STRING)?.value;
            if id == issuing_distribution_point().as_slice() {
                ca_only = idp_ca_only(value)?;
            } else if critical && !harmless(id) {
                return Err(CrlError::Unsupported);
            }
        }
    }
    if !tbs.is_empty() {
        return Err(CrlError::Malformed);
    }
    if !ca_only {
        return Err(CrlError::NotCaOnly);
    }
    Ok(CaCrl {
        tbs: tbs_tlv.raw,
        sig_alg,
        signature,
        issuer,
        this_update,
        next_update,
        revoked,
    })
}
