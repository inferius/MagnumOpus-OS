//! Volba vzhledu (svetly, tmavy, podle denni doby; kontrast a omezeny
//! pohyb): ukladani a predani kompozitoru. Popis `docs/cs/vzhled.md`.
//!
//! - Uloziste `/system/config/vzhled/`: `system.toml` (prihlasovaci
//!   obrazovka, vychozi `auto`) a `<uid>.toml` (volba uzivatele). Adresar
//!   patri systemu bez grantu: volbu cte a pise jen init, takze ji jde
//!   pouzit i pred odemcenim sifrovaneho domova. Format
//!   `theme::Pref::to_text`.
//! - Kompozitor dostane systemovou volbu a volby prihlasenych pri startu
//!   (i po padu, `gui::give_accounts`) a volbu uzivatele pred `session
//!   open` (`desktop.rs`): rezim pocita sam (i podle casu).
//! - Zmena z listy prijde od kompozitoru na kanalu `launcher`
//!   (`appearance user <uid> <slova>`), z shellu prikazem `vzhled`
//!   (`powerproto::Request::Appearance`). Systemovou volbu meni jen ukon
//!   spravy: `ELEVATE` `APPEARANCE_SYSTEM` za relaci shellu (potvrzeni
//!   clovekem s roli v systemovem dialogu), grant init overi a spotrebuje
//!   na kanalu `grants`.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use accountproto::client::SelfService;
use accountproto::elevate::{action, encode_check};
use fsproto::FsClient;
use fsproto::perm::{ACL_PROTECTED, Acl};
use guiproto::control;
use libos::{Channel, Error, Result, println, time};
use powerproto::Reply;
use theme::Pref;

pub const DIR: &str = "/system/config/vzhled";

fn fs() -> Result<&'static FsClient> {
    crate::system::devices().and_then(|d| d.fs.as_ref()).ok_or(Error::NotFound)
}

fn path(uid: Option<u64>) -> String {
    match uid {
        Some(u) => format!("{DIR}/{u}.toml"),
        None => format!("{DIR}/system.toml"),
    }
}

/// Ulozena volba (`None` = soubor neni nebo je neplatny).
fn load(uid: Option<u64>) -> Option<Pref> {
    let fs = fs().ok()?;
    let (data, _) = crate::certs::load(fs, &path(uid)).ok()??;
    match Pref::parse(&String::from_utf8_lossy(&data)) {
        Ok(p) => Some(p),
        Err(e) => {
            println!("init: {} radek {}: {}", path(uid), e.line, e.what);
            None
        }
    }
}

/// Systemova volba (vychozi: podle denni doby).
pub fn system() -> Pref {
    load(None).unwrap_or_default()
}

/// Volba uzivatele, jinak systemova.
pub fn user(uid: u64) -> Pref {
    load(Some(uid)).unwrap_or_else(system)
}

/// Ulozeni (i pro test `gui_look::store`).
#[cfg(feature = "tests")]
pub fn save_for(uid: Option<u64>, p: &Pref) -> Result<()> {
    save(uid, p)
}

fn save(uid: Option<u64>, p: &Pref) -> Result<()> {
    let fs = fs()?;
    let mut dir = String::new();
    for part in DIR.split('/').filter(|p| !p.is_empty()) {
        dir.push('/');
        dir.push_str(part);
        match fs.create(&dir, fsproto::kind::DIR) {
            Ok(()) | Err(Error::AlreadyExists) => {}
            Err(e) => return Err(e),
        }
    }
    // Jen system (init): bez grantu, ostatni adresar ani nevidi.
    if fs.permissions(DIR)?.acl.is_none_or(|a| a.flags & ACL_PROTECTED == 0 || !a.grants.is_empty()) {
        fs.set_acl(DIR, Some(&Acl { flags: ACL_PROTECTED, grants: Vec::new() }))?;
    }
    crate::certs::store(fs, &path(uid), p.to_text().as_bytes()).map(|_| ())
}

/// Prikaz kompozitoru s volbou (`appearance system|user <uid> <slova>`).
pub fn command(uid: Option<u64>, p: &Pref) -> String {
    match uid {
        Some(u) => format!("{} user {u} {}", control::APPEARANCE, p.to_words()),
        None => format!("{} system {}", control::APPEARANCE, p.to_words()),
    }
}

