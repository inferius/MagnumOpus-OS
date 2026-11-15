//! Systemove soubory (`/system`): sluzba ukonu spravy `SYSTEM_FILE`
//! (`accountproto::elevate`, `docs/cs/ucty.md`, kapitola Sprava pocitace).
//!
//! Lide do `/system` nemaji zadne pravo; zapisuje jen init svou relaci FS.
//! Prikaz shellu `system write|rm|mkdir` posle zadost initu
//! (`powerproto::Request::SystemFile`). Init zkontroluje cestu (pod
//! `/system/`, nikdy `/system/accounts`), ve vlakne pozada za relaci
//! shellu o `ELEVATE` (systemovy dialog, potvrdi clovek s roli spravy),
//! grant overi a spotrebuje na kanalu `grants` a teprve pak zmenu provede.
//! Grant plati jen pro tuto operaci, cestu a obsah (delka a XXH3-128).
//! Zadna jina cesta do `/system` pro lidi neni.

use alloc::string::String;
use alloc::vec::Vec;

use accountproto::client::SelfService;
use accountproto::elevate::{FileOp, SystemFile, action, encode_check, system_path};
use fsproto::{FsClient, kind};
use libos::{Channel, Error, Result, println, time};
use powerproto::Reply;

fn fs() -> Result<&'static FsClient> {
    crate::system::devices().and_then(|d| d.fs.as_ref()).ok_or(Error::NotFound)
}

/// Parametry ukonu pro data, ktera init zapise.
pub fn params(op: FileOp, path: &str, data: &[u8]) -> SystemFile {
    let (len, hash) = match op {
        FileOp::Write => (data.len() as u64, xxhash_rust::xxh3::xxh3_128(data)),
        _ => (0, 0),
    };
    SystemFile { op, path: String::from(path), len, hash }
}

/// Zadost z relace shellu. `Ok(kanal)` = ceka se na potvrzeni, vysledek
/// prijde na kanal; spatna cesta nebo operace = `Err` hned (bez dialogu).
pub fn from_shell(session: Option<u64>, op: u8, path: &str, data: Vec<u8>) -> core::result::Result<Channel, Reply> {
    let fail = Reply::Failed;
    let op = FileOp::from_u8(op).map_err(fail)?;
    system_path(path).map_err(fail)?;
    if op != FileOp::Write && !data.is_empty() {
        return Err(fail(Error::InvalidArgs));
    }
    let session = session.ok_or(fail(Error::AccessDenied))?;
    let (mine, theirs) = Channel::create().map_err(fail)?;
    let path = String::from(path);
    libos::thread::spawn("system-soubor", move || {
        let reply = match elevate(session, op, &path, &data) {
            Ok(()) => Reply::Ok,
            Err(e) => Reply::Failed(e),
        };
        let _ = mine.write(&reply.encode(), Vec::new());
    })
    .map_err(fail)?;
    Ok(theirs)
}

/// Ukon spravy za relaci `session`: dialog (nebo jina politika accountd),
/// grant overit, provest. Vola i test sady "ucty".
pub fn elevate(session: u64, op: FileOp, path: &str, data: &[u8]) -> Result<()> {
    system_path(path)?;
    let p = params(op, path, data).encode();
    let own = crate::session::take_self(session).ok_or(Error::BadState)?;
    let me = SelfService(own);
    let r = me.elevate(action::SYSTEM_FILE, p.clone());
    crate::session::put_self(session, me.0);
    let token = r?;
    // Grant overi a spotrebuje sluzba uctu (jako u ostatnich sluzeb).
    let grants = crate::accounts::channel("grants")?;
    grants.write(&encode_check(&token, action::SYSTEM_FILE, &p), Vec::new())?;
    let m = grants.read(Some(time::deadline_after(core::time::Duration::from_secs(5))))?;
    accountproto::parse_reply(&m.data)?;
    apply(fs()?, op, path, data)?;
    println!("init: system: {} {path} (relace {session})", what(op));
    Ok(())
}

fn what(op: FileOp) -> &'static str {
    match op {
        FileOp::Write => "zapsano",
        FileOp::Remove => "smazano",
        FileOp::Mkdir => "vytvoreno",
    }
}

/// Provedeni relaci FS initu (az po overenem grantu).
fn apply(fs: &FsClient, op: FileOp, path: &str, data: &[u8]) -> Result<()> {
    match op {
        // Atomicky (docasny soubor a prejmenovani); nadrazena slozka musi
        // existovat.
        FileOp::Write => crate::certs::store(fs, path, data).map(|_| ()),
        FileOp::Remove => fs.remove(path),
        FileOp::Mkdir => fs.create(path, kind::DIR),
    }
}
