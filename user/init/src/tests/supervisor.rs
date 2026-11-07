//! Supervisor: restart spadle sluzby a vzdani po opakovanych padech.
//!
//! Hotplug zarizeni pres supervisor overuji testy USB (`usb.rs`), ktere
//! ho ridi pres [`Supervisor::step`].

use alloc::string::String;
use alloc::vec::Vec;
use core::time::Duration;

use libos::abi::process::EXIT_KILLED;
use libos::{Channel, Startup, time};

use super::{Test, TestResult, fail};
use crate::supervisor::{Happened, RESTART_MAX, Restart, SUPERVISOR, Started, Supervisor};
use crate::system::{TimeService, spawn_time};
use crate::{check, check_eq};

pub const TESTS: &[Test] = &[
    ("pad sluzby casu: restart a odpoved", restart_time),
    ("opakovane pady: supervisor to vzda", give_up),
    ("restart bez potvrzeni: supervisor sluzbu ukonci", silent_restart),
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

/// Procesy a kanaly testu `silent_restart` (drzet, dokud test bezi).
static HELD: spin::Mutex<Vec<(TimeService, Channel)>> = spin::Mutex::new(Vec::new());

/// Restart, ktery start nikdy nepotvrdi: bezici sluzba casu, ale kanal
/// potvrzeni je cizi a nikdo do nej nepise.
fn silent_start(s: &Startup) -> libos::Result<Started> {
    let t = spawn_time(s)?;
    let (ready, peer) = Channel::create()?;
    let process = t.process.handle().duplicate(None)?;
    HELD.lock().push((t, peer));
    Ok(Started {
        process,
        ready: Some(ready.into_handle()),
    })
}

fn silent_restart(s: &Startup) -> TestResult {
    let mut sup = Supervisor::for_tests(Duration::from_millis(300));
    let t = spawn_time(s).map_err(|e| fail("start timed", e))?;
    sup.watch(
        "tichy",
        &t.process,
        Some(Restart {
            start: silent_start,
            ready: Some(|| Ok(())),
        }),
    );
    t.process.kill().map_err(|e| fail("ukonceni", e))?;
    let result = (|| -> TestResult {
        let until = time::deadline_after(TIMEOUT);
        let mut exits = Vec::new();
        let mut was_starting = false;
        while exits.len() < 2 {
            let h = sup.step(s, Some(time::deadline_after(Duration::from_millis(50))));
            was_starting |= sup.is_starting("tichy");
            for e in h {
                match e {
                    Happened::Exited { code, .. } => exits.push(code),
                    other => {
                        return Err(alloc::format!("necekana udalost {other:?}"));
                    }
                }
            }
            if time::now() > until {
                return Err(alloc::format!("supervisor tichou sluzbu neukoncil: {exits:?}"));
            }
        }
        check!(was_starting, "supervisor na potvrzeni necekal");
        // Prvni konec = nase ukonceni, druhy = supervisor po lhute.
        check_eq!(exits[1], EXIT_KILLED, "kod ukoncene sluzby");
        check!(!sup.is_starting("tichy"), "sluzba po ukonceni dal ceka");
        Ok(())
    })();
    for (t, _) in HELD.lock().drain(..) {
        let _ = t.process.kill();
    }
    result
}
