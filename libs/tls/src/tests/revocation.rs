//! Kontrola odvolani: CRL (rcgen), OCSP stapling (odpoved sestavena zde
//! a podepsana klicem CA pres ring) a odvolane CA (OneCRL).

use std::string::{String, ToString};
use std::sync::{Arc, Mutex};
use std::vec;
use std::vec::Vec;

use pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use rcgen::{BasicConstraints, CertificateParams, CrlDistributionPoint, DnType, IsCa, KeyPair, SerialNumber};
use sha1::{Digest, Sha1};
use sha2::Sha256;

use super::{Mode, PLATFORM, Pki, ServerOpts, connect, server};
use crate::revocation::{CaQuery, RevocationSource, proto};
use crate::roots::Roots;
use crate::x509::{self, Cert, oid};
use crate::{Client, Error};

pub(super) const CRL_CA: &str = "http://crl.test/ca.crl";
pub(super) const CRL_INTER: &str = "http://crl.test/inter.crl";

pub(super) struct Ca {
    pub(super) cert: rcgen::Certificate,
    pub(super) key: KeyPair,
}

pub(super) fn ca(name: &str, parent: Option<&Ca>, crl_url: Option<&str>) -> Ca {
    let key = KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).unwrap();
    let mut p = CertificateParams::new(Vec::<String>::new()).unwrap();
    p.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    p.distinguished_name.push(DnType::CommonName, name);
    p.not_before = rcgen::date_time_ymd(2020, 1, 1);
    p.not_after = rcgen::date_time_ymd(2199, 1, 1);
    if let Some(u) = crl_url {
        p.crl_distribution_points = vec![CrlDistributionPoint { uris: vec![u.to_string()] }];
    }
    let cert = match parent {
        Some(parent) => p.signed_by(&key, &parent.cert, &parent.key).unwrap(),
        None => p.self_signed(&key).unwrap(),
    };
    Ca { cert, key }
}

/// Certifikat serveru `localhost` s cislem `serial`.
pub(super) fn leaf(issuer: &Ca, serial: u64, crl_url: Option<&str>) -> (CertificateDer<'static>, PrivateKeyDer<'static>) {
    let key = KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).unwrap();
    let mut p = CertificateParams::new(vec![String::from("localhost")]).unwrap();
    p.serial_number = Some(SerialNumber::from(serial));
    p.not_before = rcgen::date_time_ymd(2020, 1, 1);
    p.not_after = rcgen::date_time_ymd(2198, 1, 1);
    if let Some(u) = crl_url {
        p.crl_distribution_points = vec![CrlDistributionPoint { uris: vec![u.to_string()] }];
    }
    let cert = p.signed_by(&key, &issuer.cert, &issuer.key).unwrap();
    (cert.der().clone(), PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key.serialize_der())))
}

pub(super) fn crl(issuer: &Ca, revoked: &[u64], next_year: i32) -> Vec<u8> {
    let params = rcgen::CertificateRevocationListParams {
        this_update: rcgen::date_time_ymd(2020, 1, 1),
        next_update: rcgen::date_time_ymd(next_year, 1, 1),
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
    };
    params.signed_by(&issuer.cert, &issuer.key).unwrap().der().to_vec()
}

/// Testovaci zdroj odvolani.
#[derive(Debug, Default)]
pub(super) struct Source {
    pub(super) crls: Vec<(&'static str, Vec<u8>)>,
    pub(super) revoked_cas: Vec<CaQuery>,
    pub(super) asked: Mutex<Vec<String>>,
}

impl RevocationSource for Source {
    fn crl(&self, url: &str) -> Option<Vec<u8>> {
        self.asked.lock().unwrap().push(url.to_string());
        self.crls.iter().find(|(u, _)| *u == url).map(|(_, d)| d.clone())
    }

