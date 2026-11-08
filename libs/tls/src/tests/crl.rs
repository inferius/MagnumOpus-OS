//! CRL "jen certifikaty CA" (Issuing Distribution Point s
//! `onlyContainsCACerts`), ktere webpki neprecte.
//!
//! Skutecne vzorky v `testdata/letsencrypt/` (stazene 2026-11-07):
//! CRL korenu ISRG Root X1 a Root YR, retez `nts.netnod.se` (YR2 -> Root
//! YR krizove podepsany X1) a kořen ISRG Root X1. Pred opravou tento retez
//! s temito CRL shodil TLS chybou "provided CRL could not be parsed"
//! (ntpd pak NTS-KE se skutecnymi servery opakovane selhaval).

use std::sync::Arc;
use std::time::Duration;
use std::vec;
use std::vec::Vec;

use pki_types::{CertificateDer, CertificateRevocationListDer, ServerName, UnixTime};
use rustls::RootCertStore;
use rustls::client::WebPkiServerVerifier;
use rustls::client::danger::ServerCertVerifier;

use super::PLATFORM;
use super::revocation::{CRL_CA, CRL_INTER, Ca, Source, ca, expect_revoked, gtime, leaf, pki, seq, tlv, try_connect};
use crate::crl::{self, CrlError};
use crate::revocation::{RevocationSource, Verifier};
use crate::x509::{Cert, oid};

const X1_CRL: &[u8] = include_bytes!("../../testdata/letsencrypt/isrg-root-x1.crl");
const YR_CRL: &[u8] = include_bytes!("../../testdata/letsencrypt/root-yr.crl");
const X1: &[u8] = include_bytes!("../../testdata/letsencrypt/isrg-root-x1.der");
const ROOT_YR: &[u8] = include_bytes!("../../testdata/letsencrypt/root-yr-by-x1.der");
const YR2: &[u8] = include_bytes!("../../testdata/letsencrypt/yr2.der");
const LEAF: &[u8] = include_bytes!("../../testdata/letsencrypt/nts-netnod-se.der");

/// 2026-11-08 00:00 UTC: vsechny vzorky jsou platne.
fn sample_time() -> UnixTime {
    UnixTime::since_unix_epoch(Duration::from_secs(1_794_096_000))
}

fn provider() -> Arc<rustls::crypto::CryptoProvider> {
    Arc::new(crate::provider::provider(&PLATFORM))
}

fn x1_roots() -> Arc<RootCertStore> {
    let mut roots = RootCertStore::empty();
    roots.add(CertificateDer::from(X1.to_vec())).unwrap();
    Arc::new(roots)
}

#[test]
fn real_ca_only_crls() {
    for (name, der, signer) in [("ISRG Root X1", X1_CRL, X1), ("Root YR", YR_CRL, ROOT_YR)] {
        // webpki je odmitne (proto vlastni cteni).
        let err = WebPkiServerVerifier::builder_with_provider(x1_roots(), provider())
            .with_crls([CertificateRevocationListDer::from(der.to_vec())])
            .build()
            .unwrap_err();
        assert!(std::format!("{err:?}").contains("UnsupportedCrlIssuingDistributionPoint"), "{name}: {err:?}");
        let c = crl::parse_ca_only(der).unwrap_or_else(|e| panic!("{name}: {e:?}"));
        let signer = Cert::parse(signer).unwrap();
        assert_eq!(c.issuer, signer.subject, "{name}: vydavatel");
        assert!(crate::ocsp::verify_signature(&signer.key(), c.sig_alg, c.tbs, c.signature), "{name}: podpis");
        assert!(c.next_update.is_some_and(|n| n > c.this_update), "{name}: platnost");
        // Poskozeny podpis neprojde.
        let mut bad = c.signature.to_vec();
        bad[10] ^= 1;
        assert!(!crate::ocsp::verify_signature(&signer.key(), c.sig_alg, c.tbs, &bad), "{name}: zmeneny podpis");
    }
    // CRL X1 obsahuje odvolane mezilehle CA, Root YR zadnou.
    assert!(!crl::parse_ca_only(X1_CRL).unwrap().revoked.is_empty());
    assert!(crl::parse_ca_only(YR_CRL).unwrap().revoked.is_empty());
    // Bezna CRL (bez IDP) neni "jen CA".
    let root = ca("Root", None, None);
    let plain = super::revocation::crl(&root, &[5], 2099);
    assert_eq!(crl::parse_ca_only(&plain).unwrap_err(), CrlError::NotCaOnly);
    assert_eq!(crl::parse_ca_only(b"neni CRL").unwrap_err(), CrlError::Malformed);
}

/// Zdroj se skutecnymi CRL korenu (CRL serveru YR2 neni k dispozici).
fn real_source() -> Arc<Source> {
    Arc::new(Source {
        crls: vec![("http://x1.c.lencr.org/", X1_CRL.to_vec()), ("http://yr.c.lencr.org/", YR_CRL.to_vec())],
        ..Default::default()
    })
}

