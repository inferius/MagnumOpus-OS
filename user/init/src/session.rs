//! Spravce sezeni: relace prihlasenych uzivatelu.
//!
//! Bezi ve vlastnim vlakne a obsluhuje soukromy kanal od `accountd`
//! (`accountproto::ToInit`). Jen init razi identitu: po uspesnem overeni
//! v accountd zalozi relaci FS `CONNECT_AS(uid, skupiny, profil)`; kanaly
//! s identitou dava jen procesum relace. accountd sam identitu vyrazit
//! nemuze a init odmitne cokoli mimo rozsah lidi (system 0 a servisni
//! ucty nikdy).
//!
//! Domovy: dataset `/home/<jmeno>`, vlastnik = uid, popis `PROTECTED` bez
//! grantu (ostatni ho nevidi), zakaz spousteni.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU64, Ordering};

use accountproto::{ToInit, encode_sessions, ids, reply_err, reply_ok, valid_name};
use fsproto::FsClient;
use fsproto::perm::{Acl, Profile, ACL_PROTECTED};
use libos::{Channel, Error, Handle, Result, println};

/// Kde lezi domovy.
pub const HOME_DIR: &str = "/home";
/// Kos spravy pro domovy smazanych uctu.
pub const DELETED_DIR: &str = "/home/.smazane";

/// Relace prihlaseneho uzivatele.
pub struct UserSession {
    pub id: u64,
    pub uid: u64,
    /// Skupiny a domov pro procesy relace (terminal, plocha, taskd -
    /// zapojeni s nimi).
    #[allow(dead_code)]
    pub groups: Vec<u64>,
    #[allow(dead_code)]
    pub home: String,
    /// Relace FS s identitou uzivatele (profil Standardni: `/ -> /`,
    /// rozhoduji opravneni).
    pub fs: FsClient,
    /// Kanal self do accountd (dostane ho nastaveni v relaci).
    pub own: Option<Channel>,
    pub locked: bool,
}

pub static SESSIONS: spin::Mutex<Vec<UserSession>> = spin::Mutex::new(Vec::new());
static NEXT: AtomicU64 = AtomicU64::new(1);

fn is_person(uid: u64) -> bool {
    (ids::FIRST_USER..=ids::LAST_USER).contains(&uid)
}

/// Dalsi relace FS se stejnou identitou (pro procesy relace a testy).
pub fn connect(session: u64) -> Result<FsClient> {
    let s = SESSIONS.lock();
    let u = s.iter().find(|u| u.id == session).ok_or(Error::NotFound)?;
    u.fs.connect()
}

/// Kanal self relace (nastaveni uctu v relaci); vezme si ho jednou.
pub fn take_self(session: u64) -> Option<Channel> {
    SESSIONS.lock().iter_mut().find(|u| u.id == session)?.own.take()
}

/// Je relace zamcena?
pub fn locked(session: u64) -> Option<bool> {
    SESSIONS.lock().iter().find(|u| u.id == session).map(|u| u.locked)
}

fn private_acl() -> Acl {
    Acl { flags: ACL_PROTECTED, grants: Vec::new() }
}

fn create_home(fs: &FsClient, uid: u64, name: &str) -> Result<()> {
    if !is_person(uid) || !valid_name(name) {
        return Err(Error::InvalidArgs);
    }
    let path = format!("{HOME_DIR}/{name}");
    match fs.create_dataset(&path) {
        Ok(_) => {}
        // Domov po predchozim pokusu (vypadek pred ulozenim uctu): jen
        // kdyz uz patri tomuto uzivateli.
        Err(Error::AlreadyExists) => {
            if fs.stat(&path)?.owner != uid {
                return Err(Error::AlreadyExists);
            }
        }
        Err(e) => return Err(e),
    }
    fs.set_owner(&path, uid)?;
    fs.set_acl(&path, Some(&private_acl()))?;
    fs.set_noexec(&path, Profile::Standard.noexec_on_data())?;
    Ok(())
}

