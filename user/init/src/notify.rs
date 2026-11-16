//! Oznameni: sluzba `notifyd` (`user/notifyd`, `libs/notifycore`,
//! `docs/cs/oznameni.md`).
//!
//! Init notifyd spusti, drzi jeho ridici kanal a jako jediny razi kanaly
//! s identitou:
//! - **odesilatel** ([`sender`], [`app_handles`]): aplikace dostane pri
//!   spusteni kanal se stitkem `abi::params::tag::NOTIFY` s identitou
//!   z manifestu (id, jmeno) a uzivatelem relace; systemove sluzby
//!   identitu `system` (smi `Urgent`),
//! - **prohlizec** ([`viewer`]): kompozitor pro kazdou otevrenou relaci
//!   (prikaz `notify <uid>`), testy.

use alloc::format;
use alloc::vec::Vec;
use core::time::Duration;

use libos::initfs::Initfs;
use libos::spawn::StartHandle;
use libos::{Channel, Error, Handle, Process, Result, Startup, println, time};

const REPLY_TIMEOUT: Duration = Duration::from_secs(5);

pub struct Notify {
    pub process: Process,
    ctl: Channel,
}

pub static NOTIFY: spin::Mutex<Option<Notify>> = spin::Mutex::new(None);

fn command(ctl: &Channel, text: &str) -> Result<Channel> {
    ctl.write(text.as_bytes(), Vec::new())?;
    let m = ctl.read(Some(time::deadline_after(REPLY_TIMEOUT)))?;
    let reply = core::str::from_utf8(&m.data).map_err(|_| Error::Corrupted)?;
    if let Some(code) = reply.strip_prefix("err ") {
        return Err(libos::abi::error::ALL.iter().copied().find(|e| e.name() == code).unwrap_or(Error::Corrupted));
    }
    m.handles.into_iter().next().map(Channel::from_handle).ok_or(Error::Corrupted)
}

/// Spusti notifyd (z initfs, bez dalsich handlu).
pub fn start(startup: &Startup) -> Result<()> {
    let fs = Initfs::open(startup)?;
    let image = fs.find("notifyd").ok_or(Error::NotFound)?;
    let (process, ctl) = libos::spawn("notifyd", image, Vec::new())?;
    *NOTIFY.lock() = Some(Notify { process, ctl });
    println!("init: sluzba oznameni (notifyd) bezi");
    Ok(())
}

/// Id aplikace pro ridici prikaz (jedno slovo, bez ridicich znaku).
fn word(s: &str) -> Option<&str> {
    (!s.is_empty() && !s.contains(char::is_whitespace) && !s.contains(char::is_control)).then_some(s)
}

/// Kanal odesilatele s identitou (`uid` 0 = systemova sluzba).
pub fn sender(app_id: &str, name: &str, uid: u64, system: bool) -> Result<Channel> {
    let app_id = word(app_id).ok_or(Error::InvalidArgs)?;
    let name: alloc::string::String = name.chars().filter(|c| !c.is_control()).collect();
    let g = NOTIFY.lock();
    let n = g.as_ref().ok_or(Error::ServiceUnavailable)?;
    command(&n.ctl, &format!("sender {uid} {} {app_id} {name}", system as u8))
}

/// Kanal prohlizece oznameni uzivatele `uid`.
pub fn viewer(uid: u64) -> Result<Channel> {
    let g = NOTIFY.lock();
    let n = g.as_ref().ok_or(Error::ServiceUnavailable)?;
    command(&n.ctl, &format!("viewer {uid}"))
}

/// Handle oznameni pro spoustenou aplikaci (identita z manifestu); bez
/// manifestu nebo sluzby aplikace bezi bez oznameni.
pub fn app_handles(startup: &Startup, name: &str, uid: u64) -> Vec<StartHandle> {
    let Ok(fs) = Initfs::open(startup) else { return Vec::new() };
    let Ok(Some(m)) = crate::apps::manifest(&fs, name) else { return Vec::new() };
    match sender(&m.id, &m.name, uid, false) {
        Ok(ch) => alloc::vec![StartHandle::tagged(libos::abi::params::tag::NOTIFY, ch.into_handle())],
        Err(e) => {
            println!("init: oznameni pro {name}: {e}");
            Vec::new()
        }
    }
}

/// Prohlizec relace pro kompozitor (zavola gui po `session open`).
pub fn viewer_handle(uid: u64) -> Option<Handle> {
    viewer(uid).ok().map(Channel::into_handle)
}
