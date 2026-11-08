//! Sluzba uctu `accountd` (popis `docs/cs/ucty.md`).
//!
//! - DB je vlastni dataset `/system/accounts`: vlastnik servisni ucet
//!   accountd (902), popis `PROTECTED` bez grantu, zakaz spousteni.
//!   accountd ma relaci FS jen s `/db -> /system/accounts`.
//! - `/home` patri systemu; skupina `uzivatele` ho (a koren) smi jen
//!   projit: kdo nezna jmeno, nedozvi se, jake domovy existuji.
//! - Soukromy kanal accountd <-> init obsluhuje spravce sezeni
//!   ([`crate::session`]) ve vlastnim vlakne s vlastni ridici relaci FS.
//! - Profil Argon2id: `accounts-qemu` v initfs (xtask run a test) = rychly
//!   profil pro QEMU, jinak `hw`. Vyvojove automaticke prihlaseni jen se
//!   souborem `accounts-dev` (pridava ho jen `cargo xtask run`).

use alloc::format;
use alloc::vec;
use alloc::vec::Vec;
use core::time::Duration;

use accountproto::ids;
use fsproto::FsClient;
use fsproto::perm::{Acl, Grant, Mount, ACL_PROTECTED, grant, rights};
use libos::initfs::Initfs;
use libos::spawn::StartHandle;
use libos::{Channel, Error, Process, Result, Startup, println, time};

use crate::session::{self, HOME_DIR};
use crate::system::Devices;

pub const DB_DIR: &str = "/system/accounts";

pub struct Accountd {
    /// Handle procesu (hlida ho supervisor).
    #[allow(dead_code)]
    pub process: Process,
    pub control: spin::Mutex<Channel>,
}

pub static ACCOUNTD: spin::Once<Accountd> = spin::Once::new();

/// Prida `who` pravo pruchodu adresarem `path`.
fn traverse(fs: &FsClient, path: &str, who: u64) -> Result<()> {
    let mut acl = fs.permissions(path)?.acl.unwrap_or_default();
    if acl.grants.iter().any(|g| g.who == who && g.rights & rights::TRAVERSE != 0) {
        return Ok(());
    }
    acl.grants.push(Grant { who, rights: rights::TRAVERSE, flags: grant::THIS });
    fs.set_acl(path, Some(&acl))
}

fn mkdir(fs: &FsClient, path: &str) -> Result<()> {
    match fs.create(path, fsproto::kind::DIR) {
        Ok(()) | Err(Error::AlreadyExists) => Ok(()),
        Err(e) => Err(e),
    }
}

/// Rozlozeni na FS: dataset DB a `/home`.
fn layout(fs: &FsClient) -> Result<()> {
    mkdir(fs, "/system")?;
    match fs.create_dataset(DB_DIR) {
        Ok(_) | Err(Error::AlreadyExists) => {}
        Err(e) => return Err(e),
    }
    fs.set_owner(DB_DIR, ids::ACCOUNTD)?;
    fs.set_acl(DB_DIR, Some(&Acl { flags: ACL_PROTECTED, grants: Vec::new() }))?;
    fs.set_noexec(DB_DIR, true)?;
    traverse(fs, "/", ids::ACCOUNTD)?;
    traverse(fs, "/system", ids::ACCOUNTD)?;
    mkdir(fs, HOME_DIR)?;
    traverse(fs, "/", ids::GROUP_USERS)?;
    traverse(fs, HOME_DIR, ids::GROUP_USERS)?;
    Ok(())
}

pub fn start(startup: &Startup, devices: &Devices, sup: &mut crate::supervisor::Supervisor) -> Result<()> {
    let fs = devices.fs.as_ref().ok_or(Error::NotFound)?;
    layout(fs)?;
    let recipe = crate::fsd::Recipe::User {
        user: ids::ACCOUNTD,
        groups: Vec::new(),
        mounts: vec![Mount::new("/db", DB_DIR, 0)],
    };
    let (db, lease) = sup.session("fsd", recipe.encode())?;
    // Vlastni ridici relace pro vlakno sezeni (FsClient se nesdili mezi
    // vlakny pres globalni zarizeni, ktera jeste nemusi byt nastavena).
    let (ctl, ctl_lease) = sup.session("fsd", crate::fsd::Recipe::System.encode())?;
    let ctl = FsClient::with_lease(ctl, ctl_lease);
    let (ours, theirs) = Channel::create()?;
    libos::thread::spawn("sezeni", move || session::serve(ours, ctl))?;
    let handles = vec![
        StartHandle::user(db.into_handle()),
        StartHandle::user(lease.into_handle()),
        StartHandle::user(theirs.into_handle()),
    ];
    let initfs = Initfs::open(startup)?;
    let kdf = if initfs.find("accounts-qemu").is_some() { "qemu" } else { "hw" };
    let dev = initfs.find("accounts-dev").is_some();
    let app = crate::apps::spawn(startup, "accountd", handles)?;
    let cfg = format!("kdf = \"{kdf}\"\ndev = {dev}\n");
    app.control.write(cfg.as_bytes(), Vec::new())?;
    let msg = app.control.read(Some(time::deadline_after(Duration::from_secs(20))))?;
    if msg.data != b"ok" {
        println!("init: accountd hlasi: {}", core::str::from_utf8(&msg.data).unwrap_or("?"));
        return Err(Error::IoError);
    }
    // Bezpecnostni klice nalezene pri startu (a kanaly ostatnich HID
    // zarizeni, ktere jejich ovladac zavre).
    for k in core::mem::take(&mut *devices.keys.lock()) {
        let _ = send_key(&app.control, k);
    }
    sup.watch("accountd", &app.process, None);
    ACCOUNTD.call_once(|| Accountd { process: app.process, control: spin::Mutex::new(app.control) });
    Ok(())
}

/// Novy kanal `login` / `setup` k accountd (prihlasovaci obrazovka,
/// instalator, testy).
pub fn channel(kind: &str) -> Result<Channel> {
    let a = ACCOUNTD.get().ok_or(Error::NotFound)?;
    let (ours, theirs) = Channel::create()?;
    let ctl = a.control.lock();
    ctl.write(kind.as_bytes(), vec![theirs.into_handle()])?;
    let r = ctl.read(Some(time::deadline_after(Duration::from_secs(5))))?;
    if r.data != b"ok" {
        return Err(Error::IoError);
    }
    Ok(ours)
}

fn send_key(ctl: &Channel, key: Channel) -> Result<()> {
    ctl.write(b"key", vec![key.into_handle()])?;
    let r = ctl.read(Some(time::deadline_after(Duration::from_secs(5))))?;
    if r.data != b"ok" {
        return Err(Error::IoError);
    }
    Ok(())
}

/// Kanal bezpecnostniho klice pripojeneho za behu (zprava `fido` od
/// devmgr). Bez bezici sluzby uctu ho vrati (init si ho schova).
pub fn add_key(key: Channel) -> core::result::Result<(), Channel> {
    match ACCOUNTD.get() {
        Some(a) => {
            let _ = send_key(&a.control.lock(), key);
            Ok(())
        }
        None => Err(key),
    }
}

/// Stav accountd (`status`).
pub fn status() -> Result<alloc::string::String> {
    let a = ACCOUNTD.get().ok_or(Error::NotFound)?;
    let ctl = a.control.lock();
    ctl.write(b"status", Vec::new())?;
    let r = ctl.read(Some(time::deadline_after(Duration::from_secs(5))))?;
    Ok(alloc::string::String::from_utf8_lossy(&r.data).into_owned())
}