fn delete_home(fs: &FsClient, uid: u64, name: &str) -> Result<()> {
    if !is_person(uid) || !valid_name(name) {
        return Err(Error::InvalidArgs);
    }
    match fs.create(DELETED_DIR, fsproto::kind::DIR) {
        Ok(()) => fs.set_acl(DELETED_DIR, Some(&private_acl()))?,
        Err(Error::AlreadyExists) => {}
        Err(e) => return Err(e),
    }
    fs.rename(&format!("{HOME_DIR}/{name}"), &format!("{DELETED_DIR}/{uid}"))
}

pub(crate) fn handle(fs: &FsClient, m: ToInit, handles: Vec<Handle>) -> Result<Vec<u8>> {
    match m {
        ToInit::OpenSession { uid, groups, home } => {
            if !is_person(uid) || groups.iter().any(|g| *g < ids::GROUP_USERS) {
                return Err(Error::AccessDenied);
            }
            let (_, user_fs) = fs.connect_as(uid, &groups, &Profile::Standard.user_mounts())?;
            let id = NEXT.fetch_add(1, Ordering::Relaxed);
            let own = handles.into_iter().next().map(Channel::from_handle);
            println!("init: relace {id} uzivatele {uid}");
            SESSIONS.lock().push(UserSession { id, uid, groups, home, fs: user_fs, own, locked: false });
            Ok(id.to_le_bytes().to_vec())
        }
        ToInit::CloseSession { session } => {
            let mut s = SESSIONS.lock();
            let i = s.iter().position(|u| u.id == session).ok_or(Error::NotFound)?;
            // Zahozeni relace FS zavre jeji kanal; procesy relace (az je
            // init bude spoustet) se ukonci zde.
            let u = s.remove(i);
            println!("init: relace {} uzivatele {} skoncila", u.id, u.uid);
            Ok(Vec::new())
        }
        ToInit::LockSession { session } | ToInit::UnlockSession { session } => {
            let lock = matches!(m, ToInit::LockSession { .. });
            let mut s = SESSIONS.lock();
            let u = s.iter_mut().find(|u| u.id == session).ok_or(Error::NotFound)?;
            u.locked = lock;
            Ok(Vec::new())
        }
        ToInit::CreateHome { uid, name } => create_home(fs, uid, &name).map(|_| Vec::new()),
        ToInit::DeleteHome { uid, name } => delete_home(fs, uid, &name).map(|_| Vec::new()),
        ToInit::ListSessions => {
            let s = SESSIONS.lock();
            let list: Vec<(u64, u64, bool)> = s.iter().map(|u| (u.id, u.uid, u.locked)).collect();
            Ok(encode_sessions(&list))
        }
        // Sifrovani datasetu FS zatim neumi (samostatny krok FS, viz
        // `docs/cs/ucty.md`): klic se nikam nepreda.
        ToInit::UnlockHome { .. } => Err(Error::NotSupported),
    }
}

/// Smycka vlakna: pozadavky accountd na kanalu `ch`, `fs` = ridici
/// relace FS (system).
pub fn serve(ch: Channel, fs: FsClient) {
    loop {
        let mut msg = match ch.read(None) {
            Ok(m) => m,
            Err(Error::PeerClosed) => {
                println!("init: accountd zavrel kanal sezeni");
                return;
            }
            Err(e) => {
                println!("init: kanal sezeni: {e}");
                return;
            }
        };
        let reply = match ToInit::decode(&msg.data) {
            Ok(m) => match handle(&fs, m, core::mem::take(&mut msg.handles)) {
                Ok(b) => reply_ok(&b),
                Err(e) => reply_err(e),
            },
            Err(e) => reply_err(e),
        };
        // Zprava mohla obsahovat klic domova.
        zeroize::Zeroize::zeroize(&mut msg.data);
        if ch.write(&reply, Vec::new()).is_err() {
            return;
        }
    }
}
