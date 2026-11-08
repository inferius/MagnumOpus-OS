//! inputd: sluzba vstupu (klavesnice, mys).
//!
//! Od zdroju vstupu (ovladace zarizeni, umely vstup) prijima surove
//! udalosti (`inputproto::device`), prevadi je podle rozlozeni klavesnice
//! (`keymap`) a posila hotove udalosti (`inputproto::event`) **jen
//! vlastnikovi fokusu** (konzole, terminal, pozdeji kompozitor). Bezpecnou
//! zkratku Ctrl+Alt+Delete z fyzicke klavesnice zachyti sama a posle ji
//! jen na kanal `secure` (system), aplikace ji nevidi ani nepodvrhne.
//!
//! Ridi ji rodic (init) textovymi prikazy po bootstrap kanalu, viz
//! `inputproto::control`. Startovni zprava nenese zadne handly navic.
//!
//! Dokumentace: `docs/cs/vstup.md`, `docs/en/input.md`.

#![no_std]
#![no_main]

extern crate alloc;

mod service;
mod source;

use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use inputproto::control::{self, Status};
use inputproto::DeviceTrust;
use keymap::LayoutId;
use libos::{Channel, Error, Handle, Signals, Startup, println};

use service::{MAX_BATCH, Service};

libos::entry!(main);

fn main(startup: Startup) -> i64 {
    let control = startup.bootstrap;
    let mut svc = Service::new();
    println!("inputd: bezim (rozlozeni {})", svc.layout.name());
    loop {
        let mut items: Vec<(&Handle, Signals)> = Vec::with_capacity(svc.sources.len() + 3);
        let wake = Signals::READABLE | Signals::PEER_CLOSED;
        // Zdroje pridane prikazem behem tohoto kola se pridavaji na konec,
        // indexy techto `n` zustanou platne.
        let n = svc.sources.len();
        items.push((control.handle(), wake));
        for s in &svc.sources {
            items.push((s.channel.handle(), wake));
        }
        let secure_at = svc.secure.as_ref().map(|ch| {
            items.push((ch.channel.handle(), wake));
            items.len() - 1
        });
        let focus_at = svc.focus.as_ref().map(|ch| {
            items.push((ch.channel.handle(), wake));
            items.len() - 1
        });
        let observed = match libos::wait_many(&items, svc.next_deadline()) {
            Ok((_, o)) => o,
            Err(Error::TimedOut) => {
                svc.tick();
                continue;
            }
            Err(e) => {
                println!("inputd: cekani selhalo: {e}");
                return 1;
            }
        };
        drop(items);

        // Poradi je dulezite: signaly kanalu fokusu a `secure` patri
        // kanalum z doby cekani. Zpracuji se driv, nez je prikaz `focus` /
        // `secure` nize vymeni (jinak by stare PEER_CLOSED zahodilo novy
        // kanal).
        if let Some(at) = secure_at {
            secure_input(&mut svc, observed[at]);
        }
        if let Some(at) = focus_at
            && let Some(ch) = &svc.focus
        {
            if observed[at].intersects(Signals::READABLE) {
                // Klient zatim nic neposila; zpravy zahodit, at fronta neroste.
                while ch.channel.try_read().is_ok() {}
            } else if observed[at].intersects(Signals::PEER_CLOSED) {
                svc.focus = None;
            }
        }

        // Zdroje odzadu, aby odebrani nemenilo indexy dosud nezpracovanych.
        for i in (0..n).rev() {
            let sig = observed[i + 1];
            if sig.intersects(Signals::READABLE) {
                for _ in 0..MAX_BATCH {
                    match svc.sources[i].channel.try_read() {
                        Ok(m) => svc.source_message(i, &m.data),
                        Err(_) => break,
                    }
                }
            }
            if sig.intersects(Signals::PEER_CLOSED) && !sig.intersects(Signals::READABLE) {
                svc.remove_source(i);
            }
        }

        // Prikazy az nakonec: novy zdroj se prida za `n`, novy fokus
        // a `secure` uz neovlivni signaly z tohoto kola.
        if observed[0].intersects(Signals::READABLE) {
            while let Ok(msg) = control.try_read() {
                let (reply, handles) = command(&mut svc, &msg.data, msg.handles);
                if control.write(reply.as_bytes(), handles).is_err() {
                    break;
                }
            }
        } else if observed[0].intersects(Signals::PEER_CLOSED) {
            // Rodic skoncil: sluzba konci s nim.
            return 0;
        }
        svc.tick();
    }
}

