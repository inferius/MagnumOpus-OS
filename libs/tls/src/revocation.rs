//! Kontrola odvolani certifikatu serveru.
//!
//! Poradi kontrol (po overeni retezu, jmena a platnosti webpki):
//! 1. **OCSP stapling**: odpoved od serveru v handshaku ([`crate::ocsp`]).
//!    `revoked` = odmitnuti, `good` = koncovy certifikat je overeny, spatny
//!    podpis = odmitnuti (podvrzena odpoved).
//! 2. **Odvolane CA** (OneCRL od Mozilly): dotaz na zdroj pro kazdy
//!    certifikat retezu krome korene.
//! 3. **CRL** z distribucnich bodu certifikatu (`http://...`): zdroj je
//!    vrati (stazene a v mezipameti), webpki overi jejich podpis a hleda
//!    v nich cisla certifikatu. Propadle CRL se zahodi.
//!
//! Stav, ktery nejde zjistit (zdroj nebezi, CRL nejde stahnout, server bez
//! staplingu), spojeni **nezastavi** - stejne jako v prohlizecich. Kdyz
//! zdroj odvolani chybi uplne, plati jen OCSP stapling.
//!
//! Zdroj je v systemu sluzba `revocd` ([`ChannelSource`] s feature
//! `libos`); protokol je v [`proto`].

use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;

use pki_types::{CertificateDer, CertificateRevocationListDer, ServerName, UnixTime};
use rustls::client::WebPkiServerVerifier;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::CryptoProvider;
use rustls::{CertificateError, DigitallySignedStruct, Error, RootCertStore, SignatureScheme};
use sha2::{Digest, Sha256};

use crate::ocsp;
use crate::provider::ALGORITHMS;
use crate::x509::{self, Cert, Key};

/// Dotaz na odvolanou CA (polozky OneCRL: vystavitel + cislo, nebo subjekt
/// + otisk klice).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaQuery {
    /// Cela polozka Name vystavitele.
    pub issuer: Vec<u8>,
    /// Obsah INTEGER cisla certifikatu.
    pub serial: Vec<u8>,
    /// Cela polozka Name subjektu.
    pub subject: Vec<u8>,
    /// SHA-256 cele polozky SubjectPublicKeyInfo.
    pub spki_sha256: [u8; 32],
}

/// Odkud brat informace o odvolani.
pub trait RevocationSource: core::fmt::Debug + Send + Sync {
    /// CRL (DER) z adresy `url`; `None` = neni k dispozici.
    fn crl(&self, url: &str) -> Option<Vec<u8>>;
    /// Je certifikat CA na seznamu odvolanych (OneCRL)?
    fn ca_revoked(&self, query: &CaQuery) -> bool;
}

/// Overovac serveru: webpki a k tomu kontrola odvolani.
#[derive(Debug)]
pub(crate) struct Verifier {
    roots: Arc<RootCertStore>,
    provider: Arc<CryptoProvider>,
    plain: Arc<WebPkiServerVerifier>,
    source: Option<Arc<dyn RevocationSource>>,
}

fn revoked() -> Error {
    Error::InvalidCertificate(CertificateError::Revoked)
}

impl Verifier {
    pub(crate) fn new(
        roots: Arc<RootCertStore>,
        provider: Arc<CryptoProvider>,
        source: Option<Arc<dyn RevocationSource>>,
    ) -> Result<Self, Error> {
        let plain = WebPkiServerVerifier::builder_with_provider(roots.clone(), provider.clone())
            .build()
            .map_err(|e| Error::General(alloc::format!("{e}")))?;
        Ok(Self {
            roots,
            provider,
            plain,
            source,
        })
    }