    fn ca_revoked(&self, q: &CaQuery) -> bool {
        self.revoked_cas.contains(q)
    }
}

pub(super) fn pki(root: &Ca, chain: Vec<CertificateDer<'static>>, key: PrivateKeyDer<'static>) -> Pki {
    Pki {
        ca_pem: root.cert.pem(),
        chain,
        key,
    }
}

/// Spojeni k serveru s `pki` (a OCSP); vrati vysledek handshaku.
pub(super) fn try_connect(pki: &Pki, source: Option<Arc<Source>>, ocsp: Vec<u8>) -> Result<(), Error> {
    let roots = Roots::from_files([("ca.pem", pki.ca_pem.as_bytes())]);
    let src = source.map(|s| s as Arc<dyn RevocationSource>);
    let c = Client::with_revocation(&PLATFORM, &roots, &[], src).unwrap();
    let (port, h) = server(pki, ServerOpts { mode: Mode::Handshake, ocsp, ..Default::default() });
    let r = c.connect("localhost", connect(port)).map(|mut s| {
        s.close().unwrap();
    });
    let _ = h.join();
    r
}

pub(super) fn expect_revoked(r: Result<(), Error>) {
    match r {
        Err(Error::Certificate(m)) if m.contains("Revoked") => {}
        other => panic!("cekano Revoked, prislo {other:?}"),
    }
}

#[test]
fn crl_revoked_leaf() {
    let root = ca("Root CRL", None, None);
    let (good, good_key) = leaf(&root, 100, Some(CRL_CA));
    let (bad, bad_key) = leaf(&root, 101, Some(CRL_CA));
    let source = Arc::new(Source {
        crls: vec![(CRL_CA, crl(&root, &[101], 2099))],
        ..Default::default()
    });
    try_connect(&pki(&root, vec![good], good_key), Some(source.clone()), Vec::new()).unwrap();
    expect_revoked(try_connect(&pki(&root, vec![bad.clone()], bad_key.clone_key()), Some(source.clone()), Vec::new()));
    assert!(source.asked.lock().unwrap().iter().all(|u| u == CRL_CA));
    // Bez zdroje odvolani (nebo bez CRL) projde - stav nezname.
    try_connect(&pki(&root, vec![bad.clone()], bad_key.clone_key()), None, Vec::new()).unwrap();
    try_connect(&pki(&root, vec![bad.clone()], bad_key.clone_key()), Some(Arc::new(Source::default())), Vec::new()).unwrap();
    // Propadla CRL se nepouzije.
    let stale = Arc::new(Source {
        crls: vec![(CRL_CA, crl(&root, &[101], 2021))],
        ..Default::default()
    });
    try_connect(&pki(&root, vec![bad.clone()], bad_key.clone_key()), Some(stale), Vec::new()).unwrap();
    // CRL podepsana jinou CA (podvrzena) se neuplatni ani neodmitne.
    let other = ca("Root CRL", None, None);
    let forged = Arc::new(Source {
        crls: vec![(CRL_CA, crl(&other, &[100], 2099))],
        ..Default::default()
    });
    let (good, good_key) = leaf(&root, 100, Some(CRL_CA));
    // Podpis nesedi = stav nezname: spojeni projde (mekke selhani).
    try_connect(&pki(&root, vec![good], good_key), Some(forged), Vec::new()).unwrap();
}

#[test]
fn crl_revoked_intermediate() {
    // Koren -> mezilehla CA (odvolana v CRL korene) -> server.
    let root = ca("Root", None, None);
    let inter = ca("Inter", Some(&root), Some(CRL_CA));
    let (leaf_cert, key) = leaf(&inter, 7, Some(CRL_INTER));
    let inter_serial = Cert::parse(inter.cert.der()).unwrap().serial.to_vec();
    // Cislo mezilehle CA vygeneroval rcgen nahodne: odvolat podle nej.
    let revoked = rcgen::CertificateRevocationListParams {
        this_update: rcgen::date_time_ymd(2020, 1, 1),
        next_update: rcgen::date_time_ymd(2099, 1, 1),
        crl_number: SerialNumber::from(2u64),
        issuing_distribution_point: None,
        revoked_certs: vec![rcgen::RevokedCertParams {
            serial_number: SerialNumber::from_slice(&inter_serial),
            revocation_time: rcgen::date_time_ymd(2021, 1, 1),
            reason_code: None,
            invalidity_date: None,
        }],
        key_identifier_method: rcgen::KeyIdMethod::Sha256,
    }
    .signed_by(&root.cert, &root.key)
    .unwrap()
    .der()
    .to_vec();
    let chain = vec![leaf_cert, inter.cert.der().clone()];
    let source = Arc::new(Source {
        crls: vec![(CRL_CA, revoked), (CRL_INTER, crl(&inter, &[], 2099))],
        ..Default::default()
    });
    expect_revoked(try_connect(&pki(&root, chain.clone(), key.clone_key()), Some(source.clone()), Vec::new()));
    let asked = source.asked.lock().unwrap().clone();
    assert!(asked.contains(&String::from(CRL_CA)) && asked.contains(&String::from(CRL_INTER)), "{asked:?}");
    // Bez odvolani projde.
    let clean = Arc::new(Source {
        crls: vec![(CRL_CA, crl(&root, &[], 2099)), (CRL_INTER, crl(&inter, &[], 2099))],
        ..Default::default()
    });
    try_connect(&pki(&root, chain, key), Some(clean), Vec::new()).unwrap();
}

#[test]
fn onecrl_revoked_ca() {
    let root = ca("Root", None, None);
    let inter = ca("Inter OneCRL", Some(&root), None);
    let (leaf_cert, key) = leaf(&inter, 9, None);
    let c = Cert::parse(inter.cert.der()).unwrap();
    let q = CaQuery {
        issuer: c.issuer.to_vec(),
        serial: c.serial.to_vec(),
        subject: c.subject.to_vec(),
        spki_sha256: Sha256::digest(c.spki).into(),
    };
    // Protokol k revocd tam a zpet.
    assert_eq!(proto::decode_ca(&proto::encode_ca(&q)), Some(q.clone()));
    assert_eq!(proto::decode_ca(b"ca \x01"), None);
    let chain = vec![leaf_cert, inter.cert.der().clone()];
    let source = Arc::new(Source {
        revoked_cas: vec![q],
        ..Default::default()
    });
    expect_revoked(try_connect(&pki(&root, chain.clone(), key.clone_key()), Some(source), Vec::new()));
    try_connect(&pki(&root, chain, key), Some(Arc::new(Source::default())), Vec::new()).unwrap();
}

// --- OCSP ---

pub(super) fn tlv(tag: u8, content: &[u8]) -> Vec<u8> {
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

pub(super) fn seq(parts: &[Vec<u8>]) -> Vec<u8> {
    tlv(0x30, &parts.concat())
}

pub(super) fn gtime(s: &str) -> Vec<u8> {
    tlv(0x18, s.as_bytes())
}

#[derive(Clone, Copy)]
enum OcspStatus {
    Good,
    Revoked,
}

/// Odpoved OCSP pro `leaf_der` od `issuer`, podepsana klicem `signer`
/// (PKCS#8 P-256).
fn ocsp_response(issuer_der: &[u8], leaf_der: &[u8], status: OcspStatus, next: &str, signer: &[u8]) -> Vec<u8> {
    let issuer = Cert::parse(issuer_der).unwrap();
    let leaf = Cert::parse(leaf_der).unwrap();
    let cert_id = seq(&[
        seq(&[tlv(0x06, &oid(&[1, 3, 14, 3, 2, 26])), vec![0x05, 0x00]]),
        tlv(0x04, &Sha1::digest(issuer.subject)),
        tlv(0x04, &Sha1::digest(issuer.public_key)),
        tlv(0x02, leaf.serial),
    ]);
    let cert_status = match status {
        OcspStatus::Good => vec![0x80, 0x00],
        OcspStatus::Revoked => tlv(0xA1, &gtime("20210101000000Z")),
    };
    let single = seq(&[cert_id, cert_status, gtime("20200101000000Z"), tlv(0xA0, &gtime(next))]);
    let tbs = seq(&[tlv(0xA1, issuer.subject), gtime("20200101000000Z"), seq(&[single])]);
    let rng = ring::rand::SystemRandom::new();
    let kp = ring::signature::EcdsaKeyPair::from_pkcs8(&ring::signature::ECDSA_P256_SHA256_ASN1_SIGNING, signer, &rng).unwrap();
    let sig = kp.sign(&rng, &tbs).unwrap();
    let mut bits = vec![0u8];
    bits.extend_from_slice(sig.as_ref());
    let basic = seq(&[tbs, seq(&[tlv(0x06, &oid(&[1, 2, 840, 10045, 4, 3, 2]))]), tlv(0x03, &bits)]);
    seq(&[
        tlv(0x0A, &[0]),
        tlv(0xA0, &seq(&[tlv(0x06, &oid(&[1, 3, 6, 1, 5, 5, 7, 48, 1, 1])), tlv(0x04, &basic)])),
    ])
}

#[test]
fn ocsp_stapling() {
    let root = ca("Root OCSP", None, None);
    let (cert, key) = leaf(&root, 55, None);
    let p = pki(&root, vec![cert.clone()], key);
    let ca_key = root.key.serialize_der();
    let ok = ocsp_response(root.cert.der(), &cert, OcspStatus::Good, "20990101000000Z", &ca_key);
    let revoked = ocsp_response(root.cert.der(), &cert, OcspStatus::Revoked, "20990101000000Z", &ca_key);
    let stale = ocsp_response(root.cert.der(), &cert, OcspStatus::Revoked, "20210101000000Z", &ca_key);
    let other_key = KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).unwrap().serialize_der();
    let forged = ocsp_response(root.cert.der(), &cert, OcspStatus::Good, "20990101000000Z", &other_key);

    // Primo (bez TLS).
    let leaf_c = Cert::parse(&cert).unwrap();
    let issuer_c = Cert::parse(root.cert.der()).unwrap();
    let now = 1_800_000_000;
    assert_eq!(crate::ocsp::check(&ok, &leaf_c, &issuer_c.key(), now), Ok(crate::ocsp::Status::Good));
    assert_eq!(crate::ocsp::check(&revoked, &leaf_c, &issuer_c.key(), now), Ok(crate::ocsp::Status::Revoked));
    assert_eq!(crate::ocsp::check(&stale, &leaf_c, &issuer_c.key(), now), Ok(crate::ocsp::Status::Unknown));
    assert_eq!(crate::ocsp::check(&forged, &leaf_c, &issuer_c.key(), now), Err(crate::ocsp::OcspError::BadSignature));
    assert_eq!(crate::ocsp::check(b"\x30\x03\x0a\x01\x03", &leaf_c, &issuer_c.key(), now), Err(crate::ocsp::OcspError::NotSuccessful));

    // V handshaku: dobra projde i bez zdroje, odvolana a podvrzena ne.
    try_connect(&p, None, ok).unwrap();
    expect_revoked(try_connect(&p, None, revoked));
    try_connect(&p, None, stale).unwrap();
    match try_connect(&p, None, forged) {
        Err(Error::Certificate(m)) if m.contains("InvalidOcspResponse") => {}
        other => panic!("podvrzena OCSP: {other:?}"),
    }
}

#[test]
fn x509_fields() {
    let root = ca("Pole", None, Some(CRL_CA));
    let c = Cert::parse(root.cert.der()).unwrap();
    assert_eq!(c.issuer, c.subject);
    assert_eq!(c.crl_urls(), vec![String::from(CRL_CA)]);
    assert!(c.not_before < c.not_after);
    let crl_der = crl(&root, &[1], 2099);
    let info = x509::crl_info(&crl_der).unwrap();
    assert_eq!(info.issuer, c.subject);
    assert!(info.next_update.unwrap() > info.this_update);
    assert!(Cert::parse(b"\x30\x00").is_err());
    assert!(Cert::parse(&root.cert.der()[..50]).is_err());
}