/// Zpravy od vlastnika kanalu `secure` (`release`) a jeho zavreni.
fn secure_input(svc: &mut Service, sig: Signals) {
    let Some(ch) = &svc.secure else {
        return;
    };
    let mut release = false;
    let mut decisions = Vec::new();
    while let Ok(m) = ch.channel.try_read() {
        let text = core::str::from_utf8(&m.data).unwrap_or("");
        let mut w = text.split_whitespace();
        match (w.next(), w.next().and_then(|s| s.parse::<u16>().ok())) {
            (Some(control::RELEASE), None) => release = true,
            (Some(control::TRUST), Some(id)) => decisions.push((id, DeviceTrust::Trusted)),
            (Some(control::REJECT), Some(id)) => decisions.push((id, DeviceTrust::Rejected)),
            _ => {}
        }
    }
    for (id, state) in decisions {
        svc.decide(id, state);
    }
    if release {
        svc.leave_secure_mode();
    }
    if sig.intersects(Signals::PEER_CLOSED) && !sig.intersects(Signals::READABLE) {
        svc.secure_closed();
    }
}

fn err(e: Error) -> (String, Vec<Handle>) {
    (format!("err {}", e.name()), Vec::new())
}

fn ok() -> (String, Vec<Handle>) {
    (String::from("ok"), Vec::new())
}

/// Ridici prikaz od rodice; vraci odpoved a handly k ni.
fn command(svc: &mut Service, data: &[u8], mut handles: Vec<Handle>) -> (String, Vec<Handle>) {
    let Ok(text) = core::str::from_utf8(data) else {
        return err(Error::InvalidArgs);
    };
    let mut words = text.split_whitespace();
    let name = words.next().unwrap_or("");
    let channel = |h: &mut Vec<Handle>| {
        if h.len() == 1 {
            Ok(Channel::from_handle(h.remove(0)))
        } else {
            Err(Error::InvalidArgs)
        }
    };
    match name {
        control::DEVICE => {
            let sync = match words.next() {
                None => false,
                Some(control::SYNC) => true,
                Some(_) => return err(Error::InvalidArgs),
            };
            match channel(&mut handles).and_then(|c| svc.add_source(c, false)) {
                Ok(()) => {
                    if sync {
                        svc.start_sync(svc.sources.len() - 1, 1);
                    }
                    ok()
                }
                Err(e) => err(e),
            }
        }
        control::RESTARTED => {
            println!("inputd: instance po restartu");
            svc.set_restarted();
            ok()
        }
        control::FOCUS => match channel(&mut handles) {
            Ok(c) => {
                svc.set_focus(c);
                ok()
            }
            Err(e) => err(e),
        },
        control::SECURE => match channel(&mut handles) {
            Ok(c) => {
                svc.set_secure(c);
                ok()
            }
            Err(e) => err(e),
        },
        control::INJECT => {
            let r = Channel::create().and_then(|(mine, theirs)| {
                svc.add_source(mine, true)?;
                Ok(theirs)
            });
            match r {
                Ok(theirs) => (String::from("ok"), vec![theirs.into_handle()]),
                Err(e) => err(e),
            }
        }
        control::LAYOUT => match words.next().and_then(LayoutId::from_name) {
            Some(id) => {
                svc.set_layout(id);
                ok()
            }
            None => err(Error::NotFound),
        },
        control::STATUS => {
            let mut st = Status {
                devices: svc.sources.len() as u32,
                keyboards: svc.sources.iter().filter(|s| s.is_keyboard()).count() as u32,
                pointers: svc.sources.iter().filter(|s| s.is_pointer()).count() as u32,
                injecting: svc.injecting(),
                dropped: svc.dropped,
                secure: svc.secure_mode,
                ..Status::default()
            };
            let name = svc.layout.name().as_bytes();
            st.layout[..name.len()].copy_from_slice(name);
            let mut text = String::from("ok ");
            let _ = st.format(&mut text);
            (text, Vec::new())
        }
        control::SOURCES => {
            let mut text = String::from("ok\n");
            for s in &svc.sources {
                let info = inputproto::control::SourceInfo {
                    id: s.id,
                    caps: s.caps,
                    synthetic: s.synthetic,
                    trust: s.trust,
                    name: &s.name,
                };
                let _ = info.format(&mut text);
            }
            (text, Vec::new())
        }
        control::POLICY => match (words.next(), words.next()) {
            (Some(control::VERIFY_NEW_DEVICES), Some(v @ ("on" | "off"))) => {
                svc.verify_new = v == "on";
                ok()
            }
            _ => err(Error::InvalidArgs),
        },
        control::SETTLED => {
            svc.settled = true;
            ok()
        }
        _ => err(Error::NotSupported),
    }
}
