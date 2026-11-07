//! Overeni podpisu (certifikaty a podpis handshaku serveru).

use pki_types::{AlgorithmIdentifier, InvalidSignature, SignatureVerificationAlgorithm, alg_id};
use rustls::SignatureScheme;
use rustls::crypto::WebPkiSupportedAlgorithms;
use sha2::{Digest, Sha256, Sha384, Sha512};
use signature::Verifier;
use signature::hazmat::PrehashVerifier;

macro_rules! ecdsa {
    ($name:ident, $ty:ident, $curve:ident, $key_alg:expr, $sig_alg:expr, $hash:ty) => {
        #[derive(Debug)]
        struct $ty;

        impl SignatureVerificationAlgorithm for $ty {
            fn public_key_alg_id(&self) -> AlgorithmIdentifier {
                $key_alg
            }

            fn signature_alg_id(&self) -> AlgorithmIdentifier {
                $sig_alg
            }

            fn verify_signature(&self, public_key: &[u8], message: &[u8], sig: &[u8]) -> Result<(), InvalidSignature> {
                let sig = $curve::ecdsa::DerSignature::from_bytes(sig).map_err(|_| InvalidSignature)?;
                let key = $curve::ecdsa::VerifyingKey::from_sec1_bytes(public_key).map_err(|_| InvalidSignature)?;
                let digest = <$hash>::digest(message);
                key.verify_prehash(&digest, &sig).map_err(|_| InvalidSignature)
            }
        }

        const $name: &dyn SignatureVerificationAlgorithm = &$ty;
    };
}

ecdsa!(ECDSA_P256_SHA256, EcdsaP256Sha256, p256, alg_id::ECDSA_P256, alg_id::ECDSA_SHA256, Sha256);
ecdsa!(ECDSA_P256_SHA384, EcdsaP256Sha384, p256, alg_id::ECDSA_P256, alg_id::ECDSA_SHA384, Sha384);
ecdsa!(ECDSA_P384_SHA256, EcdsaP384Sha256, p384, alg_id::ECDSA_P384, alg_id::ECDSA_SHA256, Sha256);
ecdsa!(ECDSA_P384_SHA384, EcdsaP384Sha384, p384, alg_id::ECDSA_P384, alg_id::ECDSA_SHA384, Sha384);

#[derive(Debug)]
struct Ed25519;

impl SignatureVerificationAlgorithm for Ed25519 {
    fn public_key_alg_id(&self) -> AlgorithmIdentifier {
        alg_id::ED25519
    }

    fn signature_alg_id(&self) -> AlgorithmIdentifier {
        alg_id::ED25519
    }

    fn verify_signature(&self, public_key: &[u8], message: &[u8], sig: &[u8]) -> Result<(), InvalidSignature> {
        let key: [u8; 32] = public_key.try_into().map_err(|_| InvalidSignature)?;
        let key = ed25519_dalek::VerifyingKey::from_bytes(&key).map_err(|_| InvalidSignature)?;
        let sig = ed25519_dalek::Signature::from_slice(sig).map_err(|_| InvalidSignature)?;
        // Prisne overeni (RFC 8032): odmitne slabe klice a nekanonicke podpisy.
        key.verify_strict(message, &sig).map_err(|_| InvalidSignature)
    }
}

const ED25519: &dyn SignatureVerificationAlgorithm = &Ed25519;

/// RSA: verejny klic v PKCS#1 (tak ho preda webpki). Nejmensi delka klice
/// 2048 bitu (kratsi jsou prolomitelne).
fn rsa_key(public_key: &[u8]) -> Result<rsa::RsaPublicKey, InvalidSignature> {
    use rsa::pkcs1::DecodeRsaPublicKey;
    use rsa::traits::PublicKeyParts;
    let key = rsa::RsaPublicKey::from_pkcs1_der(public_key).map_err(|_| InvalidSignature)?;
    if key.size() * 8 < 2048 {
        return Err(InvalidSignature);
    }
    Ok(key)
}