    /// Vydavatel certifikatu `cert`: z retezu, nebo korenova CA.
    fn issuer_key<'a>(&'a self, cert: &Cert<'_>, chain: &'a [Cert<'a>], anchor_subject: &'a mut Vec<u8>) -> Option<Key<'a>> {
        if let Some(c) = chain.iter().find(|c| c.subject == cert.issuer) {
            return Some(c.key());
        }
        for a in self.roots.roots.iter() {
            // Jmeno kotvy je obsah Name bez vnejsi znacky SEQUENCE.
            let mut raw = Vec::with_capacity(a.subject.as_ref().len() + 4);
            raw.push(x509::tag::SEQUENCE);
            push_len(&mut raw, a.subject.as_ref().len());
            raw.extend_from_slice(a.subject.as_ref());
            if raw == cert.issuer {
                let (alg, key) = x509::spki_parts(a.subject_public_key_info.as_ref()).ok()?;
                *anchor_subject = raw;
                let subject: &'a Vec<u8> = anchor_subject;
                return Some(Key {
                    subject,
                    spki_alg: alg,
                    public_key: key,
                });
            }
        }
        None
    }
}

fn push_len(out: &mut Vec<u8>, len: usize) {
    if len < 0x80 {
        out.push(len as u8);
    } else {
        let bytes = (len as u32).to_be_bytes();
        let skip = bytes.iter().take_while(|&&b| b == 0).count();
        out.push(0x80 | (4 - skip) as u8);
        out.extend_from_slice(&bytes[skip..]);
    }
}

impl ServerCertVerifier for Verifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        server_name: &ServerName<'_>,
        ocsp_response: &[u8],
        now: UnixTime,
    ) -> Result<ServerCertVerified, Error> {
        // Retez, jmeno, platnost (bez odvolani).
        self.plain.verify_server_cert(end_entity, intermediates, server_name, ocsp_response, now)?;
        let now_s = now.as_secs() as i64;
        // Certifikaty, ktere nas parser neprecte, se jen neprovereji.
        let Ok(leaf) = Cert::parse(end_entity.as_ref()) else {
            return Ok(ServerCertVerified::assertion());
        };
        let chain: Vec<Cert<'_>> = intermediates.iter().filter_map(|c| Cert::parse(c.as_ref()).ok()).collect();

        // 1. OCSP stapling.
        let mut leaf_known = false;
        if !ocsp_response.is_empty() {
            let mut anchor = Vec::new();
            if let Some(issuer) = self.issuer_key(&leaf, &chain, &mut anchor) {
                match ocsp::check(ocsp_response, &leaf, &issuer, now_s) {
                    Ok(ocsp::Status::Revoked) => return Err(revoked()),
                    Ok(ocsp::Status::Good) => leaf_known = true,
                    Ok(ocsp::Status::Unknown) | Err(ocsp::OcspError::NotSuccessful) => {}
                    Err(_) => return Err(Error::InvalidCertificate(CertificateError::InvalidOcspResponse)),
                }
            }
        }
        let Some(source) = &self.source else {
            return Ok(ServerCertVerified::assertion());
        };

        // 2. Odvolane CA (OneCRL): mezilehle CA i koncovy certifikat.
        for c in core::iter::once(&leaf).chain(chain.iter()) {
            let q = CaQuery {
                issuer: c.issuer.to_vec(),
                serial: c.serial.to_vec(),
                subject: c.subject.to_vec(),
                spki_sha256: Sha256::digest(c.spki).into(),
            };
            if source.ca_revoked(&q) {
                return Err(revoked());
            }
        }

        // 3. CRL.
        let mut urls: Vec<String> = Vec::new();
        if !leaf_known {
            urls.extend(leaf.crl_urls());
        }
        for c in &chain {
            urls.extend(c.crl_urls());
        }
        urls.sort();
        urls.dedup();
        let crls: Vec<CertificateRevocationListDer<'static>> = urls
            .iter()
            .filter_map(|u| source.crl(u))
            .filter(|der| {
                // Propadla CRL = stav nezname (radeji nez odmitnuti).
                x509::crl_info(der).is_ok_and(|i| i.next_update.is_none_or(|n| n >= now_s))
            })
            .map(CertificateRevocationListDer::from)
            .collect();
        if crls.is_empty() {
            return Ok(ServerCertVerified::assertion());
        }
        let with_crls = WebPkiServerVerifier::builder_with_provider(self.roots.clone(), self.provider.clone())
            .with_crls(crls)
            .allow_unknown_revocation_status()
            .build()
            .map_err(|e| Error::General(alloc::format!("{e}")))?;
        with_crls.verify_server_cert(end_entity, intermediates, server_name, ocsp_response, now)
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        // Klient nabizi jen TLS 1.3.
        Err(Error::PeerIncompatible(rustls::PeerIncompatible::Tls12NotOffered))
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        rustls::crypto::verify_tls13_signature(message, cert, dss, &ALGORITHMS)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        ALGORITHMS.supported_schemes()
    }
}

