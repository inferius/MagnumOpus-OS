//! Prvni ucet primo do DB uctu na cilovem FS (`/system/accounts`, stejny
//! format jako sluzba uctu: `libs/accounts`). Init pri prvnim startu
//! nainstalovaneho systemu rozlozeni DB jen overi (`accounts::layout`)
//! a accountd ucet nacte.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use accountproto::{Secret, ids};
use accounts::{Account, Config, Engine, FileKey, KdfParams, Meta, Store};
use fs::{Acl, Fs, Kind, acl_flags};
use libos::{Error, Result};

use crate::disks::FsPart;

const DB: &str = "/system/accounts";
const USERS: &str = "/system/accounts/users";
const META: &str = "/system/accounts/meta";
const DEVICE_KEY: &str = "/system/accounts/device.key";
const HOME: &str = "/home";

/// Uloziste uctu nad cilovym FS.
struct FsStore<'a, 'b> {
    fs: &'a mut Fs<FsPart<'b>>,
}

fn write_owned(fs: &mut Fs<FsPart>, path: &str, data: &[u8], owner: u64) -> Result<()> {
    match fs.create(path, Kind::File) {
        Ok(()) | Err(Error::AlreadyExists) => {}
        Err(e) => return Err(e),
    }
    fs.write(path, 0, data)?;
    fs.set_owner(path, owner)
}

impl Store for FsStore<'_, '_> {
    #[allow(clippy::type_complexity)]
    fn load(&mut self) -> Result<(Option<Meta>, Vec<core::result::Result<Account, (String, Error)>>)> {
        let meta = match self.fs.stat(META) {
            Ok(st) => Some(Meta::decode(&self.fs.read(META, 0, st.size as usize)?)?),
            Err(Error::NotFound) => None,
            Err(e) => return Err(e),
        };
        let mut list = Vec::new();
        if let Ok(entries) = self.fs.list(USERS) {
            for e in entries {
                let path = format!("{USERS}/{}", e.name);
                let size = self.fs.stat(&path)?.size as usize;
                let data = self.fs.read(&path, 0, size)?;
                list.push(Account::decode(&data).map_err(|err| (e.name.clone(), err)));
            }
        }
        Ok((meta, list))
    }

    fn save(&mut self, a: &Account) -> Result<()> {
        write_owned(self.fs, &format!("{USERS}/{}", a.uid), &a.encode(), ids::ACCOUNTD)
    }

    fn save_meta(&mut self, m: &Meta) -> Result<()> {
        write_owned(self.fs, META, &m.encode(), ids::ACCOUNTD)
    }

    fn remove(&mut self, uid: u64) -> Result<()> {
        self.fs.remove(&format!("{USERS}/{uid}"))
    }
}

/// Nahoda z jadra (ceka na nasazeny generator).
struct OsRng;

impl rand_core::RngCore for OsRng {
    fn next_u32(&mut self) -> u32 {
        let mut b = [0u8; 4];
        self.fill_bytes(&mut b);
        u32::from_le_bytes(b)
    }
    fn next_u64(&mut self) -> u64 {
        let mut b = [0u8; 8];
        self.fill_bytes(&mut b);
        u64::from_le_bytes(b)
    }
    fn fill_bytes(&mut self, dest: &mut [u8]) {
        crate::random(dest);
    }
    fn try_fill_bytes(&mut self, dest: &mut [u8]) -> core::result::Result<(), rand_core::Error> {
        self.fill_bytes(dest);
        Ok(())
    }
}

impl rand_core::CryptoRng for OsRng {}

/// Rozlozeni DB (jako init `accounts::layout`) a /home.
fn layout(fs: &mut Fs<FsPart>) -> Result<()> {
    match fs.create("/system", Kind::Dir) {
        Ok(()) | Err(Error::AlreadyExists) => {}
        Err(e) => return Err(e),
    }
    match fs.create_dataset(DB) {
        Ok(_) | Err(Error::AlreadyExists) => {}
        Err(e) => return Err(e),
    }
    fs.set_owner(DB, ids::ACCOUNTD)?;
    fs.set_acl(DB, Some(Acl { flags: acl_flags::PROTECTED, grants: Vec::new() }))?;
    fs.set_noexec(DB, true)?;
    match fs.create(USERS, Kind::Dir) {
        Ok(()) | Err(Error::AlreadyExists) => {}
        Err(e) => return Err(e),
    }
    fs.set_owner(USERS, ids::ACCOUNTD)?;
    match fs.create(HOME, Kind::Dir) {
        Ok(()) | Err(Error::AlreadyExists) => {}
        Err(e) => return Err(e),
    }
    Ok(())
}

/// Zalozi prvni ucet (a PIN, kdyz neni prazdny) a jeho domov. Vraci
/// (uid, bezpecnostni fraze). Kdyz DB uz ucty ma (preinstalace), nic
/// nezaklada: `Ok(None)`.
pub fn create_first(
    fs: &mut Fs<FsPart>,
    name: &str,
    display: &str,
    password: &str,
    pin: &str,
    qemu_kdf: bool,
    now: i64,
) -> Result<Option<(u64, String)>> {
    layout(fs)?;
    let mut key = [0u8; 32];
    match fs.stat(DEVICE_KEY) {
        Ok(_) => {
            let k = fs.read(DEVICE_KEY, 0, 32)?;
            key.copy_from_slice(k.get(..32).ok_or(Error::Corrupted)?);
        }
        Err(_) => {
            crate::random(&mut key);
            write_owned(fs, DEVICE_KEY, &key, ids::ACCOUNTD)?;
        }
    }
    let cfg = Config { params: if qemu_kdf { KdfParams::QEMU } else { KdfParams::HW }, dev: false };
    let mut eng = Engine::new(FsStore { fs }, FileKey::new(key), OsRng, cfg)?;
    if !eng.accounts().is_empty() {
        return Ok(None);
    }
    let pass = Secret::new(password.as_bytes().to_vec());
    let a = eng.prepare_first(name, display, &pass, now)?;
    let phrase = a.phrase.clone();
    let uid = eng.commit(a)?;
    if !pin.is_empty() {
        eng.set_pin(uid, &pass, &Secret::new(pin.as_bytes().to_vec()), now)?;
    }
    drop(eng);
    // Domov jako init (CREATE_HOME): dataset, vlastnik, bez grantu, NOEXEC.
    let home = format!("{HOME}/{name}");
    match fs.create_dataset(&home) {
        Ok(_) | Err(Error::AlreadyExists) => {}
        Err(e) => return Err(e),
    }
    fs.set_owner(&home, uid)?;
    fs.set_acl(&home, Some(Acl { flags: acl_flags::PROTECTED, grants: Vec::new() }))?;
    fs.set_noexec(&home, true)?;
    Ok(Some((uid, phrase)))
}
