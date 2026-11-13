//! Plocha prihlasenych relaci: reakce smycky supervisoru na udalosti
//! relaci (`crate::session::Event`).
//!
//! - **otevrena:** aplikace plochy (`gui::desktop_apps`) se spusti v jobu
//!   relace; terminal dostane identitu FS uzivatele,
//! - **skoncila (odhlaseni):** aplikace relace a termd jejich oken se
//!   ukonci,
//! - zamceni a odemceni zatim jen zaznamena (zamykaci obrazovku kresli
//!   kompozitor).
//!
//! Pred prihlasenim zadne aplikace uzivatele nebezi. Vyvojove automaticke
//! prihlaseni (`cargo xtask run`) otevre relaci hned pri startu.

use alloc::format;

use libos::{Startup, println};

use crate::gui::{GUI, desktop_apps, launch_app};
use crate::session::Event;
use crate::supervisor::Supervisor;

/// Zpracuje vsechny cekajici udalosti relaci.
pub fn on_events(startup: &Startup, sup: &mut Supervisor) {
    let Some(rx) = crate::session::events() else { return };
    while let Ok(m) = rx.try_read() {
        let Some(e) = Event::decode(&m.data) else { continue };
        match e {
            Event::Opened(s) => opened(startup, sup, s),
            Event::Closed(s) => closed(s),
            Event::Locked(s) => {
                println!("init: relace {s} zamcena");
                crate::gui::session_event(&format!("lock {s}"));
            }
            Event::Unlocked(s) => {
                println!("init: relace {s} odemcena");
                crate::gui::session_event(&format!("unlock {s}"));
            }
        }
    }
}

fn opened(startup: &Startup, sup: &mut Supervisor, session: u64) {
    let who = crate::session::name(session).unwrap_or_default();
    let uid = crate::session::uid(session).unwrap_or(0);
    println!("init: plocha relace {session} ({who})");
    // Kompozitor skryje prihlasovaci obrazovku.
    crate::gui::session_event(&format!("open {session} {uid} {who}"));
    let mut g = GUI.lock();
    let Some(gui) = g.as_mut() else {
        println!("init: relace {session} bez GUI (plocha se nespousti)");
        return;
    };
    // Sluzba "otevri cestu" a kopirovani souboru relace driv nez okna.
    if let Err(e) = crate::open::open_session(session) {
        println!("init: otevirani cest pro relaci {session}: {e}");
    }
    if let Err(e) = crate::files::start(startup, session) {
        println!("init: files-copy pro relaci {session} nelze spustit: {e}");
    }
    for app in desktop_apps() {
        if let Err(e) = launch_app(gui, startup, sup, app, session) {
            println!("init: {app} pro relaci {session} nelze spustit: {e}");
        }
    }
}

fn closed(session: u64) {
    if let Some(gui) = GUI.lock().as_mut() {
        gui.close_session(session);
    }
    crate::terminal::TERMINALS.lock().close_session(session);
    crate::files::close(session);
    crate::open::close_session(session);
    println!("init: procesy relace {session} ukonceny");
    crate::gui::session_event(&format!("close {session}"));
}
