//! Overeni odpovedi OCSP (RFC 6960) poslane serverem v handshaku (OCSP
//! stapling, RFC 6066 a RFC 8446 4.4.2.1).
//!
//! Postup:
//! 1. `OCSPResponse` se stavem `successful` a typem `id-pkix-ocsp-basic`.
//! 2. Podpisovatel: vydavatel certifikatu serveru, nebo poverena odpovidajici
//!    autorita (certifikat v odpovedi vydany vydavatelem s EKU
//!    `id-kp-OCSPSigning`, platny ted). Podpis `tbsResponseData` se overi.
//! 3. Polozka pro certifikat serveru: cislo + otisk jmena a klice vydavatele
//!    (SHA-1 nebo SHA-256).
//! 4. Casy: `thisUpdate` ne v budoucnosti (tolerance 5 min), `nextUpdate`
//!    ne v minulosti; bez `nextUpdate` plati 1 den od `thisUpdate`.
//!
//! Vysledek: `Good` / `Revoked` / `Unknown` (odpoved pro jiny certifikat,
//! propadla, stav `unknown`). Spatny podpis nebo poskozena struktura =
//! chyba (podvrzena odpoved).

use sha1::Sha1;
use sha2::{Digest, Sha256};

use crate::provider::ALGORITHMS;
use crate::x509::{Cert, Der, Key, Malformed, oids, tag, time};

/// Tolerance rozdilu hodin vuci respondentovi.
const SKEW: i64 = 300;
/// Platnost odpovedi bez `nextUpdate`.
const NO_NEXT_UPDATE: i64 = 86_400;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Good,
    Revoked,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OcspError {
    /// Poskozena odpoved nebo neocekavana struktura.
    Malformed,
    /// Podpis neodpovida nebo podpisovatel nema pravo podepisovat.
    BadSignature,
    /// Respondent hlasi chybu (malformedRequest, tryLater, ...).
    NotSuccessful,
}

impl From<Malformed> for OcspError {
    fn from(_: Malformed) -> Self {
        Self::Malformed
    }
}

/// Overi podpis `message` klicem `key` (SubjectPublicKeyInfo z certifikatu)
/// algoritmem `sig_alg` (obsah AlgorithmIdentifier).
pub fn verify_signature(key: &Key<'_>, sig_alg: &[u8], message: &[u8], signature: &[u8]) -> bool {
    ALGORITHMS.all.iter().any(|alg| {
        alg.public_key_alg_id().as_ref() == key.spki_alg
            && alg.signature_alg_id().as_ref() == sig_alg
            && alg.verify_signature(key.public_key, message, signature).is_ok()
    })
}

fn hash(alg: &[u8], data: &[u8]) -> Option<alloc::vec::Vec<u8>> {
    if alg == oids::sha1().as_slice() {
        Some(Sha1::digest(data).to_vec())
    } else if alg == oids::sha256().as_slice() {
        Some(Sha256::digest(data).to_vec())
    } else {
        None
    }
}

/// Je `signer` podpisovatel odpovedi podle ResponderID?
fn responder_matches(responder: &crate::x509::Tlv<'_>, signer: &Key<'_>) -> bool {
    match responder.tag {
        t if t == tag::ctx(1) => responder.value == signer.subject,
        t if t == tag::ctx(2) => Der::new(responder.value)
            .expect(tag::OCTET_STRING)
            .is_ok_and(|k| k.value == Sha1::digest(signer.public_key).as_slice()),
        _ => false,
    }
}

