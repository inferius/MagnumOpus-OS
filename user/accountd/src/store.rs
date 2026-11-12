//! DB uctu na FS (relace accountd vidi jen `/db` = `/system/accounts`).
//!
//! - `/db/users/<uid>`: zaznam uctu (`accounts::record`),
//! - `/db/meta`: dalsi volna cisla,
//! - `/db/device.key`: klic zarizeni (32 B), dokud neni TPM.
//!
//! Zapis je atomicky (podminene ulozeni `save_if`: docasny soubor
//! a prejmenovani) a nasleduje `fsync` - pocitadlo neuspesnych pokusu
//! musi prezit vypadek proudu.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use accounts::{Account, Meta, Store};
use fsproto::FsClient;
use fsproto::v2::{access, share};
use libos::{Error, Result};

pub const USERS: &str = "/db/users";
const META: &str = "/db/meta";
const DEVICE_KEY: &str = "/db/device.key";

pub struct FsStore {
    pub fs: FsClient,
}

fn load(fs: &FsClient, path: &str) -> Result<Option<Vec<u8>>> {
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
    result.map(|_| Some(data))
}

fn save(fs: &FsClient, path: &str, data: &[u8]) -> Result<()> {
    let expected = match fs.open(path, access::READ, share::ALL, 0) {
        Ok(f) => {
            let _ = fs.close(f.handle);
            Some(f.revision)
        }
        Err(Error::NotFound) => None,
        Err(e) => return Err(e),
    };
    fs.save_if(path, expected, data)?;
    fs.fsync()
}

impl FsStore {
    pub fn new(fs: FsClient) -> Result<Self> {
        match fs.create(USERS, fsproto::kind::DIR) {
            Ok(()) | Err(Error::AlreadyExists) => {}
            Err(e) => return Err(e),
        }
        Ok(FsStore { fs })
    }

    /// Klic zarizeni; pri prvnim startu ho vytvori z `fresh`.
    pub fn device_key(&self, fresh: [u8; 32]) -> Result<[u8; 32]> {
        if let Some(mut b) = load(&self.fs, DEVICE_KEY)? {
            let r = <[u8; 32]>::try_from(b.as_slice()).map_err(|_| Error::Corrupted);
            zeroize::Zeroize::zeroize(&mut b);
            return r;
        }
        save(&self.fs, DEVICE_KEY, &fresh)?;
        Ok(fresh)
    }
}

const AUDIT: &str = "/db/audit";
/// Nejvetsi audit; starsi polovina se pak zahodi.
const AUDIT_MAX: usize = 256 * 1024;

impl FsStore {
    /// Prida radek do auditu spravy (`cas uid_zadal uid_potvrdil ukon jak`).
    pub fn append_audit(&mut self, line: &[u8]) -> Result<()> {
        let mut data = load(&self.fs, AUDIT)?.unwrap_or_default();
        data.extend_from_slice(line);
        if data.len() > AUDIT_MAX {
            let cut = data.len() - AUDIT_MAX / 2;
            let cut = data[cut..].iter().position(|b| *b == b'\n').map_or(cut, |p| cut + p + 1);
            data.drain(..cut);
        }
        save(&self.fs, AUDIT, &data)
    }
}

impl Store for FsStore {
    fn load(&mut self) -> Result<(Option<Meta>, Vec<core::result::Result<Account, (String, Error)>>)> {
        let meta = match load(&self.fs, META)? {
            Some(b) => Some(Meta::decode(&b)?),
            None => None,
        };
        let mut list = Vec::new();
        for e in self.fs.list(USERS)? {
            if e.name.starts_with('.') {
                continue;
            }
            let path = format!("{USERS}/{}", e.name);
            let r = match load(&self.fs, &path) {
                Ok(Some(b)) => Account::decode(&b).and_then(|a| {
                    // Jmeno souboru musi odpovidat cislu v zaznamu.
                    if format!("{}", a.uid) == e.name { Ok(a) } else { Err(Error::Corrupted) }
                }),
                Ok(None) => continue,
                Err(e) => Err(e),
            };
            list.push(r.map_err(|err| (e.name.clone(), err)));
        }
        Ok((meta, list))
    }

    fn save(&mut self, a: &Account) -> Result<()> {
        save(&self.fs, &format!("{USERS}/{}", a.uid), &a.encode())
    }

    fn save_meta(&mut self, m: &Meta) -> Result<()> {
        save(&self.fs, META, &m.encode())
    }

    fn remove(&mut self, uid: u64) -> Result<()> {
        self.fs.remove(&format!("{USERS}/{uid}"))?;
        self.fs.fsync()
    }
}
