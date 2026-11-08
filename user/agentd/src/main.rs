//! agentd: sluzba agentu AI (navrh `docs/cs/agent.md`).
//!
//! Spousti ho init (brana, `user/init/src/agent.rs`) podle manifestu
//! `agentd.manifest.toml`. Handly pri startu:
//!
//! 1. vlastni relace FS (`/state` -> `/system/agent`): stav, audit, klice,
//! 2. podle konfigurace: relace site (jen `api.anthropic.com:443`), relace
//!    FS uloziste certifikatu (`/certs`, jen cteni), kanal k `revocd`.
//!
//! Prvni zprava na bootstrap kanalu je konfigurace `start [net] [certs]
//! [rev]` (ktere handly prisly); agentd odpovi `ok`. Pak zpravy brany
//! `agentproto::proto::GateMsg`: `Open` (nova relace, handly relace,
//! odpoved `ok <cislo>` / `err <KOD>`) a `Provider` (sluzba registruje
//! nastroje). Kazda relace bezi ve vlastnim vlakne.
//!
//! `agentd` nema zadna prava k souborum uzivatelu: cte a zapisuje jen pres
//! capability, ktere brana vyda pro relaci.

#![no_std]
#![no_main]

extern crate alloc;

mod ext;
mod net;
mod session;
mod state;
mod tools;
mod workspace;

use alloc::format;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::time::Duration;

use agentproto::proto::GateMsg;
use fsproto::FsClient;
use libos::{Channel, Error, Startup, println, time};
use netproto::Net;
use tls::revocation::{ChannelSource, RevocationSource};

use session::Shared;

libos::entry!(main);

/// Zasobnik vlakna relace (kryptografie TLS pri volani poskytovatele).
const SESSION_STACK: u64 = 1024 * 1024;

fn main(startup: Startup) -> i64 {
    let gate = startup.bootstrap;
    let cfg = match gate.read(Some(time::deadline_after(Duration::from_secs(10)))) {
        Ok(m) => alloc::string::String::from_utf8_lossy(&m.data).into_owned(),
        Err(e) => {
            println!("agentd: chybi konfigurace: {e}");
            return 1;
        }
    };
    let has = |w: &str| cfg.split_whitespace().any(|x| x == w);
    let mut hs = startup.handles.into_iter();
    let Some(state_fs) = hs.next().map(|h| FsClient::new(Channel::from_handle(h))) else {
        let _ = gate.write(b"err NOT_FOUND bez stavu", Vec::new());
        return 1;
    };
    let net = if has("net") { hs.next().map(|h| Net::new(Channel::from_handle(h))) } else { None };
    let certs = if has("certs") { hs.next().map(|h| FsClient::new(Channel::from_handle(h))) } else { None };
    let revocation = if has("rev") {
        hs.next().map(|h| Arc::new(ChannelSource::new(Channel::from_handle(h), Duration::from_secs(20))) as Arc<dyn RevocationSource>)
    } else {
        None
    };
    let state = match state::State::open(state_fs) {
        Ok(s) => s,
        Err(e) => {
            println!("agentd: stav v /system/agent nejde otevrit: {e}");
            let _ = gate.write(format!("err {}", e.name()).as_bytes(), Vec::new());
            return 2;
        }
    };
    let net = net.map(|n| {
        let roots = web::load_roots(certs.as_ref());
        Arc::new(spin::Mutex::new(net::NetCtx { net: n, roots, revocation }))
    });
    let shared = Arc::new(Shared { state: spin::Mutex::new(state), providers: spin::Mutex::new(Vec::new()), net });
    println!("agentd: bezim (sit {})", if shared.net.is_some() { "ano" } else { "ne" });
    if gate.write(b"ok", Vec::new()).is_err() {
        return 3;
    }
    // Cisla relaci: cas startu v horni casti (audit se neplete po restartu).
    let base = (state::now().1.max(0) as u64) << 16;
    let mut next = 1u64;
    loop {
        let m = match gate.read(None) {
            Ok(m) => m,
            Err(Error::PeerClosed) => return 0,
            Err(e) => {
                println!("agentd: brana: {e}");
                return 4;
            }
        };
        let reply = match GateMsg::decode(&m.data) {
            Ok(GateMsg::Open(spec)) => {
                let id = base | (next & 0xFFFF);
                next += 1;
                let sh = shared.clone();
                let handles = m.handles;
                match libos::thread::spawn_sized("agent-relace", SESSION_STACK, move || session::run(sh, id, spec, handles)) {
                    Ok(_) => format!("ok {id}"),
                    Err(e) => format!("err {}", e.name()),
                }
            }
            Ok(GateMsg::Provider { name }) => match m.handles.into_iter().next().map(Channel::from_handle) {
                Some(ch) => match ext::Ext::register(&name, ch) {
                    Ok(p) => {
                        println!("agentd: poskytovatel nastroju {name} ({} nastroju)", p.tools.len());
                        shared.providers.lock().push(Arc::new(spin::Mutex::new(p)));
                        alloc::string::String::from("ok")
                    }
                    Err(e) => format!("err {}", e.name()),
                },
                None => alloc::string::String::from("err INVALID_ARGS"),
            },
            Err(e) => format!("err {}", e.name()),
        };
        if gate.write(reply.as_bytes(), Vec::new()).is_err() {
            return 0;
        }
    }
}
