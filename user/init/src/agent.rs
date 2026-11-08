//! Brana agenta AI (navrh `docs/cs/agent.md`).
//!
//! - **Start** (`start`): pripravi `/system/agent` (vlastnik ucet `agentd`,
//!   jen on), spusti `agentd` podle manifestu a preda mu jeho relaci FS,
//!   v beznem behu i relaci site (jen `api.anthropic.com:443`), uloziste
//!   certifikatu a kanal k `revocd`.
//! - **Relace** (`open`): pro uzivatele vyrazi capability - FS jen pro
//!   cteni (`/ -> /` READ_ONLY), FS pracovniho prostoru (`P -> P`) a kanal
//!   systemovych nastroju - a preda je `agentd` spolu s kanalem klienta.
//!   Klient dostane druhy konec. `agentd` si o prava nemuze rict sam.
//! - **Systemove nastroje** (`sys.services`, `sys.crashes`, `sys.log`,
//!   `sys.net`): vlakno initu odpovida ze snimku stavu supervisoru
//!   ([`publish`]); jen cteni.

use alloc::collections::VecDeque;
use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use core::time::Duration;

use agentproto::proto::{GateMsg, OpenSpec};
use agentproto::registry::ProviderMsg;
use agentproto::tool::{Effect, Schema, ToolSpec};
use fsproto::perm::{ACL_PROTECTED, Acl, Grant, Mount, grant, mount_flags, rights};
use fsproto::{FsClient, kind};
use libos::spawn::StartHandle;
use libos::{Channel, Error, Process, Result, Startup, println, time};

use crate::system::Devices;

/// Ucet sluzby `agentd` (vlastnik `/system/agent`).
pub const AGENTD_UID: u64 = 65_533;
pub const STATE_DIR: &str = "/system/agent";
const REPLY_WAIT: Duration = Duration::from_secs(10);
/// Kolik udalosti supervisoru si brana pamatuje pro `sys.log`.
const EVENTS_MAX: usize = 200;

pub struct Agentd {
    pub process: Process,
    gate: Channel,
}

pub static AGENTD: spin::Mutex<Option<Agentd>> = spin::Mutex::new(None);

/// Snimek stavu sluzeb a posledni udalosti supervisoru (pro `sys.*`).
static STATUS: spin::Mutex<Vec<String>> = spin::Mutex::new(Vec::new());
static EVENTS: spin::Mutex<VecDeque<String>> = spin::Mutex::new(VecDeque::new());

/// Supervisor po kazdem kole zverejni stav a udalosti.
pub fn publish(status: &[crate::supervisor::ServiceStatus], happened: &[crate::supervisor::Happened]) {
    let lines = status
        .iter()
        .map(|s| {
            format!(
                "{} stav={:?} generace={} restarty={} posledni_konec={} duvod={}",
                s.name,
                s.state,
                s.generation,
                s.restarts,
                s.last_exit.map_or(String::from("-"), |c| libos::abi::process::describe_exit(c).into()),
                s.reason.unwrap_or("-")
            )
        })
        .collect();
    *STATUS.lock() = lines;
    if !happened.is_empty() {
        let t = libos::time::wall().map_or(0, |w| w.secs);
        let mut ev = EVENTS.lock();
        for h in happened {
            if ev.len() >= EVENTS_MAX {
                ev.pop_front();
            }
            ev.push_back(format!("{t} {h:?}"));
        }
    }
}

/// Pripravi adresar stavu: vlastni ho `agentd`, nikdo jiny do nej nevidi.
fn prepare(fs: &FsClient) -> Result<()> {
    crate::system::grant_traverse(fs, "/", fsproto::perm::EVERYONE)?;
    for p in ["/system", STATE_DIR] {
        match fs.create(p, kind::DIR) {
            Ok(()) | Err(Error::AlreadyExists) => {}
            Err(e) => return Err(e),
        }
    }
    crate::system::grant_traverse(fs, "/system", fsproto::perm::EVERYONE)?;
    fs.set_owner(STATE_DIR, AGENTD_UID)?;
    let acl = Acl { flags: ACL_PROTECTED, grants: vec![Grant { who: AGENTD_UID, rights: rights::ALL, flags: grant::BOTH }] };
    fs.set_acl(STATE_DIR, Some(&acl))
}

