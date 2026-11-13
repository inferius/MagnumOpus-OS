//! Sluzba "otevri cestu" (asociace typu, `libs/openwith`): spravce
//! souboru a shell posilaji `open <cesta>` a dostanou odpoved, ktera
//! aplikace cestu otevrela.
//!
//! - Kazda relace ma vlastni kanal; init z nej vi, cim identitou cestu
//!   overit (relace FS uzivatele) a do ktere relace pripadne spustit
//!   aplikaci. Aplikace relace dostanou duplikat koncu (`endpoint`).
//! - Asociace jsou `[open]` v manifestech aplikaci plochy
//!   (`gui::DESKTOP_APPS`).
//! - Aplikace se vola akci z manifestu pres kompozitor (kanal volajiciho
//!   `actions system`); kdyz nebezi, init ji spusti v relaci a volani
//!   zopakuje.
//! - Omezeni: kompozitor vybira aplikaci podle id, ne podle relace (vic
//!   prihlasenych uzivatelu soucasne zatim neni).

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use fsproto::{FsClient, kind};
use guiproto::action::Value;
use guiproto::control::{CallerMsg, CallerReply};
use libos::initfs::Initfs;
use libos::{Channel, Error, Handle, Result, Signals, Startup, println, time};
use openwith::msg::{Reply, parse_request};
use openwith::{Registry, Target};

use crate::supervisor::Supervisor;

/// Jak dlouho cekat na aplikaci, nez se zadost vzda.
const PENDING_NS: u64 = 20_000_000_000;

struct SessionOpen {
    session: u64,
    mine: Channel,
    theirs: Channel,
    fs: Option<FsClient>,
}

struct Pending {
    call: u32,
    session: u64,
    program: String,
    app: String,
    action: String,
    path: String,
    reveal: bool,
    reply: Channel,
    launched: bool,
    deadline: u64,
}

#[derive(Default)]
struct State {
    sessions: Vec<SessionOpen>,
    caller: Option<Channel>,
    pending: Vec<Pending>,
    registry: Option<Registry>,
    next_call: u32,
}

static OPEN: spin::Mutex<State> = spin::Mutex::new(State { sessions: Vec::new(), caller: None, pending: Vec::new(), registry: None, next_call: 1 });

/// Otevreni relace: novy kanal sluzby.
pub fn open_session(session: u64) -> Result<()> {
    let (mine, theirs) = Channel::create()?;
    OPEN.lock().sessions.push(SessionOpen { session, mine, theirs, fs: None });
    Ok(())
}

/// (Testy) Relace bez prihlaseni: kanal sluzby s danou relaci FS.
pub fn test_session(session: u64, fs: FsClient) -> Result<()> {
    let (mine, theirs) = Channel::create()?;
    OPEN.lock().sessions.push(SessionOpen { session, mine, theirs, fs: Some(fs) });
    Ok(())
}

/// (Testy) Kanal volajiciho ke kompozitoru testu (v testovacim behu neni
/// `gui::GUI`).
pub fn set_caller(c: Channel) {
    OPEN.lock().caller = Some(c);
}

pub fn close_session(session: u64) {
    let mut s = OPEN.lock();
    s.sessions.retain(|o| o.session != session);
    s.pending.retain(|p| p.session != session);
}

/// Konec kanalu sluzby pro aplikaci relace.
pub fn endpoint(session: u64) -> Result<Channel> {
    let s = OPEN.lock();
    let o = s.sessions.iter().find(|o| o.session == session).ok_or(Error::NotFound)?;
    Ok(Channel::from_handle(o.theirs.handle().duplicate(None)?))
}

/// Handly, na ktere ma supervisor cekat.
pub fn wait_handles(items: &mut Vec<(Handle, Signals)>) {
    let s = OPEN.lock();
    let rd = Signals::READABLE | Signals::PEER_CLOSED;
    for o in &s.sessions {
        if let Ok(h) = o.mine.handle().duplicate(None) {
            items.push((h, rd));
        }
    }
    if let Some(c) = &s.caller
        && let Ok(h) = c.handle().duplicate(None)
    {
        items.push((h, rd));
    }
}

fn registry() -> Registry {
    let fs: Option<&Initfs> = crate::install::STARTUP_INITFS.get();
    let texts: Vec<(&str, String)> = crate::gui::DESKTOP_APPS
        .iter()
        .filter_map(|p| {
            let t = fs?.find(&alloc::format!("{p}.manifest.toml"))?;
            Some((*p, String::from(core::str::from_utf8(t).ok()?)))
        })
        .collect();
    let (r, errors) = Registry::build(texts.iter().map(|(p, t)| (*p, t.as_str())));
    for e in errors {
        println!("init: asociace typu: {e}");
    }
    r
}

fn reply(ch: &Channel, r: Reply) {
    let _ = ch.write(&r.encode(), Vec::new());
}

/// Zadosti relaci a odpovedi kompozitoru (smycka supervisoru).
pub fn poll(startup: &Startup, sup: &mut Supervisor) {
    poll_with(Some((startup, sup)));
}

/// (Testy) Jako [`poll`], ale bez spousteni aplikaci (aplikace testu uz
/// bezi; aplikace, ktera nebezi = chyba).
pub fn poll_test() {
    poll_with(None);
}

