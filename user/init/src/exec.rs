//! Sluzba "exec": spousteni programu ze shellu relace uzivatele
//! (`launchproto::exec`, `docs/cs/navrh-shell.md`, kapitola Programy).
//!
//! - Shell (`shsess`) nema sit ani pravo razit relace. Kazda relace
//!   uzivatele (mistni i vzdalena) ma vlastni kanal sluzby; init z nej vi,
//!   za kterou relaci (uzivatel, skupiny, job) program spustit. Okna
//!   terminalu relace dostanou duplikat konce (`endpoint`), termd ho preda
//!   svym relacim shellu. Odpoved jde kanalem, ktery prisel v pozadavku
//!   (vic relaci shellu na jednom kanalu se nemicha).
//! - Program se hleda a spousti pres launchd (jeden registr prikazu: initfs,
//!   `/system/bin`); init je rodic: dodava job relace, relaci FS
//!   s identitou uzivatele (zuzenou pripojenymi cestami od shellu) a sit.
//!   Sit dostane jen program s `[network]` v manifestu
//!   (`NET_NEEDS_MANIFEST`), zuzenou manifestem; firewall rozhoduje podle id
//!   aplikace.
//! - Kazda relace ma vlakno sluzby; skonci, kdyz zavrou vsechny konce
//!   kanalu (konec relace: `close_session` zahodi konec initu, odhlaseni
//!   ukonci termd i shsess).

use alloc::string::String;
use alloc::vec::Vec;
use core::time::Duration;

use fsproto::FsClient;
use fsproto::perm::{Mount, mount_flags};
use launchproto::exec::{Exec, reply};
use launchproto::{Launch, Role, flags, is_bare_name, parse_reply};
use libos::{Channel, Error, Handle, Result, println, time};
use netproto::auth::Restriction;
use netproto::policy::{Policy, rights};

/// Konce kanalu sluzby, ktere init dava oknum relace (duplikaty).
static ENDPOINTS: spin::Mutex<Vec<(u64, Channel)>> = spin::Mutex::new(Vec::new());

/// Kanal sluzby relace `session` (duplikat; prvni volani zalozi kanal
/// a vlakno sluzby).
pub fn endpoint(session: u64) -> Result<Channel> {
    crate::session::uid(session).ok_or(Error::NotFound)?;
    let mut l = ENDPOINTS.lock();
    if let Some((_, theirs)) = l.iter().find(|(s, _)| *s == session) {
        return Ok(Channel::from_handle(theirs.handle().duplicate(None)?));
    }
    let (mine, theirs) = Channel::create()?;
    libos::thread::spawn("exec", move || serve(session, mine))?;
    let d = Channel::from_handle(theirs.handle().duplicate(None)?);
    l.push((session, theirs));
    Ok(d)
}

/// Konec relace: init zahodi svuj konec (vlakno skonci, az zavrou i okna).
pub fn close_session(session: u64) {
    ENDPOINTS.lock().retain(|(s, _)| *s != session);
}

fn serve(session: u64, ch: Channel) {
    let mut launchd: Option<Channel> = None;
    loop {
        let m = match ch.read(None) {
            Ok(m) => m,
            Err(Error::PeerClosed) => break,
            Err(e) => {
                println!("init: exec relace {session}: {e}");
                break;
            }
        };
        let mut hs = m.handles.into_iter();
        let Some(back) = hs.next().map(Channel::from_handle) else { continue };
        let stdio: Vec<Handle> = hs.by_ref().take(3).collect();
        let (tty, prompt) = (hs.next(), hs.next().map(Channel::from_handle));
        let r = match Exec::decode(&m.data) {
            Ok(req) if stdio.len() == 3 => run(session, &req, stdio, tty, prompt, &mut launchd),
            Ok(_) => Err((Error::InvalidArgs, String::from("chybi stdio"))),
            Err(e) => Err((e, String::from("neplatny pozadavek"))),
        };
        let _ = match r {
            Ok(proc_handles) => back.write(&reply(Ok(())), proc_handles),
            Err((e, msg)) => back.write(&reply(Err((e, &msg))), Vec::new()),
        };
    }
    println!("init: exec relace {session} skoncil");
}

type Fail = (Error, String);

fn err(e: Error, msg: impl Into<String>) -> Fail {
    (e, msg.into())
}

