//! Zvuk: sluzba `audiod` pod supervisorem.
//!
//! audiod dostane vsechny kanaly k ovladacum (`audio` od devmgr) a sam
//! vybere zarizeni. Supervisor drzi duplikaty techto kanalu ([`Kept`]):
//! kdyz audiod spadne, ovladace nic nepoznaji a nova instance dostane
//! stejne kanaly. Ovladac pripojeny (nebo restartovany) za behu prijde od
//! devmgr dalsi zpravou `audio`; init ho prida do seznamu a bezici
//! instanci posle zpravu `device` ([`driver_added`]).
//!
//! Bootstrap kanal sluzby je ridici (jen init); klienti dostavaji relace
//! pres `CONNECT` - bez listku ([`connect`], testy) nebo s listkem pres
//! supervisor (`session("audiod", [])`, termd), ktera prezije restart.
//!
//! Role `Audio` (relace zvuku pro potomky, `docs/cs/zvuk.md`): adapter
//! prijme jen prazdne zuzeni (stejna identita a pravidla nahravani), novy
//! listek je svazany s rodicem; [`revoke`] zrusi listky identity i z nich
//! odvozene a audiod zavre vsechny jeji relace.

use alloc::boxed::Box;
use alloc::vec::Vec;
use core::time::Duration;

use audioproto::client::Request;
use libos::spawn::StartHandle;
use libos::{Channel, Error, Process, Result, Signals, Startup, println, time};

use crate::supervisor::{Kept, Service, Spec, StartCtx, Started};

/// Jak dlouho cekat na odpoved sluzby na `CONNECT`.
const REPLY_TIMEOUT: Duration = Duration::from_secs(5);

struct State {
    /// Kanaly k ovladacum (duplikaty pro kazdou instanci).
    kept: Vec<Kept>,
    /// Ridici kanal bezici instance.
    control: Option<Channel>,
}

static STATE: spin::Mutex<State> = spin::Mutex::new(State { kept: Vec::new(), control: None });

/// `CONNECT` na ridicim kanalu: novy kanal klienta (bez listku).
fn connect_on(control: &Channel) -> Result<Channel> {
    // Zbytky (napr. "ok" pri startu) zahodit.
    while control.try_read().is_ok() {}
    control.write(&Request::Connect.encode(), Vec::new())?;
    let m = control.read(Some(time::deadline_after(REPLY_TIMEOUT)))?;
    audioproto::decode_reply(&m.data)?;
    m.handles.into_iter().next().map(Channel::from_handle).ok_or(Error::Corrupted)
}

/// Test obnovy: aktivni ovladac zvukove karty skonci jako po padu.
pub fn crash_driver() -> Result<()> {
    let st = STATE.lock();
    st.control.as_ref().ok_or(Error::ServiceUnavailable)?.write(b"crash-driver", Vec::new())
}

/// Relace aplikace (`audioproto::session::Recipe`) na ridicim kanalu.
fn app_session_on(control: &Channel, recipe: &[u8]) -> Result<Channel> {
    while control.try_read().is_ok() {}
    let mut m = Vec::from(&b"app\0"[..]);
    m.extend_from_slice(recipe);
    control.write(&m, Vec::new())?;
    let r = control.read(Some(time::deadline_after(REPLY_TIMEOUT)))?;
    audioproto::decode_reply(&r.data)?;
    r.handles.into_iter().next().map(Channel::from_handle).ok_or(Error::Corrupted)
}

/// Spoji audiod se sluzbou uctu kvuli povolenim nahravani (kanal
/// `permissions`): pri startu a po kazdem restartu audiod. Bez sluzby uctu
/// nic (nahravani pro aplikace pak neni).
fn link_permissions(control: &Channel) {
    match crate::accounts::channel("permissions") {
        Ok(c) => {
            if let Err(e) = control.write(b"perm", alloc::vec![c.into_handle()]) {
                println!("init: zvuk: kanal povoleni: {e}");
            }
        }
        Err(Error::NotFound) => {}
        Err(e) => println!("init: zvuk: kanal povoleni od sluzby uctu: {e}"),
    }
}

