//! Klient sluzby knihoven `libd` (`libdproto`, `docs/cs/dynamicke-linkovani.md`).
//!
//! Proces drzi jedno spojeni (od rodice ve startovni zprave, init ho ma od
//! spusteni libd). Pres nej `spawn` vytvori spojeni pro potomka (program se
//! zavadecem ho potrebuje, aby mohl spoustet dalsi takove programy)
//! a zavadec `ld.so` bere knihovny jako zapecetene sablony (sdileny kod).

use alloc::vec::Vec;
use core::time::Duration;

use abi::{Error, Result};
use libdproto::{Library, Reply, Request, Stats};

use crate::sync::Mutex;
use crate::{Channel, Handle, Vmo};

static CONNECTION: Mutex<Option<Channel>> = Mutex::new(None);

/// Jak dlouho cekat na odpoved libd.
const TIMEOUT: Duration = Duration::from_secs(10);

/// Nastavi spojeni s libd (init po spusteni sluzby, start procesu ze
/// startovni zpravy, zavadec ze zpravy zavadece).
pub fn set_connection(ch: Channel) {
    *CONNECTION.lock() = Some(ch);
}

/// Ma proces spojeni s libd?
pub fn available() -> bool {
    CONNECTION.lock().is_some()
}

/// Zadost a odpoved (spojeni je pod zamkem, odpovedi se nemichaji).
fn call(req: &Request, handles: Vec<Handle>) -> Result<(Reply, Vec<Handle>)> {
    let g = CONNECTION.lock();
    let ch = g.as_ref().ok_or(Error::NotFound)?;
    ch.write(&req.encode(), handles)?;
    let msg = ch.read(Some(crate::time::deadline_after(TIMEOUT)))?;
    let reply = Reply::decode(&msg.data).ok_or(Error::Corrupted)?;
    if let Reply::Error(code) = reply {
        return Err(Error::from_code(i64::from(code)).unwrap_or(Error::IoError));
    }
    Ok((reply, msg.handles))
}

/// Nove spojeni s libd (pro potomka).
pub fn connect() -> Result<Channel> {
    let (ours, theirs) = Channel::create()?;
    match call(&Request::Connect, alloc::vec![theirs.into_handle()])? {
        (Reply::Ok, _) => Ok(ours),
        _ => Err(Error::Corrupted),
    }
}

/// Knihovna `name` z `/system/lib`: rozlozeni a VMO segmentu (kod
/// zapeceteny ke sdileni, data jako sablona pro klon s kopii pri zapisu).
pub fn open(name: &str) -> Result<(Library, Vec<Vmo>)> {
    if !libdproto::valid_name(name) {
        return Err(Error::InvalidArgs);
    }
    match call(&Request::Open { name: alloc::string::String::from(name) }, Vec::new())? {
        (Reply::Library(l), handles) if handles.len() == l.segments.len() => {
            Ok((l, handles.into_iter().map(Vmo::from_handle).collect()))
        }
        _ => Err(Error::Corrupted),
    }
}

/// Statistika libd (sablony, zasahy mezipameti, spojeni).
pub fn stats() -> Result<Stats> {
    match call(&Request::Stats, Vec::new())? {
        (Reply::Stats(s), _) => Ok(s),
        _ => Err(Error::Corrupted),
    }
}