fn poll_with(mut launcher: Option<(&Startup, &mut Supervisor)>) {
    let mut s = OPEN.lock();
    if s.registry.is_none() {
        s.registry = Some(registry());
    }
    // Zadosti.
    let mut requests: Vec<(u64, String, Channel)> = Vec::new();
    for o in &s.sessions {
        while let Ok(m) = o.mine.try_read() {
            let Some(reply_ch) = m.handles.into_iter().next().map(Channel::from_handle) else { continue };
            match parse_request(&m.data) {
                Some(p) => requests.push((o.session, String::from(p), reply_ch)),
                None => reply(&reply_ch, Reply::Failed(String::from("neplatná žádost"))),
            }
        }
    }
    for (session, path, reply_ch) in requests {
        request(&mut s, session, path, reply_ch);
    }
    // Odpovedi kompozitoru.
    let mut results = Vec::new();
    if let Some(c) = &s.caller {
        while let Ok(m) = c.try_read() {
            if let Ok(CallerReply::Result { call, ok, message, .. }) = CallerReply::decode(&m.data) {
                results.push((call, ok, message));
            }
        }
    }
    let mut launch = Vec::new();
    for (call, ok, message) in results {
        let Some(i) = s.pending.iter().position(|p| p.call == call) else { continue };
        let p = s.pending.remove(i);
        if !ok && message == "aplikace nebezi" && !p.launched {
            launch.push(p);
            continue;
        }
        let r = match (ok, p.reveal) {
            (true, false) => Reply::Opened(p.app.clone()),
            (true, true) => Reply::Revealed(p.app.clone()),
            (false, _) => Reply::Failed(alloc::format!("{}: {message}", p.app)),
        };
        reply(&p.reply, r);
    }
    // Zadosti, na ktere aplikace neodpovedela.
    let now = time::now();
    let (late, keep): (Vec<Pending>, Vec<Pending>) = core::mem::take(&mut s.pending).into_iter().partition(|p| now > p.deadline);
    s.pending = keep;
    for p in late {
        reply(&p.reply, Reply::Failed(alloc::format!("{} neodpověděla", p.app)));
    }
    // Spustit v relaci a zavolat znovu (kompozitor ma klienta hned po
    // spusteni; akci aplikace zpracuje, az se rozbehne). Bez zamku sluzby:
    // spusteni spravce souboru si bere konec kanalu sluzby (`endpoint`).
    drop(s);
    for mut p in launch {
        let r = match launcher.as_mut() {
            Some((startup, sup)) => crate::gui::GUI.lock().as_mut().map(|g| crate::gui::launch_app(g, startup, sup, &p.program, p.session)),
            None => Some(Err(Error::NotFound)),
        };
        match r {
            Some(Ok(_)) => {
                p.launched = true;
                send_call(&mut OPEN.lock(), p);
            }
            Some(Err(e)) => reply(&p.reply, Reply::Failed(alloc::format!("{} nejde spustit: {e}", p.app))),
            None => reply(&p.reply, Reply::Failed(String::from("grafické rozhraní neběží"))),
        }
    }
}

fn request(s: &mut State, session: u64, path: String, reply_ch: Channel) {
    let Some(o) = s.sessions.iter_mut().find(|o| o.session == session) else { return };
    if o.fs.is_none() {
        o.fs = crate::session::connect(session).ok();
    }
    let Some(fs) = &o.fs else {
        return reply(&reply_ch, Reply::Failed(String::from("relace nemá souborový systém")));
    };
    let st = match fs.stat(&path) {
        Ok(st) => st,
        Err(e) => return reply(&reply_ch, Reply::Failed(alloc::format!("{path}: {}", e.name()))),
    };
    let is_dir = matches!(st.kind, kind::DIR | kind::DATASET);
    let target = s.registry.as_ref().and_then(|r| r.resolve(&path, is_dir)).map(|t| (t.assoc().clone(), matches!(t, Target::Reveal(_))));
    let Some((a, reveal)) = target else {
        return reply(&reply_ch, Reply::Failed(alloc::format!("{path}: žádná aplikace ho neotevře")));
    };
    let p = Pending { call: 0, session, program: a.program, app: a.app, action: a.action, path, reveal, reply: reply_ch, launched: false, deadline: time::now() + PENDING_NS };
    send_call(s, p);
}

fn send_call(s: &mut State, mut p: Pending) {
    if s.caller.is_none() {
        s.caller = crate::gui::GUI.lock().as_ref().and_then(|g| g.channel(b"actions system").ok());
    }
    let Some(c) = &s.caller else {
        return reply(&p.reply, Reply::Failed(String::from("grafické rozhraní neběží")));
    };
    p.call = s.next_call;
    s.next_call = s.next_call.wrapping_add(1).max(1);
    let m = CallerMsg::Call { call: p.call, app: p.app.clone(), action: p.action.clone(), args: vec![(String::from("path"), Value::Path(p.path.clone()))], synthetic: false };
    if c.write(&m.encode(), Vec::new()).is_err() {
        s.caller = None;
        return reply(&p.reply, Reply::Failed(String::from("kompozitor neodpovídá")));
    }
    s.pending.push(p);
}
