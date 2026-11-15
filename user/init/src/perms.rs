//! Povoleni aplikaci z shellu (prikaz `permissions`, docs/cs/ucty.md, kapitola
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

/// Prikaz `permissions` relace `session`: `app` prazdne = vypis, jinak
/// odvolani `perm` (slovo shellu, napr. `microphone`) aplikace `app`.
/// Vypis: radek na rozhodnuti `aplikace\tpovoleni\tallow|deny` (shell
/// z nej udela tabulku), prazdny text = zadne rozhodnuti.
pub fn shell(session: Option<u64>, app: &str, perm: &str) -> Result<String> {
    let uid = session.and_then(crate::session::uid).ok_or(Error::AccessDenied)?;
    for_uid(uid, app, perm)
}

/// Jako [`shell`] pro uzivatele `uid` (testy).
pub fn for_uid(uid: u64, app: &str, perm: &str) -> Result<String> {
    if app.is_empty() {
        let list = decode_entries(&call(&ChanMsg::List { uid })?)?;
        let mut s = String::new();
        for e in list {
            s.push_str(&format!("{}\t{}\t{}\n", e.app_id, shell_word(e.perm), if e.allow { "allow" } else { "deny" }));
        }
        return Ok(s);
    }
    let p = from_shell_word(perm).ok_or(Error::InvalidArgs)?;
    call(&ChanMsg::Revoke { uid, app_id: String::from(app), perm: p })?;
    Ok(format!("Povolení {perm} pro {app} odvoláno (aplikace se příště zeptá znovu)."))
}

/// Ucty lidi pro jmena v prikazu `acl`: radek na ucet
/// `uid\tjmeno\tzobrazovane jmeno`. Zdroj je kanal accountd `names`
/// (`NAMES`, jen jmena; stejny jako karta Opravneni v Souborech). Shell
/// kanal nedostava primo: duplikat sdileny relacemi by michal odpovedi,
/// dotaz jde kanalem napajeni, ktery ma kazda relace vlastni.
pub fn accounts() -> Result<String> {
    let names = accountproto::client::Names(crate::accounts::channel("names")?);
    let mut s = String::new();
    for a in names.names()? {
        s.push_str(&format!("{}\t{}\t{}\n", a.uid, a.name, a.display));
    }
    Ok(s)
}

/// Jmeno povoleni v prikazu shellu (anglicky jako vsechna jmena prikazu
/// a voleb; ulozeny tvar `Perm::name` zustava).
fn shell_word(p: Perm) -> &'static str {
    match p {
        Perm::Microphone => "microphone",
    }
}

fn from_shell_word(s: &str) -> Option<Perm> {
    match s {
        "microphone" => Some(Perm::Microphone),
        _ => None,
    }
}
