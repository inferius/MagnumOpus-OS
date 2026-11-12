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
//!
//! Kazda relace ma **job**: v nem bezi procesy uzivatele (aplikace plochy,
//! termd a jeho shsess). Udalosti relaci (otevrena, zamcena, skoncila) jdou
//! kanalem [`EVENTS`] smycce supervisoru, ktera spousti a ukoncuje plochu
//! relace (`crate::desktop`).

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU64, Ordering};

use accountproto::{ToInit, encode_sessions, ids, reply_err, reply_ok, valid_name};
use fsproto::FsClient;
use fsproto::perm::{Acl, Profile, ACL_PROTECTED};
use libos::job::Job;
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
    /// Job s procesy relace.
    pub job: Job,
    /// Jmeno uzivatele pro kompozitor (`/home/<jmeno>`).
    pub name: String,
}

pub static SESSIONS: spin::Mutex<Vec<UserSession>> = spin::Mutex::new(Vec::new());

/// Udalost relace pro smycku supervisoru.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    Opened(u64),
    Locked(u64),
    Unlocked(u64),
    Closed(u64),
}

impl Event {
    fn encode(self) -> [u8; 9] {
        let (k, id) = match self {
            Event::Opened(i) => (1, i),
            Event::Locked(i) => (2, i),
            Event::Unlocked(i) => (3, i),
            Event::Closed(i) => (4, i),
        };
        let mut b = [0u8; 9];
        b[0] = k;
        b[1..].copy_from_slice(&id.to_le_bytes());
        b
    }

    pub fn decode(b: &[u8]) -> Option<Event> {
        let id = u64::from_le_bytes(b.get(1..9)?.try_into().ok()?);
        Some(match b.first()? {
            1 => Event::Opened(id),
            2 => Event::Locked(id),
            3 => Event::Unlocked(id),
            4 => Event::Closed(id),
            _ => return None,
        })
    }
}

/// Kanal udalosti relaci: (zapis = vlakno sezeni, cteni = supervisor).
pub static EVENTS: spin::Once<(Channel, Channel)> = spin::Once::new();

fn notify(e: Event) {
    if let Some((tx, _)) = EVENTS.get() {
        let _ = tx.write(&e.encode(), Vec::new());
    }
}

/// Konec kanalu udalosti pro supervisor.
pub fn events() -> Option<&'static Channel> {
    EVENTS.get().map(|(_, rx)| rx)
}

/// Pripravi kanal udalosti (pred startem accountd).
pub fn init_events() -> Result<()> {
    let (tx, rx) = Channel::create()?;
    EVENTS.call_once(|| (tx, rx));
    Ok(())
}

/// Duplikat jobu relace (spousteni procesu relace).
pub fn job(session: u64) -> Result<Job> {
    let s = SESSIONS.lock();
    s.iter().find(|u| u.id == session).ok_or(Error::NotFound)?.job.duplicate(None)
}

/// Uzivatel relace.
pub fn uid(session: u64) -> Option<u64> {
    SESSIONS.lock().iter().find(|u| u.id == session).map(|u| u.uid)
}

/// Jmeno uzivatele relace.
pub fn name(session: u64) -> Option<String> {
    SESSIONS.lock().iter().find(|u| u.id == session).map(|u| u.name.clone())
}

/// Nejnovejsi odemcena relace (do ni jdou aplikace spustene z listy).
pub fn active() -> Option<u64> {
    SESSIONS.lock().iter().rev().find(|u| !u.locked).map(|u| u.id)
}
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
    // Existujici domov (opakovane zalozeni po vypadku) jen kdyz uz patri
    // tomuto uzivateli.
    match fs.create_dataset(&path) {
        Ok(_) => {}
        Err(Error::AlreadyExists) => {
            if fs.stat(&path)?.owner != uid {
                return Err(Error::AlreadyExists);
            }
        }
        Err(e) => return Err(e),
    }
    // Jen co se lisi (opakovane zalozeni po vypadku nema nic menit).
    if fs.stat(&path)?.owner != uid {
        fs.set_owner(&path, uid)?;
    }
    let acl = private_acl();
    if fs.permissions(&path)?.acl.as_ref() != Some(&acl) {
        fs.set_acl(&path, Some(&acl))?;
    }
    let noexec = Profile::Standard.noexec_on_data();
    if fs.noexec(&path)? != noexec {
        fs.set_noexec(&path, noexec)?;
    }
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
            let name = home.strip_prefix("/home/").filter(|n| valid_name(n)).ok_or(Error::InvalidArgs)?;
            let name = String::from(name);
            let (_, user_fs) = fs.connect_as(uid, &groups, &Profile::Standard.user_mounts())?;
            let job = Job::create(&libos::job::current()?)?;
            let id = NEXT.fetch_add(1, Ordering::Relaxed);
            let own = handles.into_iter().next().map(Channel::from_handle);
            println!("init: relace {id} uzivatele {uid}");
            SESSIONS.lock().push(UserSession { id, uid, groups, home, fs: user_fs, own, locked: false, job, name });
            notify(Event::Opened(id));
            Ok(id.to_le_bytes().to_vec())
        }
        ToInit::CloseSession { session } => {
            let mut s = SESSIONS.lock();
            let i = s.iter().position(|u| u.id == session).ok_or(Error::NotFound)?;
            // Zahozeni relace FS zavre jeji kanal; procesy relace (az je
            // init bude spoustet) se ukonci zde.
            let u = s.remove(i);
            drop(s);
            println!("init: relace {} uzivatele {} skoncila", u.id, u.uid);
            notify(Event::Closed(u.id));
            Ok(Vec::new())
        }
        ToInit::LockSession { session } | ToInit::UnlockSession { session } => {
            let lock = matches!(m, ToInit::LockSession { .. });
            let mut s = SESSIONS.lock();
            let u = s.iter_mut().find(|u| u.id == session).ok_or(Error::NotFound)?;
            u.locked = lock;
            drop(s);
            notify(if lock { Event::Locked(session) } else { Event::Unlocked(session) });
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