/// Overi odpoved `response` pro certifikat `leaf` vydany `issuer` v case
/// `now` (sekundy od 1970).
pub fn check(response: &[u8], leaf: &Cert<'_>, issuer: &Key<'_>, now: i64) -> Result<Status, OcspError> {
    let mut outer = Der::new(response).nested(tag::SEQUENCE)?;
    let status = outer.expect(tag::ENUMERATED)?;
    if status.value != [0] {
        return Err(OcspError::NotSuccessful);
    }
    let mut bytes = Der::new(outer.expect(tag::ctx(0))?.value).nested(tag::SEQUENCE)?;
    if bytes.expect(tag::OID)?.value != oids::ocsp_basic().as_slice() {
        return Err(OcspError::Malformed);
    }
    let basic_der = bytes.expect(tag::OCTET_STRING)?.value;
    let mut basic = Der::new(basic_der).nested(tag::SEQUENCE)?;
    let tbs = basic.expect(tag::SEQUENCE)?;
    let sig_alg = basic.expect(tag::SEQUENCE)?.value;
    let sig_bits = basic.expect(tag::BIT_STRING)?.value;
    let signature = match sig_bits.split_first() {
        Some((0, s)) => s,
        _ => return Err(OcspError::Malformed),
    };
    let extra_certs = basic.optional(tag::ctx(0))?;

    let mut data = Der::new(tbs.value);
    data.optional(tag::ctx(0))?; // verze
    let responder = data.any()?;
    data.expect(tag::GENERALIZED_TIME)?; // producedAt
    let mut responses = data.nested(tag::SEQUENCE)?;

    // Podpisovatel: vydavatel, nebo poverena autorita z odpovedi.
    let signed_by_issuer = responder_matches(&responder, issuer)
        && verify_signature(issuer, sig_alg, tbs.raw, signature);
    if !signed_by_issuer {
        let Some(certs) = extra_certs else {
            return Err(OcspError::BadSignature);
        };
        let mut list = Der::new(certs.value).nested(tag::SEQUENCE)?;
        let mut ok = false;
        while !list.is_empty() {
            let raw = list.expect(tag::SEQUENCE)?.raw;
            let Ok(d) = Cert::parse(raw) else {
                continue;
            };
            if responder_matches(&responder, &d.key())
                && d.issuer == issuer.subject
                && verify_signature(issuer, d.sig_alg, d.tbs, d.signature)
                && d.has_eku(&oids::ocsp_signing())
                && d.not_before <= now + SKEW
                && now <= d.not_after
                && verify_signature(&d.key(), sig_alg, tbs.raw, signature)
            {
                ok = true;
                break;
            }
        }
        if !ok {
            return Err(OcspError::BadSignature);
        }
    }

    // Polozka pro nas certifikat.
    while !responses.is_empty() {
        let mut single = responses.nested(tag::SEQUENCE)?;
        let mut id = single.nested(tag::SEQUENCE)?;
        let alg = Der::new(id.expect(tag::SEQUENCE)?.value).expect(tag::OID)?.value;
        let name_hash = id.expect(tag::OCTET_STRING)?.value;
        let key_hash = id.expect(tag::OCTET_STRING)?.value;
        let serial = id.expect(tag::INTEGER)?.value;
        let cert_status = single.any()?;
        let this_update = time(&single.expect(tag::GENERALIZED_TIME)?)?;
        let next_update = match single.optional(tag::ctx(0))? {
            Some(t) => Some(time(&Der::new(t.value).expect(tag::GENERALIZED_TIME)?)?),
            None => None,
        };
        let matches = serial == leaf.serial
            && hash(alg, issuer.subject).is_some_and(|h| h == name_hash)
            && hash(alg, issuer.public_key).is_some_and(|h| h == key_hash);
        if !matches {
            continue;
        }
        let fresh = this_update <= now + SKEW && now <= next_update.unwrap_or(this_update + NO_NEXT_UPDATE) + SKEW;
        if !fresh {
            return Ok(Status::Unknown);
        }
        return Ok(match cert_status.tag {
            t if t == tag::ctx_prim(0) => Status::Good,
            t if t == tag::ctx(1) => Status::Revoked,
            _ => Status::Unknown,
        });
    }
    Ok(Status::Unknown)
}
