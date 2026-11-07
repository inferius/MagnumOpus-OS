//! Supervisor: restart spadle sluzby a vzdani po opakovanych padech.
//!
//! Hotplug zarizeni pres supervisor overuji testy USB (`usb.rs`), ktere
//! ho ridi pres [`Supervisor::step`].

use alloc::string::String;
use alloc::vec::Vec;
use core::time::Duration;

use libos::{Startup, time};

use super::{Test, TestResult, fail};
use crate::supervisor::{Happened, RESTART_MAX, SUPERVISOR};
use crate::{check, check_eq};

pub const TESTS: &[Test] = &[
    ("pad sluzby casu: restart a odpoved", restart_time),
    ("opakovane pady: supervisor to vzda", give_up),
];

/// Jak dlouho cekat na reakci supervisoru (vcetne prodlevy restartu).
const TIMEOUT: Duration = Duration::from_secs(15);

/// Ukonci proces sluzby casu (jako pad).
fn kill_time() -> TestResult {
    let g = crate::system::TIME.lock();
    let t = g.as_ref().ok_or("sluzba casu nebezi")?;
    t.process.kill().map_err(|e| fail("ukonceni timed", e))
}

/// Ridi supervisor, dokud nenastane udalost splnujici `done`; vraci vse,
/// co se stalo.
fn step_until(s: &Startup, done: impl Fn(&Happened) -> bool) -> Result<Vec<Happened>, String> {
    let until = time::deadline_after(TIMEOUT);
    let mut all = Vec::new();
    loop {
        let h = SUPERVISOR
            .lock()
            .step(s, Some(time::deadline_after(Duration::from_millis(100))));
        let hit = h.iter().any(&done);
        all.extend(h);
        if hit {
            return Ok(all);
        }
        if time::now() > until {
            return Err(alloc::format!("supervisor nereagoval, stalo se: {all:?}"));
        }
    }
}

/// Sluzba casu odpovida (stejny pozadavek jako test nastaveni hodin).
fn time_answers() -> TestResult {
    let g = crate::system::TIME.lock();
    let t = g.as_ref().ok_or("sluzba casu nebezi")?;
    let now = time::wall().ok_or("realny cas neni znamy")?;
    t.channel
        .write(&super::time::set_request(now, false), Vec::new())
        .map_err(|e| fail("zapis", e))?;
    let reply = t
        .channel
        .read(Some(time::deadline_after(Duration::from_secs(5))))
        .map_err(|e| fail("odpoved nove sluzby", e))?;
    check!(reply.data.starts_with(b"ok"), "nova sluzba casu neodpovida ok");
    Ok(())
}

fn restart_time(s: &Startup) -> TestResult {
    check!(SUPERVISOR.lock().is_running("timed"), "supervisor sluzbu casu nehlida");
    kill_time()?;
    let got = step_until(s, |h| *h == Happened::Restarted("timed"))?;
    check!(
        got.iter()
            .any(|h| matches!(h, Happened::Exited { name: "timed", .. })),
        "supervisor nezaznamenal konec sluzby"
    );
    time_answers()
}

fn give_up(s: &Startup) -> TestResult {
    // Restart z minuleho testu se pocita do okna; po RESTART_MAX restartech
    // dalsi pad = vzdat.
    let mut gave_up = false;
    for _ in 0..RESTART_MAX + 2 {
        kill_time()?;
        let got = step_until(s, |h| {
            matches!(h, Happened::Restarted("timed") | Happened::GaveUp("timed"))
        })?;
        if got.contains(&Happened::GaveUp("timed")) {
            gave_up = true;
            break;
        }
    }
    check!(gave_up, "supervisor restartuje porad dokola");
    check!(!SUPERVISOR.lock().is_running("timed"), "vzdana sluzba bezi");
    // Rucni obnova: sluzba bezi a odpovida.
    check_eq!(SUPERVISOR.lock().revive("timed"), true, "obnova sluzby");
    step_until(s, |h| *h == Happened::Restarted("timed"))?;
    time_answers()
}