/// Zpravy mezi programem a sluzbou `revocd` (kanal od init).
///
/// - `crl` + adresa (UTF-8) -> `ok` + CRL (DER), nebo `none`,
/// - `ca` + 4 pole s delkou u32 LE (vystavitel, cislo, subjekt, otisk
///   SHA-256 klice) -> `revoked` nebo `ok`.
pub mod proto {
    use alloc::vec::Vec;

    use super::CaQuery;

    pub const CRL: &[u8] = b"crl ";
    pub const CA: &[u8] = b"ca ";

    fn put(out: &mut Vec<u8>, field: &[u8]) {
        out.extend_from_slice(&(field.len() as u32).to_le_bytes());
        out.extend_from_slice(field);
    }

    fn take<'a>(data: &mut &'a [u8]) -> Option<&'a [u8]> {
        let len = u32::from_le_bytes(data.get(..4)?.try_into().ok()?) as usize;
        let field = data.get(4..4 + len)?;
        *data = &data[4 + len..];
        Some(field)
    }

    pub fn encode_ca(q: &CaQuery) -> Vec<u8> {
        let mut out = Vec::from(CA);
        put(&mut out, &q.issuer);
        put(&mut out, &q.serial);
        put(&mut out, &q.subject);
        put(&mut out, &q.spki_sha256);
        out
    }

    pub fn decode_ca(msg: &[u8]) -> Option<CaQuery> {
        let mut d = msg.strip_prefix(CA)?;
        let issuer = take(&mut d)?.to_vec();
        let serial = take(&mut d)?.to_vec();
        let subject = take(&mut d)?.to_vec();
        let spki_sha256 = take(&mut d)?.try_into().ok()?;
        d.is_empty().then_some(CaQuery { issuer, serial, subject, spki_sha256 })
    }
}

/// Zdroj odvolani pres kanal ke sluzbe `revocd`.
#[cfg(feature = "libos")]
pub struct ChannelSource {
    channel: spin::Mutex<libos::Channel>,
    timeout: core::time::Duration,
}

#[cfg(feature = "libos")]
impl core::fmt::Debug for ChannelSource {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("ChannelSource")
    }
}

#[cfg(feature = "libos")]
impl ChannelSource {
    /// `timeout` = jak dlouho cekat na odpoved (stazeni CRL).
    pub fn new(channel: libos::Channel, timeout: core::time::Duration) -> Self {
        Self {
            channel: spin::Mutex::new(channel),
            timeout,
        }
    }

    fn call(&self, msg: &[u8]) -> Option<Vec<u8>> {
        let ch = self.channel.lock();
        // Starsi odpovedi (po vyprseni predchoziho dotazu) zahodit.
        while ch.try_read().is_ok() {}
        ch.write(msg, Vec::new()).ok()?;
        let deadline = libos::time::deadline_after(self.timeout);
        ch.read(Some(deadline)).ok().map(|m| m.data)
    }
}

#[cfg(feature = "libos")]
impl RevocationSource for ChannelSource {
    fn crl(&self, url: &str) -> Option<Vec<u8>> {
        let mut msg = Vec::from(proto::CRL);
        msg.extend_from_slice(url.as_bytes());
        let reply = self.call(&msg)?;
        reply.strip_prefix(b"ok").map(|d| d.to_vec())
    }

    fn ca_revoked(&self, query: &CaQuery) -> bool {
        self.call(&proto::encode_ca(query)).is_some_and(|r| r == b"revoked")
    }
}
