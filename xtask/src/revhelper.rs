//! Odvolani certifikatu pro testovaci servery (`webhelper`): CRL (rcgen),
//! odpoved OCSP (sestavena zde, podepsana ring) a dokument OneCRL.
//!
//! Pole certifikatu (jmeno vystavitele, klic, cislo) cte `tls::x509` -
//! jen cteni, podpisy a overeni jsou nezavisle (ring, webpki v klientovi).

use rcgen::{KeyPair, SerialNumber};
use tls::x509::Cert;

fn tlv(tag: u8, content: &[u8]) -> Vec<u8> {
    let mut out = vec![tag];
    let len = content.len();
    if len < 0x80 {
        out.push(len as u8);
    } else if len < 0x100 {
        out.extend_from_slice(&[0x81, len as u8]);
    } else {
        out.extend_from_slice(&[0x82, (len >> 8) as u8, len as u8]);
    }
    out.extend_from_slice(content);
    out
}

fn seq(parts: &[Vec<u8>]) -> Vec<u8> {
    tlv(0x30, &parts.concat())
}

fn gtime(s: &str) -> Vec<u8> {
    tlv(0x18, s.as_bytes())
}

fn sha1(data: &[u8]) -> Vec<u8> {
    ring::digest::digest(&ring::digest::SHA1_FOR_LEGACY_USE_ONLY, data).as_ref().to_vec()
}

/// CRL vystavitele `issuer` s odvolanymi cisly `revoked`.
pub fn crl(issuer: &rcgen::Certificate, key: &KeyPair, revoked: &[u64]) -> Vec<u8> {
    rcgen::CertificateRevocationListParams {
        this_update: rcgen::date_time_ymd(2020, 1, 1),
        next_update: rcgen::date_time_ymd(2099, 1, 1),
        crl_number: SerialNumber::from(1u64),
        issuing_distribution_point: None,
        revoked_certs: revoked
            .iter()
            .map(|&s| rcgen::RevokedCertParams {
                serial_number: SerialNumber::from(s),
                revocation_time: rcgen::date_time_ymd(2021, 1, 1),
                reason_code: Some(rcgen::RevocationReason::KeyCompromise),
                invalidity_date: None,
            })
            .collect(),
        key_identifier_method: rcgen::KeyIdMethod::Sha256,
    }
    .signed_by(issuer, key)
    .unwrap()
    .der()
    .to_vec()
}

/// Odpoved OCSP "revoked" pro `leaf` od `issuer` (klic CA = podpisovatel).
pub fn ocsp_revoked(issuer_der: &[u8], issuer_key: &KeyPair, leaf_der: &[u8]) -> Vec<u8> {
    let issuer = Cert::parse(issuer_der).unwrap();
    let leaf = Cert::parse(leaf_der).unwrap();
    let cert_id = seq(&[
        seq(&[tlv(0x06, &tls::x509::oid(&[1, 3, 14, 3, 2, 26])), vec![0x05, 0x00]]),
        tlv(0x04, &sha1(issuer.subject)),
        tlv(0x04, &sha1(issuer.public_key)),
        tlv(0x02, leaf.serial),
    ]);
    let status = tlv(0xA1, &gtime("20210101000000Z"));
    let single = seq(&[cert_id, status, gtime("20200101000000Z"), tlv(0xA0, &gtime("20990101000000Z"))]);
    let tbs = seq(&[tlv(0xA1, issuer.subject), gtime("20200101000000Z"), seq(&[single])]);
    let rng = ring::rand::SystemRandom::new();
    let kp = ring::signature::EcdsaKeyPair::from_pkcs8(
        &ring::signature::ECDSA_P256_SHA256_ASN1_SIGNING,
        &issuer_key.serialize_der(),
        &rng,
    )
    .unwrap();
    let sig = kp.sign(&rng, &tbs).unwrap();
    let mut bits = vec![0u8];
    bits.extend_from_slice(sig.as_ref());
    let ecdsa_sha256 = seq(&[tlv(0x06, &tls::x509::oid(&[1, 2, 840, 10045, 4, 3, 2]))]);
    let basic = seq(&[tbs, ecdsa_sha256, tlv(0x03, &bits)]);
    let ocsp_basic = tlv(0x06, &tls::x509::oid(&[1, 3, 6, 1, 5, 5, 7, 48, 1, 1]));
    seq(&[tlv(0x0A, &[0]), tlv(0xA0, &seq(&[ocsp_basic, tlv(0x04, &basic)]))])
}

fn b64(data: &[u8]) -> String {
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in data.chunks(3) {
        let v = chunk.iter().enumerate().fold(0u32, |v, (i, &b)| v | u32::from(b) << (16 - 8 * i));
        for i in 0..4 {
            out.push(if i <= chunk.len() { A[(v >> (18 - 6 * i) & 63) as usize] as char } else { '=' });
        }
    }
    out
}

/// Dokument OneCRL (jako remote settings Mozilly) s certifikatem `ca_der`.
pub fn onecrl(ca_der: &[u8]) -> String {
    let c = Cert::parse(ca_der).unwrap();
    format!(
        "{{\"changes\":[{{\"issuerName\":\"{}\",\"serialNumber\":\"{}\",\"enabled\":true,\"id\":\"test\"}},\
         {{\"issuerName\":\"AAAA\",\"serialNumber\":\"AQ==\",\"enabled\":false}}],\"timestamp\":1}}",
        b64(c.issuer),
        b64(c.serial)
    )
}