/// Relace zvuku pro aplikaci s oknem: kanal a listek (`tag::AUDIO`,
/// `tag::AUDIO_LEASE`). Recept nese identitu a prava (`session.rs`).
pub fn app_session(sup: &mut crate::supervisor::Supervisor, recipe: &audioproto::session::Recipe) -> Result<(Channel, Channel)> {
    sup.session("audiod", recipe.encode())
}

/// Odvola zvuk aplikaci `app` (id z manifestu): supervisor zrusi listky
/// jeji identity u audiod i z nich odvozene (role `Audio` potomku), audiod
/// zavre vsechny jeji relace (streamy zmlknou). Vraci (pocet listku, pocet
/// relaci).
pub fn revoke(sup: &mut crate::supervisor::Supervisor, app: &str) -> Result<(usize, usize)> {
    let leases = sup.revoke_leases("audiod", |r| audioproto::session::Recipe::decode(r).is_ok_and(|x| x.is_some_and(|x| x.app_id == app)));
    let st = STATE.lock();
    let control = st.control.as_ref().ok_or(Error::ServiceUnavailable)?;
    while control.try_read().is_ok() {}
    let mut m = Vec::from(&b"revoke\0"[..]);
    m.extend_from_slice(app.as_bytes());
    control.write(&m, Vec::new())?;
    let r = control.read(Some(time::deadline_after(REPLY_TIMEOUT)))?;
    let n = core::str::from_utf8(audioproto::decode_reply(&r.data)?).ok().and_then(|t| t.parse().ok()).unwrap_or(0);
    println!("init: zvuk aplikace {app} odvolan ({leases} listku, {n} relaci)");
    Ok((leases, n))
}

/// Bezici audiod i v supervisoru testu `sup` (relace zvuku s listky pro
/// programy, ktere test spousti a jejichz supervisor ridi sam: testy Wine).
/// Hlavni supervisor dal hlida restart.
pub fn register_for_tests(sup: &mut crate::supervisor::Supervisor) -> Result<()> {
    let h = crate::supervisor::SUPERVISOR.lock().process("audiod").ok_or(Error::ServiceUnavailable)?.duplicate(None)?;
    let p = Process::from_handle(h);
    sup.register(Spec { name: "audiod", deps: &[], service: Some(Box::new(Audiod)) }, Some(&p));
    Ok(())
}

/// Relace aplikace bez listku (testy: program spousteny testem).
pub fn app_channel(recipe: &audioproto::session::Recipe) -> Result<Channel> {
    let st = STATE.lock();
    app_session_on(st.control.as_ref().ok_or(Error::ServiceUnavailable)?, &recipe.encode())
}

/// Novy kanal ke zvukove sluzbe bez listku (testy).
pub fn connect() -> Result<Channel> {
    let st = STATE.lock();
    connect_on(st.control.as_ref().ok_or(Error::ServiceUnavailable)?)
}

/// Spusti proces audiod s duplikaty kanalu k ovladacum.
fn spawn(startup: &Startup, kept: &[Kept]) -> Result<(Process, Channel)> {
    let mut handles = Vec::new();
    for k in kept {
        handles.push(StartHandle::user(k.give()?.into_handle()));
    }
    let app = crate::apps::spawn(startup, "audiod", handles)?;
    Ok((app.process, app.control))
}

/// Adapter pro supervisor.
struct Audiod;

