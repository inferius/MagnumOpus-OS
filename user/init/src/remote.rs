//! Vzdalene relace (SSH, `docs/cs/ssh.md`): relace uctu po prihlaseni
//! klicem na dalku.
//!
//! Zaklada je accountd (`ToInit::OpenRemote`) az po overeni podpisu;
//! sshd dostane jen nahodny jednorazovy listek a relaci si jim vyzvedne
//! (`claim`). Vzdalena relace je oddelena od mistnich (`session::SESSIONS`):
//! nema plochu, kompozitor, schranku ani zvuk, supervisor ji nehlasi
//! kompozitoru a nepocita se do "aktivni relace" pro aplikace z listy.
//! Ma vlastni relaci FS s identitou uctu a vlastni job.

use alloc::string::String;
use alloc::vec::Vec;

use accountproto::ssh::TICKET;
use fsproto::FsClient;
use libos::job::Job;
use libos::{Channel, Error, Result, println};

pub struct RemoteSession {
    pub id: u64,
    pub uid: u64,
    pub name: String,
    /// Adresa klienta (indikator, audit).
    pub peer: String,
    pub fs: FsClient,
    /// Kanal self do accountd (nastaveni uctu v relaci).
    pub own: Option<Channel>,
    pub job: Job,
    ticket: [u8; TICKET],
    /// Listek uz pouzity (vyzvednout jde jen jednou).
    claimed: bool,
    /// Kdy zalozena (ns monotonniho casu; delka relace do auditu).
    pub started: u64,
}

pub static REMOTE: spin::Mutex<Vec<RemoteSession>> = spin::Mutex::new(Vec::new());

/// Zalozi vzdalenou relaci; vraci telo odpovedi accountd: `u64` relace
/// + listek.
pub(crate) fn open(id: u64, uid: u64, name: String, peer: String, fs: FsClient, own: Option<Channel>, job: Job) -> Result<Vec<u8>> {
    let mut ticket = [0u8; TICKET];
    libos::random::fill_seeded(&mut ticket)?;
    println!("init: vzdalena relace {id} uzivatele {uid} z {peer}");
    let mut out = id.to_le_bytes().to_vec();
    out.extend_from_slice(&ticket);
    REMOTE.lock().push(RemoteSession { id, uid, name, peer, fs, own, job, ticket, claimed: false, started: libos::time::now() });
    to_bar();
    Ok(out)
}

/// Porovnani bez zavislosti casu na obsahu (listek je tajemstvi).
fn same(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |d, (x, y)| d | (x ^ y)) == 0
}

/// Vyzvednuti relace listkem (sshd). Vraci cislo relace; listek plati
/// jednou.
pub fn claim(ticket: &[u8]) -> Result<u64> {
    let mut l = REMOTE.lock();
    let s = l.iter_mut().find(|s| !s.claimed && same(&s.ticket, ticket)).ok_or(Error::AccessDenied)?;
    s.claimed = true;
    s.ticket = [0; TICKET];
    Ok(s.id)
}

/// Konec vzdalene relace (accountd `CloseSession` nebo konec spojeni).
/// `false` = takova vzdalena relace neni.
pub fn close(session: u64) -> bool {
    let mut l = REMOTE.lock();
    let Some(i) = l.iter().position(|s| s.id == session) else { return false };
    let s = l.remove(i);
    drop(l);
    if let Err(e) = s.job.kill(libos::abi::process::EXIT_KILLED) {
        println!("init: job vzdalene relace {}: {e}", s.id);
    }
    to_bar();
    let secs = libos::time::now().saturating_sub(s.started) / 1_000_000_000;
    println!("init: vzdalena relace {} uzivatele {} z {} skoncila po {secs} s", s.id, s.uid, s.peer);
    true
}

/// Uzivatel, jmeno, duplikat jobu a dalsi relace FS vzdalene relace (pro
/// funkce `session::*`, ktere hledaji podle cisla relace).
pub fn uid(session: u64) -> Option<u64> {
    REMOTE.lock().iter().find(|s| s.id == session).map(|s| s.uid)
}

pub fn name(session: u64) -> Option<String> {
    REMOTE.lock().iter().find(|s| s.id == session).map(|s| s.name.clone())
}

pub fn job(session: u64) -> Result<Job> {
    REMOTE.lock().iter().find(|s| s.id == session).ok_or(Error::NotFound)?.job.duplicate(None)
}

pub fn connect(session: u64) -> Result<FsClient> {
    REMOTE.lock().iter().find(|s| s.id == session).ok_or(Error::NotFound)?.fs.connect()
}

/// Indikator v liste (pilulka v liste, prikaz `remote`
/// kompozitoru). Bez kompozitoru nic.
fn to_bar() {
    let (_, who) = summary();
    let cmd = alloc::format!("{} {}", guiproto::control::REMOTE, who.join(" "));
    if let Some(g) = crate::gui::GUI.lock().as_ref()
        && let Err(e) = g.command(cmd.trim_end().as_bytes(), alloc::vec::Vec::new())
    {
        println!("init: indikator vzdaleneho pristupu: {e}");
    }
}

/// Pocet vzdalenych relaci a adresy (indikator "pripojen vzdalene").
pub fn summary() -> (usize, Vec<String>) {
    let l = REMOTE.lock();
    (l.len(), l.iter().map(|s| alloc::format!("{}@{}", s.name, s.peer)).collect())
}