/// Prikazy pro novou instanci kompozitoru: systemova volba a volby
/// prihlasenych (posila je `gui::give_accounts`).
pub fn commands() -> Vec<String> {
    let uids: Vec<u64> = crate::session::SESSIONS.lock().iter().map(|u| u.uid).collect();
    let mut v = alloc::vec![command(None, &system())];
    v.extend(uids.into_iter().map(|u| command(Some(u), &user(u))));
    v
}

fn send(uid: Option<u64>, p: &Pref) {
    if let Some(g) = crate::gui::GUI.lock().as_ref()
        && let Err(e) = g.command(command(uid, p).as_bytes(), Vec::new())
    {
        println!("init: volba vzhledu pro kompozitor: {e}");
    }
}

/// Pred `session open`: kompozitor zna volbu uzivatele driv, nez skryje
/// prihlasovaci obrazovku (jeden prechod, ne dva).
pub fn send_user(uid: u64) {
    send(Some(uid), &user(uid));
}

/// Zmena z listy (kanal `launcher`): `user <uid> <slova>`. Ulozi se jen
/// uzivateli, ktery je prihlaseny.
pub fn from_compositor(rest: &str) {
    let Some(r) = rest.strip_prefix("user ") else { return };
    let Some((uid, words)) = r.split_once(' ') else { return };
    let (Ok(uid), Some(p)) = (uid.parse::<u64>(), Pref::from_words(words)) else { return };
    if !crate::session::SESSIONS.lock().iter().any(|u| u.uid == uid) {
        println!("init: volba vzhledu pro neprihlaseneho {uid} zahozena");
        return;
    }
    if let Err(e) = save(Some(uid), &p) {
        println!("init: volbu vzhledu {uid} nejde ulozit: {e}");
    }
}

/// Prikaz `vzhled` z relace shellu. `Ok(Some(kanal))` = systemova zmena
/// ceka na potvrzeni (vysledek prijde na kanal).
pub fn from_shell(session: Option<u64>, system: bool, change: &str) -> core::result::Result<Option<Channel>, Reply> {
    let fail = |e| Reply::Failed(e);
    if system {
        let mut p = self::system();
        if change.is_empty() {
            return Err(Reply::Text(p.to_words()));
        }
        p.apply(change).map_err(|_| fail(Error::InvalidArgs))?;
        let session = session.ok_or(fail(Error::AccessDenied))?;
        let (mine, theirs) = Channel::create().map_err(fail)?;
        libos::thread::spawn("vzhled-system", move || {
            let r = elevate_system(session, &p);
            let reply = match r {
                Ok(()) => Reply::Text(p.to_words()),
                Err(e) => Reply::Failed(e),
            };
            let _ = mine.write(&reply.encode(), Vec::new());
        })
        .map_err(fail)?;
        return Ok(Some(theirs));
    }
    let uid = session.and_then(crate::session::uid).ok_or(fail(Error::AccessDenied))?;
    let mut p = user(uid);
    if !change.is_empty() {
        p.apply(change).map_err(|_| fail(Error::InvalidArgs))?;
        save(Some(uid), &p).map_err(fail)?;
        send(Some(uid), &p);
    }
    Err(Reply::Text(p.to_words()))
}

/// Ukon spravy: dialog za relaci, grant overit, ulozit, poslat.
fn elevate_system(session: u64, p: &Pref) -> Result<()> {
    let own = crate::session::take_self(session).ok_or(Error::BadState)?;
    let me = SelfService(own);
    let params = p.to_words().into_bytes();
    let r = me.elevate(action::APPEARANCE_SYSTEM, params.clone());
    crate::session::put_self(session, me.0);
    let token = r?;
    // Grant overi a spotrebuje sluzba uctu (stejne jako jine sluzby).
    let grants = crate::accounts::channel("grants")?;
    grants.write(&encode_check(&token, action::APPEARANCE_SYSTEM, &params), Vec::new())?;
    let m = grants.read(Some(time::deadline_after(core::time::Duration::from_secs(5))))?;
    accountproto::parse_reply(&m.data)?;
    save(None, p)?;
    println!("init: systemova volba vzhledu: {}", p.to_words());
    send(None, p);
    Ok(())
}
