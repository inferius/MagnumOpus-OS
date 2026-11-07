//! svctest: testovaci sluzba a klient pro testy obnovy po padu
//! (`user/init/src/tests/recovery.rs`, `docs/cs/obnova-sluzeb.md`).
//!
//! Role urci prvni zprava na bootstrap kanalu:
//!
//! - `serve <generace> <rezim>` - **sluzba**. Prvni handle startovni zpravy
//!   je kanal udalosti (sluzba tam hlasi `held`). Rezim: `normal`,
//!   `slow:<ms>` (potvrzeni startu az po prodleve), `crash` (hned skonci).
//!   Po startu posle na bootstrap `ready`. Ridici prikaz `connect <kdo>`
//!   vrati kanal relace s identitou `kdo`. Pozadavky relace:
//!   - `who` -> `gen=<g> who=<kdo>`,
//!   - `hold` a `bump` -> v generaci 1 odpoved nikdy neprijde (sluzba ohlasi
//!     `held` a ceka, az ji test zabije); v dalsich generacich hned
//!     `gen=<g> who=<kdo>` (`bump` = pozadavek s ucinkem),
//!   - `close-me` -> sluzba relaci zavre (umyslne zavreni).
//! - `client` - **klient**. Handly: kanal relace a listek. Prikazy na
//!   bootstrap: `call idem <pozadavek>`, `call effect <pozadavek>`,
//!   `narrow <pripona>` (dal pracuje se zuzenou relaci), `wait <ms>`,
//!   `gen`. Odpoved `ok <text>` nebo `err <JMENO_CHYBY>`.

#![no_std]
#![no_main]

extern crate alloc;

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::time::Duration;

use libos::{Channel, Error, Signals, Startup, println, time};
use svcclient::{Conn, Kind};

libos::entry!(main);

fn main(startup: Startup) -> i64 {
    let first = match startup.bootstrap.read(None) {
        Ok(m) => m,
        Err(_) => return 1,
    };
    let text = String::from_utf8_lossy(&first.data).into_owned();
    let mut words = text.split_whitespace();
    match words.next() {
        Some("serve") => {
            let generation = words.next().and_then(|g| g.parse().ok()).unwrap_or(1);
            let mode = words.next().unwrap_or("normal");
            serve(startup, generation, mode)
        }
        Some("client") => client(startup),
        _ => {
            println!("svctest: neznama role {text:?}");
            2
        }
    }
}

struct Session {
    channel: Channel,
    who: String,
    /// Pozadavek drzeny bez odpovedi (generace 1).
    held: bool,
}

fn serve(startup: Startup, generation: u64, mode: &str) -> i64 {
    if mode == "crash" {
        return 3;
    }
    let events = match startup.handles.into_iter().next() {
        Some(h) => Channel::from_handle(h),
        None => return 4,
    };
    if let Some(ms) = mode.strip_prefix("slow:").and_then(|m| m.parse().ok()) {
        time::sleep(Duration::from_millis(ms));
    }
    let control = startup.bootstrap;
    if control.write(b"ready", Vec::new()).is_err() {
        return 5;
    }
    let mut sessions: Vec<Session> = Vec::new();
    loop {
        let mut items = alloc::vec![(control.handle(), Signals::READABLE | Signals::PEER_CLOSED)];
        for s in &sessions {
            items.push((s.channel.handle(), Signals::READABLE | Signals::PEER_CLOSED));
        }
        let observed = match libos::wait_many(&items, None) {
            Ok((_, o)) => o,
            Err(_) => return 6,
        };
        drop(items);
        if observed[0].intersects(Signals::READABLE) {
            match control.try_read() {
                Ok(m) => {
                    let t = String::from_utf8_lossy(&m.data).into_owned();
                    let Some(who) = t.strip_prefix("connect ") else { continue };
                    let reply = Channel::create().map(|(mine, theirs)| {
                        sessions.push(Session {
                            channel: mine,
                            who: String::from(who),
                            held: false,
                        });
                        theirs
                    });
                    let _ = match reply {
                        Ok(ch) => control.write(b"ok", alloc::vec![ch.into_handle()]),
                        Err(e) => control.write(format!("err {}", e.name()).as_bytes(), Vec::new()),
                    };
                }
                Err(Error::ShouldWait) => {}
                Err(_) => return 0,
            }
        } else if observed[0].intersects(Signals::PEER_CLOSED) {
            return 0;
        }
        let mut closed = Vec::new();
        for (k, sig) in observed.iter().enumerate().skip(1) {
            let s = &mut sessions[k - 1];
            if !sig.intersects(Signals::READABLE | Signals::PEER_CLOSED) {
                continue;
            }
            let m = match s.channel.try_read() {
                Ok(m) => m,
                Err(Error::ShouldWait) => continue,
                Err(_) => {
                    closed.push(k - 1);
                    continue;
                }
            };
            let req = String::from_utf8_lossy(&m.data).into_owned();
            let answer = format!("gen={generation} who={}", s.who);
            match req.as_str() {
                "who" => {
                    let _ = s.channel.write(answer.as_bytes(), Vec::new());
                }
                "hold" | "bump" if generation == 1 => {
                    s.held = true;
                    let _ = events.write(b"held", Vec::new());
                }
                "hold" | "bump" => {
                    let _ = s.channel.write(answer.as_bytes(), Vec::new());
                }
                "close-me" => closed.push(k - 1),
                _ => {
                    let _ = s.channel.write(b"?", Vec::new());
                }
            }
        }
        for k in closed.into_iter().rev() {
            sessions.remove(k);
        }
    }
}

fn client(startup: Startup) -> i64 {
    let mut h = startup.handles.into_iter();
    let (Some(session), Some(lease)) = (h.next(), h.next()) else {
        return 7;
    };
    let mut conn = Conn::new("svctest", Channel::from_handle(session), Some(Channel::from_handle(lease)));
    let control = startup.bootstrap;
    loop {
        let m = match control.read(None) {
            Ok(m) => m,
            Err(_) => return 0,
        };
        let cmd = String::from_utf8_lossy(&m.data).into_owned();
        let reply = command(&mut conn, &cmd);
        let text = match reply {
            Ok(t) => format!("ok {t}"),
            Err(e) => format!("err {}", e.name()),
        };
        if control.write(text.as_bytes(), Vec::new()).is_err() {
            return 0;
        }
    }
}

fn command(conn: &mut Conn, cmd: &str) -> libos::Result<String> {
    let mut w = cmd.splitn(3, ' ');
    match (w.next(), w.next(), w.next()) {
        (Some("call"), Some(kind), Some(req)) => {
            let kind = if kind == "effect" { Kind::Effect } else { Kind::Idempotent };
            let m = conn.call(kind, req.as_bytes(), Vec::new())?;
            Ok(String::from_utf8_lossy(&m.data).into_owned())
        }
        (Some("narrow"), Some(suffix), None) => {
            let (ch, lease) = conn.narrow(suffix.as_bytes())?;
            *conn = Conn::new("svctest", ch, Some(lease));
            Ok(String::new())
        }
        (Some("wait"), Some(ms), None) => {
            conn.set_wait(Duration::from_millis(ms.parse().map_err(|_| Error::InvalidArgs)?));
            Ok(String::new())
        }
        (Some("gen"), None, None) => Ok(format!("{}", conn.generation())),
        _ => Err(Error::InvalidArgs),
    }
}
