//! Plocha prihlasenych relaci: reakce smycky supervisoru na udalosti
//! relaci (`crate::session::Event`).
//!
//! - **otevrena:** aplikace plochy (`gui::DESKTOP_APPS`) se spusti v jobu
//!   relace; terminal dostane identitu FS uzivatele,
//! - **skoncila (odhlaseni):** aplikace relace a termd jejich oken se
//!   ukonci,
//! - zamceni a odemceni zatim jen zaznamena (zamykaci obrazovku kresli
//!   kompozitor).
//!
//! Pred prihlasenim zadne aplikace uzivatele nebezi. Vyvojove automaticke
//! prihlaseni (`cargo xtask run`) otevre relaci hned pri startu.

use libos::{Startup, println};

use crate::gui::{DESKTOP_APPS, GUI, launch_app};
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
            Event::Locked(s) => println!("init: relace {s} zamcena"),
            Event::Unlocked(s) => println!("init: relace {s} odemcena"),
        }
    }
}

fn opened(startup: &Startup, sup: &mut Supervisor, session: u64) {
    let who = crate::session::name(session).unwrap_or_default();
    println!("init: plocha relace {session} ({who})");
    let mut g = GUI.lock();
    let Some(gui) = g.as_mut() else {
        println!("init: relace {session} bez GUI (plocha se nespousti)");
        return;
    };
    for app in DESKTOP_APPS {
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
    println!("init: procesy relace {session} ukonceny");
}
