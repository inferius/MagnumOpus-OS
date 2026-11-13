//! Spravce souboru: pomocny proces `files-copy` pro kazdou relaci
//! uzivatele (kopirovani a presun jako ulohy, prezije zavreni okna)
//! a handly pro okna `gui-files`.
//!
//! `files-copy` bezi v jobu relace s relaci FS uzivatele a kanalem
//! producenta na taskd; skonci s relaci (ridici kanal se zavre). Stav
//! uloh je v domove uzivatele, takze po pristim prihlaseni pokracuji.
//! Okno spravce dostane vlastni relaci FS se stejnou identitou, kanal
//! k `files-copy` a domov jako pracovni adresar.

use alloc::format;
use alloc::vec;
use alloc::vec::Vec;

use libos::params::Params;
use libos::spawn::StartHandle;
use libos::{Channel, Error, Result, Startup, println};

use crate::apps::App;

struct Copier {
    session: u64,
    ctl: Channel,
    _app: App,
}

static COPIERS: spin::Mutex<Vec<Copier>> = spin::Mutex::new(Vec::new());

/// Identita ulohy v prehledu uloh (jako terminal relace).
const TASKS_USER: &str = "uzivatel";

fn home(session: u64) -> Result<alloc::string::String> {
    let name = crate::session::user_name(session).ok_or(Error::NotFound)?;
    Ok(format!("/home/{name}"))
}

/// Blok parametru s domovem jako pracovnim adresarem.
fn params(session: u64) -> Result<StartHandle> {
    Params { cwd: home(session)?.into_bytes(), args: vec![], env: vec![] }.start_handle()
}

/// Spusti `files-copy` pro relaci (pri otevreni relace).
pub fn start(startup: &Startup, session: u64) -> Result<()> {
    let fs = crate::session::connect(session)?;
    let job = crate::session::job(session)?;
    let (mine, theirs) = Channel::create()?;
    let mut hs = vec![StartHandle::user(fs.into_channel().into_handle()), StartHandle::user(theirs.into_handle())];
    match crate::tasks::producer(TASKS_USER) {
        Ok(p) => hs.push(StartHandle::user(p.into_handle())),
        Err(e) => println!("init: files-copy bez prehledu uloh: {e}"),
    }
    hs.push(params(session)?);
    let app = crate::apps::spawn_in(startup, "files-copy", hs, Some(&job))?;
    crate::tasks::app("Kopírování souborů", "org.magnumopus.files.copy", &app.job);
    COPIERS.lock().push(Copier { session, ctl: mine, _app: app });
    Ok(())
}

/// Konec relace: ridici kanal pryc (proces skonci, job relace ho stejne
/// ukonci).
pub fn close(session: u64) {
    COPIERS.lock().retain(|c| c.session != session);
}

/// Handly pro okno spravce souboru: relace FS uzivatele, kanal
/// k `files-copy` (kdyz bezi) a domov v bloku parametru.
pub fn window_handles(session: u64) -> Result<Vec<StartHandle>> {
    let fs = crate::session::connect(session)?;
    let mut hs = vec![StartHandle::user(fs.into_channel().into_handle())];
    let copiers = COPIERS.lock();
    if let Some(c) = copiers.iter().find(|c| c.session == session) {
        let (mine, theirs) = Channel::create()?;
        match c.ctl.write(b"client", vec![mine.into_handle()]) {
            Ok(()) => hs.push(StartHandle::user(theirs.into_handle())),
            Err(e) => println!("init: kanal files-copy relace {session}: {e}"),
        }
    }
    drop(copiers);
    hs.push(params(session)?);
    Ok(hs)
}
