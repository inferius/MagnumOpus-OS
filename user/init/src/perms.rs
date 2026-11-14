//! Povoleni aplikaci z shellu (prikaz `povoleni`, docs/cs/ucty.md, kapitola
//! Povoleni aplikaci): init se pta sluzby uctu jmenem uzivatele relace
//! terminalu (kanal `permissions-admin`). Shell s `accountd` primo
//! nemluvi a cizi rozhodnuti nevidi.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::time::Duration;

use accountproto::perm::{ChanMsg, Perm, decode_entries};
use libos::{Channel, Error, Result, time};

static ADMIN: spin::Mutex<Option<Channel>> = spin::Mutex::new(None);

fn call(m: &ChanMsg) -> Result<Vec<u8>> {
    let mut g = ADMIN.lock();
    if g.is_none() {
        *g = Some(crate::accounts::channel("permissions-admin")?);
    }
    let ch = g.as_ref().ok_or(Error::NotFound)?;
    let r = ch.write(&m.encode(), Vec::new()).and_then(|_| ch.read(Some(time::deadline_after(Duration::from_secs(5)))));
    match r {
        Ok(msg) => accountproto::parse_reply(&msg.data).map(<[u8]>::to_vec),
        Err(e) => {
            // Sluzba uctu se mohla restartovat: priste novy kanal.
            *g = None;
            Err(e)
        }
    }
}

/// Prikaz `povoleni` relace `session`: `app` prazdne = vypis, jinak
/// odvolani `perm` aplikace `app`. Vraci text pro terminal.
pub fn shell(session: Option<u64>, app: &str, perm: &str) -> Result<String> {
    let uid = session.and_then(crate::session::uid).ok_or(Error::AccessDenied)?;
    for_uid(uid, app, perm)
}

/// Jako [`shell`] pro uzivatele `uid` (testy).
pub fn for_uid(uid: u64, app: &str, perm: &str) -> Result<String> {
    if app.is_empty() {
        let list = decode_entries(&call(&ChanMsg::List { uid })?)?;
        if list.is_empty() {
            return Ok(String::from("Žádná uložená povolení."));
        }
        let mut s = String::new();
        for e in list {
            s.push_str(&format!("{}\t{}\t{}\n", e.app_id, e.perm.name(), if e.allow { "povoleno" } else { "zamítnuto" }));
        }
        return Ok(s);
    }
    let p = Perm::from_name(perm).ok_or(Error::InvalidArgs)?;
    call(&ChanMsg::Revoke { uid, app_id: String::from(app), perm: p })?;
    Ok(format!("Povolení {perm} pro {app} odvoláno (aplikace se příště zeptá znovu)."))
}
