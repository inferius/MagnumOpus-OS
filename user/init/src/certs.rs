//! Systemove uloziste korenovych certifikatu `/system/config/certs`.
//!
//! - `mozilla.pem`: vychozi sada CA (Mozilla, z initfs `certs.pem`). Init ji
//!   pri kazdem startu porovna s initfs a pri zmene atomicky prepise - sada
//!   se aktualizuje se systemem.
//! - Vlastni CA (firma, testy): dalsi soubory `*.pem` / `*.der` vedle;
//!   init na ne nesaha. V testovacim behu init prida `test-ca.pem` (CA
//!   pomocneho serveru hostitele z initfs `net-test-ca.pem`).
//! - `distrust.txt`: otisky SHA-256 CA, kterym se neveri (i kdyz jsou ve
//!   `mozilla.pem`) - spravce ho upravuje, init ne.
//!
//! Adresar patri systemu, vsichni ho smi jen cist (`EVERYONE`: vypis,
//! pruchod, cteni). Programy dostanou relaci FS s jedinym pripojenim
//! `/certs` jen pro cteni ([`session`]) - zapis do uloziste maji jen
//! systemove relace.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use fsproto::v2::{Revision, access, share};
use fsproto::FsClient;
use libos::initfs::Initfs;
use libos::{Error, Result, Startup, println};

use crate::system::Devices;

pub const DIR: &str = "/system/config/certs";
/// Ucet programu bez prihlaseneho uzivatele (zatim vsechny spoustene
/// initem). Smi jen to, co smi `EVERYONE`.
pub const NOBODY: u64 = 65_534;

/// Cely soubor a jeho revize (`None` = soubor neexistuje).
pub(crate) fn load(fs: &FsClient, path: &str) -> Result<Option<(Vec<u8>, Revision)>> {
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
    result.map(|_| Some((data, f.revision)))
}

/// Ulozi `data`, kdyz se lisi od souboru (atomicky: docasny soubor
/// a prejmenovani). Vraci, jestli se zapisovalo.
pub(crate) fn store(fs: &FsClient, path: &str, data: &[u8]) -> Result<bool> {
    let expected = match load(fs, path)? {
        Some((old, _)) if old == data => return Ok(false),
        Some((_, rev)) => Some(rev),
        None => None,
    };
    fs.save_if(path, expected, data)?;
    Ok(true)
}

/// Vytvori adresar uloziste (s nadrazenymi), prava pro cteni vsem.
fn prepare(fs: &FsClient) -> Result<()> {
    use fsproto::kind;
    use fsproto::perm::{EVERYONE, Grant, grant, rights};
    crate::system::grant_traverse(fs, "/", EVERYONE)?;
    let mut path = String::new();
    for part in DIR.split('/').filter(|p| !p.is_empty()) {
        path.push('/');
        path.push_str(part);
        match fs.create(&path, kind::DIR) {
            Ok(()) | Err(Error::AlreadyExists) => {}
            Err(e) => return Err(e),
        }
        if path != DIR {
            crate::system::grant_traverse(fs, &path, EVERYONE)?;
        }
    }
    let mut acl = fs.permissions(DIR)?.acl.unwrap_or_default();
    if !acl.grants.iter().any(|g| g.who == EVERYONE && g.rights == rights::VIEW && g.flags == grant::BOTH) {
        acl.grants.retain(|g| g.who != EVERYONE);
        acl.grants.push(Grant {
            who: EVERYONE,
            rights: rights::VIEW,
            flags: grant::BOTH,
        });
        fs.set_acl(DIR, Some(&acl))?;
    }
    Ok(())
}

/// Pripravi uloziste a aktualizuje vychozi sadu z initfs.
pub fn seed(startup: &Startup, devices: &Devices) -> Result<()> {
    let fs = devices.fs.as_ref().ok_or(Error::NotFound)?;
    let initfs = Initfs::open(startup)?;
    prepare(fs)?;
    let bundle = initfs.find("certs.pem").ok_or(Error::NotFound)?;
    if store(fs, &format!("{DIR}/mozilla.pem"), bundle)? {
        println!("init: korenove certifikaty aktualizovany ({} kB)", bundle.len() / 1024);
    }
    if let Some(ca) = initfs.find("net-test-ca.pem") {
        store(fs, &format!("{DIR}/test-ca.pem"), ca)?;
    }
    Ok(())
}

/// Relace FS pro program: jen `/certs` (uloziste) a jen pro cteni.
pub fn session(devices: &Devices) -> Result<FsClient> {
    use fsproto::perm::{Mount, mount_flags};
    let fs = devices.fs.as_ref().ok_or(Error::NotFound)?;
    let (_, session) = fs.connect_as(NOBODY, &[], &[Mount::new("/certs", DIR, mount_flags::READ_ONLY)])?;
    Ok(session)
}
