//! OneCRL: seznam odvolanych certifikatu CA od Mozilly (remote settings,
//! kolekce `security-state/onecrl`, stejny jako ve Firefoxu).
//!
//! Zaznam je bud `issuerName` + `serialNumber` (base64 DER jmena
//! vystavitele a obsahu cisla), nebo `subject` + `pubKeyHash` (base64 DER
//! jmena subjektu a SHA-256 SubjectPublicKeyInfo). Zaznamy s
//! `"enabled": false` se preskoci.

use alloc::vec::Vec;

use tls::revocation::CaQuery;
use tls::roots::base64_decode;

use crate::json;

#[derive(Debug, Default)]
pub struct OneCrl {
    by_issuer: Vec<(Vec<u8>, Vec<u8>)>,
    by_subject: Vec<(Vec<u8>, [u8; 32])>,
}

/// Cislo certifikatu bez uvodnich nul (DER muze mit nulu kvuli znamenku).
fn serial(s: &[u8]) -> &[u8] {
    let skip = s.iter().take_while(|&&b| b == 0).count().min(s.len().saturating_sub(1));
    &s[skip..]
}

impl OneCrl {
    /// Rozebere odpoved (`{"changes": [...]}` nebo `{"data": [...]}`).
    /// `None` = neplatny dokument (pak plati predchozi seznam).
    pub fn parse(text: &str) -> Option<Self> {
        let doc = json::parse(text)?;
        let records = doc.get("changes").or_else(|| doc.get("data"))?.as_array()?;
        let mut out = Self::default();
        for r in records {
            if r.get("enabled") == Some(&json::Value::Bool(false)) {
                continue;
            }
            let b64 = |k: &str| r.get(k).and_then(|v| v.as_str()).and_then(|s| base64_decode(s.as_bytes()));
            if let (Some(issuer), Some(sn)) = (b64("issuerName"), b64("serialNumber")) {
                out.by_issuer.push((issuer, serial(&sn).to_vec()));
            } else if let (Some(subject), Some(hash)) = (b64("subject"), b64("pubKeyHash"))
                && let Ok(h) = <[u8; 32]>::try_from(hash.as_slice())
            {
                out.by_subject.push((subject, h));
            }
        }
        Some(out)
    }

    pub fn len(&self) -> usize {
        self.by_issuer.len() + self.by_subject.len()
    }

    pub fn revoked(&self, q: &CaQuery) -> bool {
        let sn = serial(&q.serial);
        self.by_issuer.iter().any(|(i, s)| *i == q.issuer && s.as_slice() == sn)
            || self.by_subject.iter().any(|(s, h)| *s == q.subject && *h == q.spki_sha256)
    }
}
