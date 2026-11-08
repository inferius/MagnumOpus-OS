//! Obnova sluzby vstupu po padu (`docs/cs/obnova-sluzeb.md`): test
//! "zabij inputd uprostred prace" s hlavnim supervisorem initu.
//!
//! Test hraje ovladac (vlastni zdroj pres `device`) i vlastnika fokusu
//! a kanalu `secure` (init). Supervisor ridi pres `step`; vse ostatni ceka
//! s terminem, takze nic nevisi. Po testu bezi nova instance a dalsi sady
//! (USB, GUI) ji pouzivaji.

use alloc::string::String;
use alloc::vec::Vec;
use core::time::Duration;

use inputproto::code::*;
use inputproto::device::{self, DeviceMessage, RawEvent, caps};
use inputproto::{Action, Event, Record, control};
use libos::{Channel, Startup, time};

use super::input::{deadline, failr, has, input};
use super::{TestResult, fail};
use crate::input::{Input, drain, read_until};
use crate::supervisor::{Happened, SUPERVISOR};
use crate::{check, check_eq};

/// Nejdel na restart a na jednu udalost.
const TIMEOUT: Duration = Duration::from_secs(15);
const NAME: &str = "test obnovy vstupu";

/// Jmena fyzickych zdroju (bez testovaciho), serazena.
fn physical(inp: &Input) -> Result<Vec<String>, String> {
    let (text, _) = inp.command(control::SOURCES, Vec::new()).map_err(|e| fail("sources", e))?;
    let mut v: Vec<String> = control::SourceInfo::parse_list(&text)
        .filter(|s| !s.synthetic && s.name != NAME)
        .map(|s| String::from(s.name))
        .collect();
    v.sort();
    Ok(v)
}

fn key(r: &Record) -> Option<(u16, Action)> {
    match r.event {
        Event::Key { code, action, .. } => Some((code, action)),
        _ => None,
    }
}

fn send(ch: &Channel, events: &[RawEvent]) -> TestResult {
    ch.write(&device::encode_events(events), Vec::new())
        .map_err(|e| fail("zapis zdroje", e))
}

pub(super) fn kill_inputd(s: &Startup) -> TestResult {
    let inp = input()?;
    // Vlastni zdroj ("ovladac"): klavesnice.
    let (drv, srv) = Channel::create().map_err(|e| fail("kanal", e))?;
    let hello = device::encode_hello(caps::KEYBOARD, NAME);
    drv.write(&hello, Vec::new()).map_err(|e| fail("predstaveni", e))?;
    inp.command(control::DEVICE, alloc::vec![srv.into_handle()])
        .map_err(|e| fail("device", e))?;
    let end = deadline(TIMEOUT);
    while inp.source_id(NAME).is_err() {
        check!(time::now() < end, "testovaci zdroj se nepredstavil");
        time::sleep(Duration::from_millis(20));
    }
    let before = physical(inp)?;
    drain(&inp.focus);
    drain(&inp.secure);

    // Uprostred prace: klavesa A drzena.
    send(&drv, &[RawEvent::key(KEY_A, true), RawEvent::syn()])?;
    read_until(&inp.focus, deadline(TIMEOUT), |r| {
        r.iter().any(|r| key(r) == Some((KEY_A, Action::Press)))
    })
    .map_err(|e| failr("stisk A pred padem", e))?;

    // Pad: zabit inputd, supervisor ji spusti znovu.
    let mut sup = SUPERVISOR.lock();
    let generation = sup.generation("inputd").ok_or("inputd neni v registru")?;
    let p = sup.process("inputd").ok_or("inputd nebezi")?;
    libos::sys::process_kill(p.raw()).map_err(|e| fail("ukonceni inputd", e))?;
    // Stara zprava ve fronte: nova instance ji musi zahodit.
    send(&drv, &[RawEvent::key(KEY_C, true), RawEvent::syn()])?;
    let end = deadline(TIMEOUT);
    let mut seen = Vec::new();
    while !seen.contains(&Happened::Restarted("inputd")) {
        if time::now() > end {
            return Err(alloc::format!("inputd se neobnovila: {seen:?}"));
        }
        seen.extend(sup.step(s, Some(time::deadline_after(Duration::from_millis(200)))));
    }
    check_eq!(sup.generation("inputd"), Some(generation + 1), "generace inputd");
    drop(sup);

    // Synchronizace: nova instance posle ISYN, odpoved + predstaveni.
    let end = deadline(TIMEOUT);
    loop {
        let m = drv.read(Some(end)).map_err(|e| fail("ISYN od nove instance", e))?;
        if matches!(device::decode(&m.data), Some(DeviceMessage::Sync(_))) {
            device::answer_sync(&m.data, &hello, None, |b| drv.write(b, Vec::new()).is_ok());
            break;
        }
    }

    // Vlastnik fokusu: klavesy uvolnene (Focus(false)), pak bezpecny rezim
    // (stara instance mohla byt uprostred zabezpeceneho dialogu).
    read_until(&inp.secure, deadline(TIMEOUT), |r| has(r, Event::SecureAttention))
        .map_err(|e| failr("SecureAttention po restartu", e))?;
    check!(inp.status().map_err(|e| fail("status", e))?.secure, "po restartu neni bezpecny rezim");
    let got = read_until(&inp.focus, deadline(TIMEOUT), |r| {
        r.iter().filter(|r| r.event == Event::Focus(false)).count() >= 2
    })
    .map_err(|e| failr("Focus(false) po restartu", e))?;
    check!(
        !got.iter().any(|r| key(r).is_some()),
        "klavesa pred koncem bezpecneho rezimu"
    );
    inp.secure
        .write(control::RELEASE.as_bytes(), Vec::new())
        .map_err(|e| fail("release", e))?;
    read_until(&inp.focus, deadline(TIMEOUT), |r| has(r, Event::Focus(true)))
        .map_err(|e| failr("Focus(true) po release", e))?;

    // Uvolneni A patri stisku stare instance: zahodi se. B projde, stara
    // zprava (C) ne.
    send(&drv, &[RawEvent::key(KEY_A, false), RawEvent::syn()])?;
    send(&drv, &super::input::tap(KEY_B))?;
    let got = read_until(&inp.focus, deadline(TIMEOUT), |r| {
        r.iter().any(|r| key(r) == Some((KEY_B, Action::Release)))
    })
    .map_err(|e| failr("klavesa B po restartu", e))?;
    let keys: Vec<_> = got.iter().filter_map(key).collect();
    check_eq!(
        keys,
        alloc::vec![(KEY_B, Action::Press), (KEY_B, Action::Release)],
        "klavesy po restartu"
    );

    // Skutecne ovladace se synchronizovaly samy.
    let end = deadline(TIMEOUT);
    loop {
        let now = physical(inp)?;
        if now == before {
            break;
        }
        if time::now() > end {
            return Err(alloc::format!("zarizeni po restartu {now:?}, pred {before:?}"));
        }
        time::sleep(Duration::from_millis(100));
    }
    Ok(())
}
