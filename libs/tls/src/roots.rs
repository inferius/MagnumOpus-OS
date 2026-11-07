//! Korenove certifikaty (duveryhodne CA).
//!
//! Systemove uloziste je adresar na FS (`/system/config/certs`): soubory
//! `*.pem` (jeden nebo vic certifikatu) a `*.der` (jeden certifikat).
//! Vychozi sada je Mozilla (stejna jako ve Firefoxu), init ji pri prvnim
//! startu ulozi z initfs. Vlastni CA (firma, testy) = dalsi soubor.
//! Zneuzitou CA jde vyradit souborem `distrust.txt`: otisk SHA-256
//! certifikatu (hex) na radek, `#` = komentar - plati i pro CA z ostatnich
//! souboru, takze se vychozi sada nemusi upravovat.

use alloc::string::String;
use alloc::vec::Vec;

use pki_types::CertificateDer;
use sha2::{Digest, Sha256};

/// Korenove certifikaty pro overeni serveru.
pub struct Roots {
    store: rustls::RootCertStore,
    /// Kolik certifikatu se nepodarilo nacist (poskozene, nepodporovane).
    pub rejected: usize,
    /// Kolik bylo vyrazeno podle `distrust.txt`.
    pub distrusted: usize,
}

/// Otisk SHA-256 certifikatu (DER).
pub fn fingerprint(der: &[u8]) -> [u8; 32] {
    Sha256::digest(der).into()
}

fn hex_value(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

/// Otisky z `distrust.txt` (hex, s dvojteckami nebo bez).
pub fn parse_distrust(text: &str) -> Vec<[u8; 32]> {
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.split('#').next().unwrap_or("");
        let digits: Vec<u8> = line.bytes().filter(|b| b.is_ascii_hexdigit()).collect();
        if digits.len() != 64 {
            continue;
        }
        let mut fp = [0u8; 32];
        for (i, pair) in digits.chunks(2).enumerate() {
            fp[i] = hex_value(pair[0]).unwrap() << 4 | hex_value(pair[1]).unwrap();
        }
        out.push(fp);
    }
    out
}

fn base64_value(b: u8) -> Option<u8> {
    Some(match b {
        b'A'..=b'Z' => b - b'A',
        b'a'..=b'z' => b - b'a' + 26,
        b'0'..=b'9' => b - b'0' + 52,
        b'+' => 62,
        b'/' => 63,
        _ => return None,
    })
}

/// Dekoduje base64 (bez mezer; `=` na konci). `None` = neplatne.
pub fn base64_decode(text: &[u8]) -> Option<Vec<u8>> {
    let data: Vec<u8> = text.iter().copied().filter(|b| !b.is_ascii_whitespace()).collect();
    if data.len() % 4 != 0 {
        return None;
    }
    let mut out = Vec::with_capacity(data.len() / 4 * 3);
    for chunk in data.chunks(4) {
        let pad = chunk.iter().rev().take_while(|&&b| b == b'=').count();
        if pad > 2 {
            return None;
        }
        let mut v = 0u32;
        for &b in &chunk[..4 - pad] {
            v = v << 6 | base64_value(b)? as u32;
        }
        v <<= 6 * pad as u32;
        let bytes = [(v >> 16) as u8, (v >> 8) as u8, v as u8];
        out.extend_from_slice(&bytes[..3 - pad]);
    }
    Some(out)
}

const BEGIN: &str = "-----BEGIN CERTIFICATE-----";
const END: &str = "-----END CERTIFICATE-----";

/// Certifikaty z textu PEM (bloky `CERTIFICATE`, ostatni se preskoci).
/// Vraci certifikaty a pocet poskozenych bloku.
pub fn parse_pem(text: &str) -> (Vec<Vec<u8>>, usize) {
    let mut certs = Vec::new();
    let mut bad = 0;
    let mut rest = text;
    while let Some(start) = rest.find(BEGIN) {
        let after = &rest[start + BEGIN.len()..];
        let Some(end) = after.find(END) else {
            bad += 1;
            break;
        };
        match base64_decode(&after.as_bytes()[..end]) {
            Some(der) if !der.is_empty() => certs.push(der),
            _ => bad += 1,
        }
        rest = &after[end + END.len()..];
    }
    (certs, bad)
}

impl Roots {
    /// Prazdne uloziste (zadnemu serveru se neveri).
    pub fn empty() -> Self {
        Self {
            store: rustls::RootCertStore::empty(),
            rejected: 0,
            distrusted: 0,
        }
    }

    /// Nacte soubory uloziste: (jmeno, obsah). `*.pem`, `*.der`
    /// a `distrust.txt`; ostatni se preskoci.
    pub fn from_files<'a>(files: impl IntoIterator<Item = (&'a str, &'a [u8])>) -> Self {
        let mut ders: Vec<Vec<u8>> = Vec::new();
        let mut distrust = Vec::new();
        let mut roots = Self::empty();
        for (name, data) in files {
            let lower = String::from(name).to_ascii_lowercase();
            if lower.ends_with(".pem") || lower.ends_with(".crt") {
                match core::str::from_utf8(data) {
                    Ok(text) => {
                        let (c, bad) = parse_pem(text);
                        ders.extend(c);
                        roots.rejected += bad;
                    }
                    Err(_) => roots.rejected += 1,
                }
            } else if lower.ends_with(".der") || lower.ends_with(".cer") {
                ders.push(data.to_vec());
            } else if lower == "distrust.txt"
                && let Ok(text) = core::str::from_utf8(data)
            {
                distrust.extend(parse_distrust(text));
            }
        }
        for der in ders {
            if distrust.contains(&fingerprint(&der)) {
                roots.distrusted += 1;
                continue;
            }
            if roots.store.add(CertificateDer::from(der)).is_err() {
                roots.rejected += 1;
            }
        }
        roots
    }

    /// Pocet duveryhodnych CA.
    pub fn len(&self) -> usize {
        self.store.len()
    }

    pub fn is_empty(&self) -> bool {
        self.store.is_empty()
    }

    pub(crate) fn store(&self) -> rustls::RootCertStore {
        self.store.clone()
    }
}
