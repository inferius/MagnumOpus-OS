//! Mezipamet CRL a OneCRL na FS (`/crl` v relaci revocd =
//! `/system/config/certs/crl`).
//!
//! - CRL: soubor `<prvnich 32 hex SHA-256 adresy>.crl`; plati do
//!   `nextUpdate`, CRL bez nej `crl_default` od stazeni (cas zmeny
//!   souboru). V pameti nejvys [`MEM_ENTRIES`] poslednich.
//! - Neuspesne stazeni: adresa se `retry` znovu nezkousi (jinak by kazde
//!   spojeni cekalo na vyprseni).
//! - OneCRL: `onecrl.json` (posledni platna verze), obnova `onecrl_refresh`.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use fsproto::FsClient;
use fsproto::v2::{access, share};
use libos::{Error, Result};
use sha2::{Digest, Sha256};

pub const MEM_ENTRIES: usize = 64;

pub struct Entry {
    pub url: String,
    pub der: Vec<u8>,
    /// Plati do (realny cas, sekundy).
    pub valid_until: i64,
}

pub fn crl_path(url: &str) -> String {
    let h = Sha256::digest(url.as_bytes());
    let hex: String = h[..16].iter().map(|b| format!("{b:02x}")).collect();
    format!("/crl/{hex}.crl")
}

/// Cely soubor (`None` = neexistuje).
pub fn load(fs: &FsClient, path: &str) -> Result<Option<(Vec<u8>, i64)>> {
    let f = match fs.open(path, access::READ, share::ALL, 0) {
        Ok(f) => f,
        Err(Error::NotFound) => return Ok(None),
        Err(e) => return Err(e),
    };
    let mut data = Vec::new();
    let mut result = Ok(());
    while (data.len() as u64) < f.stat.size {
        match fs.read_h(f.handle, data.len() as u64, 32 * 1024) {
            Ok(chunk) if !chunk.is_empty() => data.extend_from_slice(&chunk),
            Ok(_) => break,
            Err(e) => {
                result = Err(e);
                break;
            }
        }
    }
    let _ = fs.close(f.handle);
    result.map(|_| Some((data, f.stat.mtime.secs)))
}

/// Atomicke ulozeni (docasny soubor a prejmenovani).
pub fn save(fs: &FsClient, path: &str, data: &[u8]) -> Result<()> {
    let expected = match fs.open(path, access::READ, share::ALL, 0) {
        Ok(f) => {
            let _ = fs.close(f.handle);
            Some(f.revision)
        }
        Err(Error::NotFound) => None,
        Err(e) => return Err(e),
    };
    fs.save_if(path, expected, data).map(|_| ())
}

/// Doba platnosti CRL: `nextUpdate`, jinak `fetched + default`.
pub fn valid_until(der: &[u8], fetched: i64, default: i64) -> Option<i64> {
    let info = tls::x509::crl_info(der).ok()?;
    Some(info.next_update.unwrap_or(fetched + default))
}
