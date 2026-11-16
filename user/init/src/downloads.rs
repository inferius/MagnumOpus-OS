//! Stahovani z webu do `~/Stazene` (docs/cs/stahovani.md): pro relaci
//! uzivatele init pri prvni zadosti spusti `fetch` v jobu relace s:
//!
//! 1. relaci site podle manifestu `fetch` (jen TCP 80/443 a DNS),
//! 2. relaci FS `/certs` (koreny CA), 3. kanalem k `revocd`,
//! 4. relaci FS s identitou uzivatele, ktera vidi **jen** `/Stazene`
//!    (`~/Stažené`, priznak `ORIGIN`: puvod `sys.origin` smi zapsat jen
//!    fetch; Soubory ani jine aplikace ne),
//! 5. producentem do prehledu uloh a odesilatelem oznameni.
//!
//! Okno Soubory dostane jen kanal pro prikazy `save <adresa>` (jeden
//! spolecny ridici kanal fetch; prikazy jdou po rade). Stahovani prezije
//! zavreni okna a skonci s relaci.

use alloc::format;
use alloc::vec;
use alloc::vec::Vec;

use fsproto::perm::{Mount, mount_flags};
use libos::initfs::Initfs;
use libos::spawn::StartHandle;
use libos::{Channel, Error, Result, Startup, println};

use crate::apps::App;

/// Slozka stazenych souboru v domove.
pub const DIR_NAME: &str = "Stažené";

struct Downloader {
    session: u64,
    app: App,
}

static DOWNLOADERS: spin::Mutex<Vec<Downloader>> = spin::Mutex::new(Vec::new());

/// Spusti fetch pro stahovani relace `session` (handly viz modul).
fn spawn(startup: &Startup, session: u64) -> Result<App> {
    let (uid, groups, home) = {
        let s = crate::session::SESSIONS.lock();
        let u = s.iter().find(|u| u.id == session).ok_or(Error::NotFound)?;
        (u.uid, u.groups.clone(), format!("/home/{}", u.name))
    };
    let dir = format!("{home}/{DIR_NAME}");
    // Slozka s vlastnikem uzivatelem (jeho relaci).
    let user = crate::session::connect(session)?;
    match user.create(&dir, fsproto::kind::DIR) {
        Ok(()) | Err(Error::AlreadyExists) => {}
        Err(e) => return Err(e),
    }
    let d = crate::system::devices().ok_or(Error::ServiceUnavailable)?;
    let sys = d.fs.as_ref().ok_or(Error::ServiceUnavailable)?;
    let (_, fs) = sys.connect_as(uid, &groups, &[Mount::new("/Stazene", &dir, mount_flags::ORIGIN)])?;
    let certs = crate::certs::session(d)?;
    let fsi = Initfs::open(startup)?;
    let m = crate::apps::manifest(&fsi, "fetch")?.ok_or(Error::NotFound)?;
    let net = crate::system::NET.get().ok_or(Error::ServiceUnavailable)?;
    let session_net = crate::apps::net_session(&net.root, &m)?.ok_or(Error::NotSupported)?;
    // Bez revocd prazdny kanal (fetch kontroluje jen OCSP stapling).
    let revocation = match crate::revocation::client() {
        Some(c) => c,
        None => Channel::create()?.1,
    };
    let mut hs = vec![
        StartHandle::user(session_net.into_channel()?.into_handle()),
        StartHandle::user(certs.into_channel().into_handle()),
        StartHandle::user(revocation.into_handle()),
        StartHandle::user(fs.into_channel().into_handle()),
    ];
    match crate::tasks::producer("uzivatel") {
        Ok(p) => hs.push(StartHandle::user(p.into_handle())),
        Err(e) => println!("init: stahovani bez prehledu uloh: {e}"),
    }
    if let Ok(n) = crate::notify::sender(&m.id, &m.name, uid, false) {
        hs.push(StartHandle::tagged(libos::abi::params::tag::NOTIFY, n.into_handle()));
    }
    let job = crate::session::job(session)?;
    let app = crate::apps::spawn_in(startup, "fetch", hs, Some(&job))?;
    println!("init: stahovani pro relaci {session} bezi");
    Ok(app)
}

/// Kanal pro prikazy stahovani relace (spusti fetch pri prvni zadosti).
/// Je to duplikat ridiciho kanalu fetch: okna posilaji `save-bg` bez
/// odpovedi (vysledek je v prehledu uloh a v oznameni).
pub fn client(startup: &Startup, session: u64) -> Result<Channel> {
    let mut g = DOWNLOADERS.lock();
    if !g.iter().any(|d| d.session == session) {
        let app = spawn(startup, session)?;
        g.push(Downloader { session, app });
    }
    let d = g.iter().find(|d| d.session == session).ok_or(Error::NotFound)?;
    Ok(Channel::from_handle(d.app.control.handle().duplicate(None)?))
}

/// Konec relace: proces skonci s jobem relace.
pub fn close(session: u64) {
    DOWNLOADERS.lock().retain(|d| d.session != session);
}
