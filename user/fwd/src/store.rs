//! Ulozeni konfigurace na souborovy system.
//!
//! fwd dostane relaci FS s jedinou pripojenou cestou `/config` (adresar
//! konfigurace firewallu). Soubor `/config/firewall.toml` je konfigurace
//! psana clovekem i stav (rozhodnuti uzivatele) - je verzovany se
//! snapshotem FS. Ukladani je podminene revizi: kdyz soubor mezitim zmenil
//! nekdo jiny (uzivatel v editoru), fwd ho znovu nacte, rozhodnuti prida
//! k nove verzi a ulozi znovu - cizi upravy se neztrati.

use alloc::vec::Vec;

use fsproto::FsClient;
use fsproto::v2::{Revision, access, share};
use libos::{Error, Result};

pub const PATH: &str = "/config/firewall.toml";

pub struct Store {
    fs: Option<FsClient>,
    rev: Option<Revision>,
}

impl Store {
    pub fn new(fs: Option<FsClient>) -> Self {
        Self { fs, rev: None }
    }

    pub fn available(&self) -> bool {
        self.fs.is_some()
    }

    /// Nacte soubor (a zapamatuje si jeho revizi). `NOT_FOUND` = neni.
    pub fn load(&mut self) -> Result<Vec<u8>> {
        let fs = self.fs.as_ref().ok_or(Error::NotFound)?;
        let f = fs.open(PATH, access::READ, share::ALL, 0)?;
        let mut data = Vec::new();
        let size = f.stat.size;
        let mut off = 0u64;
        let mut result = Ok(());
        while off < size {
            match fs.read_h(f.handle, off, (size - off).min(32 * 1024) as usize) {
                Ok(chunk) if !chunk.is_empty() => {
                    off += chunk.len() as u64;
                    data.extend_from_slice(&chunk);
                }
                Ok(_) => break,
                Err(e) => {
                    result = Err(e);
                    break;
                }
            }
        }
        let _ = fs.close(f.handle);
        result?;
        self.rev = Some(f.revision);
        Ok(data)
    }

    /// Ulozi; `CONFLICT` = soubor mezitim zmenil nekdo jiny.
    pub fn save(&mut self, data: &[u8]) -> Result<()> {
        let fs = self.fs.as_ref().ok_or(Error::NotFound)?;
        let rev = fs.save_if(PATH, self.rev, data)?;
        self.rev = Some(rev);
        Ok(())
    }
}