/// Spusti `agentd`. `with_net` = predat sit a uloziste certifikatu (bezny
/// beh); testy bezi bez site (falesny poskytovatel).
pub fn start(startup: &Startup, devices: &Devices, with_net: bool) -> Result<()> {
    let fs = devices.fs.as_ref().ok_or(Error::NotFound)?;
    prepare(fs)?;
    let (_, state) = fs.connect_as(AGENTD_UID, &[], &[Mount::new("/state", STATE_DIR, 0)])?;
    let mut handles = vec![StartHandle::user(state.into_channel().into_handle())];
    let mut cfg = String::from("start");
    if with_net && let Some(n) = crate::system::NET.get() {
        let initfs = libos::initfs::Initfs::open(startup)?;
        let m = crate::apps::manifest(&initfs, "agentd")?.ok_or(Error::NotFound)?;
        if let Some(net) = crate::apps::net_session(&n.root, &m)? {
            handles.push(StartHandle::user(net.into_channel().into_handle()));
            cfg.push_str(" net");
            if let Ok(c) = crate::certs::session(devices) {
                handles.push(StartHandle::user(c.into_channel().into_handle()));
                cfg.push_str(" certs");
            }
            if let Some(r) = crate::revocation::client() {
                handles.push(StartHandle::user(r.into_handle()));
                cfg.push_str(" rev");
            }
        }
    }
    let app = crate::apps::spawn(startup, "agentd", handles)?;
    app.control.write(cfg.as_bytes(), Vec::new())?;
    let reply = app.control.read(Some(time::deadline_after(REPLY_WAIT)))?;
    if reply.data != b"ok" {
        println!("init: agentd: {}", String::from_utf8_lossy(&reply.data));
        let _ = app.process.kill();
        return Err(Error::BadState);
    }
    *AGENTD.lock() = Some(Agentd { process: app.process, gate: app.control });
    if let Err(e) = start_terminal_gate() {
        println!("init: brana agenta pro terminal: {e}");
    }
    Ok(())
}

/// Uzivatel relaci agenta z terminalu. Dokud neni prihlaseni, bezi terminal
/// s identitou systemu; relaci FS se systemovou identitou ale vydat nejde
/// (a nema jit), proto agent z terminalu jedna jako vychozi uzivatel 1000 -
/// ma mene prav nez terminal (viz `docs/cs/agent.md`).
pub const TERMINAL_USER: u64 = 1000;

/// Zapisovaci konec brany terminalu (termd ho duplikuje relacim shsess).
static TERM_GATE: spin::Mutex<Option<Channel>> = spin::Mutex::new(None);

/// Duplikat zapisovaciho konce brany terminalu pro termd.
pub fn terminal_gate() -> Option<libos::Handle> {
    TERM_GATE.lock().as_ref().and_then(|c| c.handle().duplicate(None).ok())
}

/// Spusti vlakno brany terminalu: zadost = `OpenSpec` + kanal odpovedi.
fn start_terminal_gate() -> Result<()> {
    let (ours, theirs) = Channel::create()?;
    libos::thread::spawn("agent-brana", move || {
        while let Ok(m) = ours.read(None) {
            let Some(reply) = m.handles.into_iter().next().map(Channel::from_handle) else { continue };
            let spec = OpenSpec::get(&mut wire::Reader::new(&m.data));
            let r = spec.map_err(|_| Error::InvalidArgs).and_then(|mut s| {
                // Shell potvrzeni neumi a skript falesneho modelu jen pro poskytovatele fake.
                s.can_confirm = false;
                open(TERMINAL_USER, &[], s, true)
            });
            let _ = match r {
                Ok((client, id)) => reply.write(format!("ok {id}").as_bytes(), vec![client.into_handle()]),
                Err(e) => reply.write(format!("err {}", e.name()).as_bytes(), Vec::new()),
            };
        }
    })?;
    *TERM_GATE.lock() = Some(theirs);
    Ok(())
}

/// Odpoved agentd na zpravu brany: `ok [cislo]` nebo `err KOD`.
fn gate_call(msg: &GateMsg, handles: Vec<libos::Handle>) -> Result<u64> {
    let guard = AGENTD.lock();
    let a = guard.as_ref().ok_or(Error::ServiceUnavailable)?;
    a.gate.write(&msg.encode(), handles)?;
    let r = a.gate.read(Some(time::deadline_after(REPLY_WAIT)))?;
    let text = String::from_utf8_lossy(&r.data).into_owned();
    match text.split_once(' ').unwrap_or((text.as_str(), "")) {
        ("ok", rest) => Ok(rest.trim().parse().unwrap_or(0)),
        (_, code) => Err(error_named(code)),
    }
}

fn error_named(name: &str) -> Error {
    (-64..0).filter_map(|c| Error::from_code(c)).find(|e| e.name() == name.trim()).unwrap_or(Error::IoError)
}