/// Spusti program; vraci handly pro shell `[proces, kanal k potomkovi]`.
fn run(session: u64, req: &Exec, stdio: Vec<Handle>, tty: Option<Handle>, prompt: Option<Channel>, launchd: &mut Option<Channel>) -> core::result::Result<Vec<Handle>, Fail> {
    let program = req.program.as_str();
    // v1: jen programy systemu z registru launchd (initfs, /system/bin).
    if !is_bare_name(program) {
        return Err(err(Error::NotSupported, alloc::format!("{program}: spusteni cestou zatim neni (jen programy systemu)")));
    }
    let uid = crate::session::uid(session).ok_or_else(|| err(Error::NotFound, "relace skoncila"))?;
    let fs = session_fs(session, uid, &req.mounts).map_err(|e| err(e, "relace FS programu"))?;
    let job = crate::session::job(session).map_err(|e| err(e, "job relace"))?;
    let net = if req.net { program_net().map_err(|e| err(e, "relace site"))? } else { None };
    // Prostredi: domov a jmeno uzivatele relace (napr. `~/.ssh` klienta SSH).
    let name = crate::session::name(session).unwrap_or_default();
    let env = [alloc::format!("HOME=/home/{name}"), alloc::format!("USER={name}")];
    let params = params(program, &req.cwd, &req.args, &env).ok_or_else(|| err(Error::InvalidArgs, "prilis dlouhe argumenty"))?;
    let mut roles = alloc::vec![Role::Job, Role::Fs];
    let mut handles: Vec<Handle> = alloc::vec![job.into_handle(), fs.into_channel().into_handle()];
    if let Some(n) = net {
        roles.push(Role::Net);
        handles.push(n);
    }
    for (role, h) in [Role::Stdin, Role::Stdout, Role::Stderr].into_iter().zip(stdio) {
        roles.push(role);
        handles.push(h);
    }
    if let Some(t) = tty {
        roles.push(Role::Tty);
        handles.push(t);
    }
    // Klient SSH: kanal podpisu u sluzby uctu (soukromy klic program nema)
    // a jen program z initfs (jmeno si nikdo jiny nevezme).
    let mut fl = flags::NET_NEEDS_MANIFEST;
    if launchproto::exec::SSH_PROGRAMS.contains(&program) {
        fl |= flags::INITFS_ONLY;
        let initfs = crate::install::STARTUP_INITFS.get().ok_or_else(|| err(Error::NotFound, "initfs"))?;
        let app = crate::apps::manifest(initfs, program)
            .ok()
            .flatten()
            .map(|m| m.id)
            .ok_or_else(|| err(Error::NotFound, alloc::format!("{program}: chybi manifest v systemu")))?;
        let (ours, theirs) = Channel::create().map_err(|e| err(e, "kanal podpisu"))?;
        crate::accounts::agent(session, uid, &app, theirs, prompt).map_err(|e| err(e, "sluzba uctu (podpis SSH)"))?;
        roles.push(Role::Agent);
        handles.push(ours.into_handle());
    }
    let l = Launch { program: String::from(program), flags: fl, roles, params };
    let msg = l.encode(1).map_err(|e| err(e, "pozadavek launchd"))?;
    let ch = match launchd.take() {
        Some(c) => c,
        None => crate::launch::client().map_err(|e| err(e, "spousteci sluzba nebezi"))?,
    };
    ch.write(&msg, handles).map_err(|e| err(e, "spousteci sluzba"))?;
    let m = ch.read(Some(time::deadline_after(Duration::from_secs(30)))).map_err(|e| err(e, "spousteci sluzba neodpovida"))?;
    *launchd = Some(ch);
    let (_, r) = parse_reply(&m.data).map_err(|e| err(e, "odpoved spousteci sluzby"))?;
    match r {
        Ok(_) => {
            println!("init: exec {program} v relaci {session} (uzivatel {uid})");
            Ok(m.handles)
        }
        Err((e, msg)) => Err((e, if msg.is_empty() { alloc::format!("{program}: {e}") } else { msg })),
    }
}

/// Relace FS programu: identita uzivatele relace; s pripojenymi cestami
/// od shellu jen ony (zuzeni, nikdy vic nez uzivatel).
fn session_fs(session: u64, uid: u64, mounts: &[launchproto::exec::Mount]) -> Result<FsClient> {
    if mounts.is_empty() {
        return crate::session::connect(session);
    }
    let groups = crate::session::groups(session).ok_or(Error::NotFound)?;
    let ctl = crate::system::devices().and_then(|d| d.fs.as_ref()).ok_or(Error::ServiceUnavailable)?;
    let list: Vec<Mount> = mounts
        .iter()
        .map(|m| Mount::new(&m.name, &m.target, if m.read_only { mount_flags::READ_ONLY } else { 0 }))
        .collect();
    Ok(ctl.connect_as(uid, &groups, &list)?.1)
}

/// Sit pro program: bezna prava klienta i serveru a `RESTRICT` (launchd
/// ji zuzi manifestem programu); bez nastaveni a auditu. Bez site = `None`.
fn program_net() -> Result<Option<Handle>> {
    let Some(net) = crate::system::NET.get() else { return Ok(None) };
    let r = Restriction::new(rights::CLIENT | rights::TCP_LISTEN | rights::PING | rights::RESTRICT, Policy::default());
    Ok(Some(net.root.restrict_with(&r)?.into_channel()?.into_handle()))
}

/// Blok parametru: `argv[0]` = jmeno programu, prostredi `env`.
fn params(program: &str, cwd: &str, args: &[String], env: &[String]) -> Option<Vec<u8>> {
    let argv: Vec<&[u8]> = core::iter::once(program.as_bytes()).chain(args.iter().map(|a| a.as_bytes())).collect();
    let env: Vec<&[u8]> = env.iter().map(|e| e.as_bytes()).collect();
    let len = libos::abi::params::encoded_len(cwd.as_bytes(), argv.iter().copied(), env.iter().copied(), &[]);
    let mut out = alloc::vec![0u8; len];
    libos::abi::params::encode(&mut out, 0, cwd.as_bytes(), argv.iter().copied(), env.iter().copied(), &[])?;
    Some(out)
}
