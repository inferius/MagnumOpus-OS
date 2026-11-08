//! Klice COSE (RFC 9052/9053) pouzivane v CTAP2: verejne klice
//! prihlasovacich udaju (ES256, EdDSA) a klic dohody ECDH P-256.

use alloc::vec;
use alloc::vec::Vec;

use p256::ecdsa::signature::Verifier;
use p256::elliptic_curve::sec1::FromEncodedPoint;

use super::cbor::{Value, int_map};

pub const KTY_OKP: i64 = 1;
pub const KTY_EC2: i64 = 2;
pub const ALG_ES256: i64 = -7;
pub const ALG_EDDSA: i64 = -8;
/// Klic dohody v clientPIN (ECDH-ES+HKDF-256).
pub const ALG_ECDH: i64 = -25;
pub const CRV_P256: i64 = 1;
pub const CRV_ED25519: i64 = 6;

/// Verejny klic prihlasovacich udaju.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PublicKey {
    Es256 { x: [u8; 32], y: [u8; 32] },
    Ed25519([u8; 32]),
}

fn arr32(v: Option<&Value>) -> Option<[u8; 32]> {
    v?.as_bytes()?.try_into().ok()
}

impl PublicKey {
    pub fn from_cose(v: &Value) -> Option<Self> {
        let kty = v.get_int(1)?.as_int()?;
        let alg = v.get_int(3)?.as_int()?;
        let crv = v.get_int(-1)?.as_int()?;
        match (kty, alg, crv) {
            (KTY_EC2, ALG_ES256, CRV_P256) => Some(PublicKey::Es256 { x: arr32(v.get_int(-2))?, y: arr32(v.get_int(-3))? }),
            (KTY_OKP, ALG_EDDSA, CRV_ED25519) => Some(PublicKey::Ed25519(arr32(v.get_int(-2))?)),
            _ => None,
        }
    }

    pub fn to_cose(&self) -> Value {
        match self {
            PublicKey::Es256 { x, y } => int_map(vec![
                (1, Value::int(KTY_EC2)),
                (3, Value::int(ALG_ES256)),
                (-1, Value::int(CRV_P256)),
                (-2, Value::bytes(x)),
                (-3, Value::bytes(y)),
            ]),
            PublicKey::Ed25519(x) => int_map(vec![
                (1, Value::int(KTY_OKP)),
                (3, Value::int(ALG_EDDSA)),
                (-1, Value::int(CRV_ED25519)),
                (-2, Value::bytes(x)),
            ]),
        }
    }

    /// Overi podpis `sig` nad `msg` (ES256: DER ECDSA nad SHA-256, EdDSA:
    /// 64 B).
    pub fn verify(&self, msg: &[u8], sig: &[u8]) -> bool {
        match self {
            PublicKey::Es256 { x, y } => {
                let point = p256::EncodedPoint::from_affine_coordinates(x.into(), y.into(), false);
                let Ok(key) = p256::ecdsa::VerifyingKey::from_encoded_point(&point) else {
                    return false;
                };
                let Ok(sig) = p256::ecdsa::Signature::from_der(sig) else {
                    return false;
                };
                // Podpis s velkym `s` (neni "low-S") je take platny ECDSA;
                // klice ho mohou vratit, normalizace ho prijme.
                let sig = sig.normalize_s().unwrap_or(sig);
                key.verify(msg, &sig).is_ok()
            }
            PublicKey::Ed25519(x) => {
                let Ok(key) = ed25519_dalek::VerifyingKey::from_bytes(x) else {
                    return false;
                };
                let Ok(sig) = ed25519_dalek::Signature::from_slice(sig) else {
                    return false;
                };
                key.verify_strict(msg, &sig).is_ok()
            }
        }
    }
}

/// Bod P-256 klice dohody ve tvaru COSE.
pub fn ecdh_key(x: &[u8; 32], y: &[u8; 32]) -> Value {
    int_map(vec![
        (1, Value::int(KTY_EC2)),
        (3, Value::int(ALG_ECDH)),
        (-1, Value::int(CRV_P256)),
        (-2, Value::bytes(x)),
        (-3, Value::bytes(y)),
    ])
}

/// Verejny bod P-256 z klice dohody COSE.
pub fn ecdh_point(v: &Value) -> Option<p256::PublicKey> {
    if v.get_int(1)?.as_int()? != KTY_EC2 || v.get_int(-1)?.as_int()? != CRV_P256 {
        return None;
    }
    let (x, y) = (arr32(v.get_int(-2))?, arr32(v.get_int(-3))?);
    let point = p256::EncodedPoint::from_affine_coordinates(&x.into(), &y.into(), false);
    Option::from(p256::PublicKey::from_encoded_point(&point))
}

/// Souradnice verejneho bodu P-256.
pub fn xy(pk: &p256::PublicKey) -> ([u8; 32], [u8; 32]) {
    use p256::elliptic_curve::sec1::ToEncodedPoint;
    let p = pk.to_encoded_point(false);
    let mut x = [0u8; 32];
    let mut y = [0u8; 32];
    x.copy_from_slice(p.x().map(|v| v.as_slice()).unwrap_or(&[0; 32]));
    y.copy_from_slice(p.y().map(|v| v.as_slice()).unwrap_or(&[0; 32]));
    (x, y)
}

/// Bajty zakodovaneho klice COSE (ulozeni v DB).
pub fn encode_public(pk: &PublicKey) -> Vec<u8> {
    pk.to_cose().encode()
}