/// Otevre relaci agenta pro uzivatele. Vraci kanal klienta a cislo relace.
/// `spec.workspace` (neprazdny) = pracovni prostor (dataset).
pub fn open(user: u64, groups: &[u64], mut spec: OpenSpec, sys_tools: bool) -> Result<(Channel, u64)> {
    let fs = crate::system::devices().and_then(|d| d.fs.as_ref()).ok_or(Error::NotFound)?;
    spec.user = user;
    let (_, read) = fs.connect_as(user, groups, &[Mount::new("/", "/", mount_flags::READ_ONLY)])?;
    let (client, theirs) = Channel::create()?;
    let mut handles = vec![theirs.into_handle(), read.into_channel().into_handle()];
    spec.has_ws = !spec.workspace.is_empty();
    if spec.has_ws {
        let ws = agentproto::policy::normalize(&spec.workspace)?;
        let (_, wfs) = fs.connect_as(user, groups, &[Mount::new(&ws, &ws, 0)])?;
        spec.workspace = ws;
        handles.push(wfs.into_channel().into_handle());
    }
    spec.has_sys = sys_tools;
    if sys_tools {
        let (ours, theirs) = Channel::create()?;
        libos::thread::spawn("agent-sys", move || serve_sys(ours))?;
        handles.push(theirs.into_handle());
    }
    let id = gate_call(&GateMsg::Open(spec), handles)?;
    Ok((client, id))
}

/// Zaregistruje poskytovatele nastroju (kanal, na kterem posle `Register`).
pub fn register_provider(name: &str, chan: Channel) -> Result<()> {
    gate_call(&GateMsg::Provider { name: String::from(name) }, vec![chan.into_handle()]).map(|_| ())
}

fn sys_tools() -> Vec<ToolSpec> {
    let none = || Schema::object().build();
    vec![
        ToolSpec::new("sys.services", "Stav systemovych sluzeb ze supervisoru (stav, generace, restarty, posledni konec).", Effect::Read, none()),
        ToolSpec::new("sys.crashes", "Sluzby, ktere spadly nebo se restartovaly, s kodem konce a duvodem.", Effect::Read, none()),
        ToolSpec::new("sys.log", "Posledni udalosti supervisoru (pady, restarty, obnovy relaci).", Effect::Read,
            Schema::object().integer("max", "nejvic udalosti (vychozi 50)", false).build()),
        ToolSpec::new("sys.net", "Stav site: adresa, brana, DNS, linka.", Effect::Read, none()),
    ]
}

fn sys_call(tool: &str, input: &agentproto::json::Value) -> (String, bool) {
    match tool {
        "sys.services" => (STATUS.lock().join("\n"), false),
        "sys.crashes" => {
            let lines: Vec<String> = STATUS
                .lock()
                .iter()
                .filter(|l| !l.contains("restarty=0 ") || !l.contains("posledni_konec=-"))
                .cloned()
                .collect();
            (if lines.is_empty() { String::from("zadne pady") } else { lines.join("\n") }, false)
        }
        "sys.log" => {
            let max = input.u64_of("max");
            let max = if max == 0 { 50 } else { max.min(EVENTS_MAX as u64) } as usize;
            let ev = EVENTS.lock();
            let skip = ev.len().saturating_sub(max);
            let lines: Vec<String> = ev.iter().skip(skip).cloned().collect();
            (if lines.is_empty() { String::from("zadne udalosti") } else { lines.join("\n") }, false)
        }
        "sys.net" => match crate::system::NET.get().map(|n| n.root.info()) {
            Some(Ok(i)) => (
                format!(
                    "linka={} adresa={:?} brana={:?} dns={:?} mtu={}",
                    if i.link_up { "ano" } else { "ne" },
                    i.addr,
                    i.gateway,
                    i.dns,
                    i.mtu
                ),
                false,
            ),
            Some(Err(e)) => (format!("chyba {}", e.name()), true),
            None => (String::from("sluzba site nebezi"), true),
        },
        _ => (format!("neznamy nastroj {tool}"), true),
    }
}

/// Vlakno systemovych nastroju jedne relace (konec kanalu = konec).
fn serve_sys(ch: Channel) {
    if ch.write(&ProviderMsg::Register { tools: sys_tools() }.encode(), Vec::new()).is_err() {
        return;
    }
    while let Ok(m) = ch.read(None) {
        let reply = match ProviderMsg::decode(&m.data) {
            Ok(ProviderMsg::Call { id, tool, input }) => {
                let v = agentproto::json::parse(&input).unwrap_or_else(|_| agentproto::json::Value::object());
                let (content, is_error) = sys_call(&tool, &v);
                ProviderMsg::Result { id, is_error, content }
            }
            _ => continue,
        };
        if ch.write(&reply.encode(), Vec::new()).is_err() {
            return;
        }
    }
}
