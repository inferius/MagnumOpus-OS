//! Export klicu z TLS 1.3 (RFC 8446 7.5, RFC 5705) - potrebuje ho NTS
//! (RFC 8915: klice pro NTP z handshaku NTS-KE).
//!
//! rustls bez `std` (`UnbufferedClientConnection`) export nenabizi. Tajemstvi
//! `exporter_master_secret` ale preda rozhrani `KeyLog` (stitek
//! `EXPORTER_SECRET`, s nahodou klienta z ClientHello). Klient si ho tak
//! zapamatuje podle `client_random` a `TlsStream` z nej spocita:
//!
//! ```text
//! TLS-Exporter(label, context, L) =
//!     HKDF-Expand-Label(Derive-Secret(exporter_master_secret, label, ""),
//!                       "exporter", Hash(context), L)
//! ```
//!
//! Zadna jina tajemstvi se neukladaji (`will_log` jen pro `EXPORTER_SECRET`).

use alloc::vec::Vec;

use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256, Sha384};

/// Kolik tajemstvi drzet (souběžné handshaky jednoho klienta).
const KEEP: usize = 16;

#[derive(Debug, Default)]
pub(crate) struct ExporterLog {
    secrets: spin::Mutex<Vec<([u8; 32], Vec<u8>)>>,
}

impl rustls::KeyLog for ExporterLog {
    fn log(&self, label: &str, client_random: &[u8], secret: &[u8]) {
        let Ok(random) = <[u8; 32]>::try_from(client_random) else {
            return;
        };
        if label != "EXPORTER_SECRET" {
            return;
        }
        let mut s = self.secrets.lock();
        s.retain(|(r, _)| *r != random);
        if s.len() >= KEEP {
            s.remove(0);
        }
        s.push((random, secret.to_vec()));
    }

    fn will_log(&self, label: &str) -> bool {
        label == "EXPORTER_SECRET"
    }
}

impl ExporterLog {
    /// Vyjme tajemstvi spojeni s nahodou klienta `random`.
    pub(crate) fn take(&self, random: &[u8; 32]) -> Option<Vec<u8>> {
        let mut s = self.secrets.lock();
        let i = s.iter().position(|(r, _)| r == random)?;
        Some(s.remove(i).1)
    }
}

/// Hash sady (podle delky tajemstvi: 32 B = SHA-256, 48 B = SHA-384).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Hash {
    Sha256,
    Sha384,
}

impl Hash {
    pub(crate) fn for_secret(secret: &[u8]) -> Option<Self> {
        match secret.len() {
            32 => Some(Self::Sha256),
            48 => Some(Self::Sha384),
            _ => None,
        }
    }

    fn len(self) -> usize {
        match self {
            Self::Sha256 => 32,
            Self::Sha384 => 48,
        }
    }

    fn digest(self, data: &[u8]) -> Vec<u8> {
        match self {
            Self::Sha256 => Sha256::digest(data).to_vec(),
            Self::Sha384 => Sha384::digest(data).to_vec(),
        }
    }

    fn hmac(self, key: &[u8], parts: &[&[u8]]) -> Vec<u8> {
        match self {
            Self::Sha256 => {
                let mut m = <Hmac<Sha256> as Mac>::new_from_slice(key).expect("HMAC prijme kazdy klic");
                for p in parts {
                    m.update(p);
                }
                m.finalize().into_bytes().to_vec()
            }
            Self::Sha384 => {
                let mut m = <Hmac<Sha384> as Mac>::new_from_slice(key).expect("HMAC prijme kazdy klic");
                for p in parts {
                    m.update(p);
                }
                m.finalize().into_bytes().to_vec()
            }
        }
    }
}

/// HKDF-Expand (RFC 5869) s PRK `prk`.
fn hkdf_expand(hash: Hash, prk: &[u8], info: &[u8], len: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(len);
    let mut t: Vec<u8> = Vec::new();
    let mut counter = 1u8;
    while out.len() < len {
        t = hash.hmac(prk, &[&t, info, &[counter]]);
        out.extend_from_slice(&t);
        counter += 1;
    }
    out.truncate(len);
    out
}

/// HKDF-Expand-Label (RFC 8446 7.1).
pub(crate) fn expand_label(hash: Hash, secret: &[u8], label: &[u8], context: &[u8], len: usize) -> Vec<u8> {
    let mut info = Vec::with_capacity(4 + 6 + label.len() + context.len());
    info.extend_from_slice(&(len as u16).to_be_bytes());
    info.push((6 + label.len()) as u8);
    info.extend_from_slice(b"tls13 ");
    info.extend_from_slice(label);
    info.push(context.len() as u8);
    info.extend_from_slice(context);
    hkdf_expand(hash, secret, &info, len)
}

/// TLS-Exporter (RFC 8446 7.5) z `exporter_master_secret`.
pub(crate) fn export(exporter_master: &[u8], label: &[u8], context: &[u8], out: &mut [u8]) -> bool {
    let Some(hash) = Hash::for_secret(exporter_master) else {
        return false;
    };
    if label.len() > 249 || context.len() > 255 || out.is_empty() || out.len() > 255 * hash.len() {
        return false;
    }
    let derived = expand_label(hash, exporter_master, label, &hash.digest(b""), hash.len());
    let key = expand_label(hash, &derived, b"exporter", &hash.digest(context), out.len());
    out.copy_from_slice(&key);
    true
}