#[test]
fn real_chain_with_ca_only_crls() {
    let source = real_source();
    let v = Verifier::new(x1_roots(), provider(), Some(source.clone() as Arc<dyn RevocationSource>)).unwrap();
    let inter = [CertificateDer::from(YR2), CertificateDer::from(ROOT_YR)];
    let name = ServerName::try_from("nts.netnod.se").unwrap();
    let r = v.verify_server_cert(&CertificateDer::from(LEAF), &inter, &name, &[], sample_time());
    assert!(r.is_ok(), "{r:?}");
    let asked = source.asked.lock().unwrap().clone();
    for u in ["http://x1.c.lencr.org/", "http://yr.c.lencr.org/", "http://yr2.c.lencr.org/62.crl"] {
        assert!(asked.iter().any(|a| a == u), "{u} v {asked:?}");
    }
    // Propadla CRL korene se nepouzije (stav nezname), spojeni projde.
    let later = UnixTime::since_unix_epoch(Duration::from_secs(1_794_096_000 + 400 * 86_400));
    let r = v.verify_server_cert(&CertificateDer::from(LEAF), &inter, &name, &[], later);
    // Za 400 dni uz neplati certifikat serveru - to hlasi webpki, ne CRL.
    assert!(!std::format!("{r:?}").contains("CRL"), "{r:?}");
}

/// CRL "jen CA" jako od Let's Encrypt: IDP bez distribucniho bodu, jen
/// `onlyContainsCACerts` (takovou webpki odmitne; rcgen IDP bez adresy
/// neumi, proto rucne). Podpis ECDSA P-256 klicem `issuer`.
fn ca_only_crl(issuer: &Ca, revoked: &[Vec<u8>]) -> Vec<u8> {
    let ecdsa_sha256 = seq(&[tlv(0x06, &oid(&[1, 2, 840, 10045, 4, 3, 2]))]);
    let name = Cert::parse(issuer.cert.der()).unwrap().subject.to_vec();
    let entries: Vec<Vec<u8>> = revoked.iter().map(|s| seq(&[tlv(0x02, s), gtime("20210101000000Z")])).collect();
    let idp = seq(&[vec![0x82, 0x01, 0xFF]]);
    let ext = seq(&[tlv(0x06, &oid(&[2, 5, 29, 28])), vec![0x01, 0x01, 0xFF], tlv(0x04, &idp)]);
    let mut tbs_parts = vec![
        tlv(0x02, &[1]),
        ecdsa_sha256.clone(),
        name,
        gtime("20200101000000Z"),
        gtime("20990101000000Z"),
    ];
    if !entries.is_empty() {
        tbs_parts.push(seq(&entries));
    }
    tbs_parts.push(tlv(0xA0, &seq(&[ext])));
    let tbs = seq(&tbs_parts);
    let rng = ring::rand::SystemRandom::new();
    let kp = ring::signature::EcdsaKeyPair::from_pkcs8(
        &ring::signature::ECDSA_P256_SHA256_ASN1_SIGNING,
        &issuer.key.serialize_der(),
        &rng,
    )
    .unwrap();
    let sig = kp.sign(&rng, &tbs).unwrap();
    let mut bits = vec![0u8];
    bits.extend_from_slice(sig.as_ref());
    seq(&[tbs, ecdsa_sha256, tlv(0x03, &bits)])
}

#[test]
fn ca_only_crl_revokes_intermediate() {
    let root = ca("Root CA-only", None, None);
    let inter = ca("Inter CA-only", Some(&root), Some(CRL_CA));
    let (leaf_cert, key) = leaf(&inter, 9, Some(CRL_INTER));
    let serial = Cert::parse(inter.cert.der()).unwrap().serial.to_vec();
    let chain = vec![leaf_cert, inter.cert.der().clone()];
    let user_crl = super::revocation::crl(&inter, &[], 2099);
    // webpki ji neprecte: overi ji vlastni kod (crate::crl).
    let probe = ca_only_crl(&root, &[serial.clone()]);
    assert!(crl::parse_ca_only(&probe).is_ok());
    assert!(
        WebPkiServerVerifier::builder_with_provider(x1_roots(), provider())
            .with_crls([CertificateRevocationListDer::from(probe)])
            .build()
            .is_err()
    );
    // Odvolana mezilehla CA v CRL "jen CA" korene.
    let source = Arc::new(Source {
        crls: vec![(CRL_CA, ca_only_crl(&root, &[serial.clone()])), (CRL_INTER, user_crl.clone())],
        ..Default::default()
    });
    expect_revoked(try_connect(&pki(&root, chain.clone(), key.clone_key()), Some(source), Vec::new()));
    // Bez odvolani projde.
    let clean = Arc::new(Source {
        crls: vec![(CRL_CA, ca_only_crl(&root, &[])), (CRL_INTER, user_crl.clone())],
        ..Default::default()
    });
    try_connect(&pki(&root, chain.clone(), key.clone_key()), Some(clean), Vec::new()).unwrap();
    // Podvrzena CRL (podepsana cizi CA se stejnym jmenem) nic neodvola
    // a spojeni neshodi.
    let other = ca("Root CA-only", None, None);
    let forged = Arc::new(Source {
        crls: vec![(CRL_CA, ca_only_crl(&other, &[serial])), (CRL_INTER, user_crl)],
        ..Default::default()
    });
    try_connect(&pki(&root, chain, key), Some(forged), Vec::new()).unwrap();
}