macro_rules! rsa_alg {
    ($name:ident, $ty:ident, $sig_alg:expr, $verifier:ty, $sig:ty) => {
        #[derive(Debug)]
        struct $ty;

        impl SignatureVerificationAlgorithm for $ty {
            fn public_key_alg_id(&self) -> AlgorithmIdentifier {
                alg_id::RSA_ENCRYPTION
            }

            fn signature_alg_id(&self) -> AlgorithmIdentifier {
                $sig_alg
            }

            fn verify_signature(&self, public_key: &[u8], message: &[u8], sig: &[u8]) -> Result<(), InvalidSignature> {
                let key = rsa_key(public_key)?;
                let sig = <$sig>::try_from(sig).map_err(|_| InvalidSignature)?;
                <$verifier>::new(key).verify(message, &sig).map_err(|_| InvalidSignature)
            }
        }

        const $name: &dyn SignatureVerificationAlgorithm = &$ty;
    };
}

use rsa::{pkcs1v15, pss};

rsa_alg!(RSA_PKCS1_SHA256, RsaPkcs1Sha256, alg_id::RSA_PKCS1_SHA256, pkcs1v15::VerifyingKey<Sha256>, pkcs1v15::Signature);
rsa_alg!(RSA_PKCS1_SHA384, RsaPkcs1Sha384, alg_id::RSA_PKCS1_SHA384, pkcs1v15::VerifyingKey<Sha384>, pkcs1v15::Signature);
rsa_alg!(RSA_PKCS1_SHA512, RsaPkcs1Sha512, alg_id::RSA_PKCS1_SHA512, pkcs1v15::VerifyingKey<Sha512>, pkcs1v15::Signature);
rsa_alg!(RSA_PSS_SHA256, RsaPssSha256, alg_id::RSA_PSS_SHA256, pss::VerifyingKey<Sha256>, pss::Signature);
rsa_alg!(RSA_PSS_SHA384, RsaPssSha384, alg_id::RSA_PSS_SHA384, pss::VerifyingKey<Sha384>, pss::Signature);
rsa_alg!(RSA_PSS_SHA512, RsaPssSha512, alg_id::RSA_PSS_SHA512, pss::VerifyingKey<Sha512>, pss::Signature);

/// Algoritmy pro webpki (certifikaty) a mapovani schemat podpisu TLS.
pub static ALGORITHMS: WebPkiSupportedAlgorithms = WebPkiSupportedAlgorithms {
    all: &[
        ECDSA_P256_SHA256,
        ECDSA_P256_SHA384,
        ECDSA_P384_SHA256,
        ECDSA_P384_SHA384,
        ED25519,
        RSA_PKCS1_SHA256,
        RSA_PKCS1_SHA384,
        RSA_PKCS1_SHA512,
        RSA_PSS_SHA256,
        RSA_PSS_SHA384,
        RSA_PSS_SHA512,
    ],
    mapping: &[
        (SignatureScheme::ECDSA_NISTP384_SHA384, &[ECDSA_P384_SHA384, ECDSA_P256_SHA384]),
        (SignatureScheme::ECDSA_NISTP256_SHA256, &[ECDSA_P256_SHA256, ECDSA_P384_SHA256]),
        (SignatureScheme::ED25519, &[ED25519]),
        (SignatureScheme::RSA_PSS_SHA512, &[RSA_PSS_SHA512]),
        (SignatureScheme::RSA_PSS_SHA384, &[RSA_PSS_SHA384]),
        (SignatureScheme::RSA_PSS_SHA256, &[RSA_PSS_SHA256]),
        (SignatureScheme::RSA_PKCS1_SHA512, &[RSA_PKCS1_SHA512]),
        (SignatureScheme::RSA_PKCS1_SHA384, &[RSA_PKCS1_SHA384]),
        (SignatureScheme::RSA_PKCS1_SHA256, &[RSA_PKCS1_SHA256]),
    ],
};