impl Service for Audiod {
    fn start(&mut self, ctx: &StartCtx) -> Result<Started> {
        let mut st = STATE.lock();
        // Kanaly mrtvych ovladacu nova instance nepotrebuje.
        st.kept.retain(|k| k.give().is_ok_and(|c| !closed(&c)));
        let (process, control) = spawn(ctx.startup, &st.kept)?;
        let started = Started { process: process.handle().duplicate(None)?, ready: Some(control.handle().duplicate(None)?) };
        link_permissions(&control);
        st.control = Some(control);
        // Proces drzi supervisor (Started); nas handle muze zaniknout.
        drop(process);
        Ok(started)
    }

    fn ready(&mut self) -> Result<bool> {
        let st = STATE.lock();
        let control = st.control.as_ref().ok_or(Error::BadState)?;
        match control.try_read() {
            Ok(m) if m.data == b"ok" => Ok(true),
            Ok(_) => Err(Error::IoError),
            Err(Error::ShouldWait) => Ok(false),
            Err(e) => Err(e),
        }
    }

    fn connect(&mut self, recipe: &[u8]) -> Result<Channel> {
        let st = STATE.lock();
        let control = st.control.as_ref().ok_or(Error::ServiceUnavailable)?;
        // Prazdny recept = systemova relace (termd, lista), jinak aplikace.
        if recipe.is_empty() { connect_on(control) } else { app_session_on(control, recipe) }
    }

    /// Prava relace urcuje recept (aplikace / system). Zuzeni jen prazdne
    /// (role Audio pro potomka: stejny recept, nahravani dal jen pres
    /// dialog povoleni, nikdy sirsi).
    fn narrow(&self, parent: &[u8], narrowing: &[u8]) -> Result<Vec<u8>> {
        if !narrowing.is_empty() {
            return Err(Error::NotSupported);
        }
        // Recept aplikace overit (systemova relace = prazdny recept).
        if !parent.is_empty() {
            audioproto::session::Recipe::decode(parent)?;
        }
        Ok(parent.to_vec())
    }

    fn stopped(&mut self) {
        STATE.lock().control = None;
    }
}

/// Je druhy konec kanalu zavreny?
fn closed(c: &Channel) -> bool {
    libos::wait_one(c.handle(), Signals::PEER_CLOSED, Some(0)).is_ok()
}

/// Spusti audiod nad ovladaci `devices` (i prazdne: sluzba bezi a streamy
/// odmita, aby klienti dostali jasnou chybu) a zaregistruje ho
/// v supervisoru (restart po padu, relace s listky).
pub fn start(startup: &Startup, devices: Vec<Channel>, sup: &mut crate::supervisor::Supervisor) -> Result<()> {
    let n = devices.len();
    let kept: Vec<Kept> = devices.into_iter().map(Kept::new).collect();
    let (process, control) = spawn(startup, &kept)?;
    let msg = control.read(Some(time::deadline_after(Duration::from_secs(10))))?;
    if msg.data != b"ok" {
        let _ = process.kill();
        return Err(Error::Corrupted);
    }
    link_permissions(&control);
    {
        let mut st = STATE.lock();
        st.kept = kept;
        st.control = Some(control);
    }
    sup.register(Spec { name: "audiod", deps: &[], service: Some(Box::new(Audiod)) }, Some(&process));
    println!("init: zvukova sluzba bezi ({n} zvukovych karet)");
    Ok(())
}

/// Ovladac zvukove karty pripojeny za behu (nebo po restartu ovladace):
/// schovat pro dalsi instance a poslat ho bezici sluzbe.
pub fn driver_added(chan: Channel) {
    let mut st = STATE.lock();
    st.kept.retain(|k| k.give().is_ok_and(|c| !closed(&c)));
    let kept = Kept::new(chan);
    let dup = match kept.give() {
        Ok(d) => d,
        Err(e) => {
            println!("init: kanal k ovladaci zvuku: {e}");
            return;
        }
    };
    st.kept.push(kept);
    if let Some(c) = st.control.as_ref()
        && let Err(e) = c.write(b"device", alloc::vec![dup.into_handle()])
    {
        println!("init: zvukove sluzbe nejde predat ovladac: {e}");
    }
}
