//! Obnova sluzeb po padu: protokol listku, rozpracovane pozadavky,
//! zavislosti (testovaci sluzba `svctest`, kostra v `recovery_kit.rs`).
//!
//! Kazdy test ma vlastni supervisor a vlastni instance `svctest`, hlavni
//! supervisor initu se nemeni.

use alloc::format;
use alloc::string::String;

use libos::Startup;

use super::recovery_kit::Harness;
use super::{Test, TestResult};
use crate::supervisor::{Dep, Happened, State};
use crate::{check, check_eq};

pub const TESTS: &[Test] = &[
    ("idempotentni pozadavek prezije pad", hold_idempotent),
    ("pozadavek s ucinkem vrati SERVICE_RESTARTED", hold_effect),
    ("zuzeny listek zustane zuzeny", narrowed),
    ("vzdana sluzba: SERVICE_UNAVAILABLE hned", gave_up),
    ("pomaly restart: SERVICE_UNAVAILABLE do lhuty klienta", slow_restart),
    ("umyslne zavreni relace: PEER_CLOSED", deliberate_close),
    ("zavislosti: poradi restartu a oznameni", dependencies),
    ("restart kvuli zavislosti se nepocita do limitu", dep_restarts_not_counted),
    ("revive obnovi i neprimo zavisle sluzby", revive_transitive),
];

const SVC: &str = "svctest";

fn hold_idempotent(s: &Startup) -> TestResult {
    let mut h = Harness::new(s);
    h.add(SVC, &[], "normal")?;
    let c = h.client(SVC, "alice")?;
    check_eq!(h.ask(c, "call idem who")?, String::from("ok gen=1 who=alice"), "pred padem");
    h.send(c, "call idem hold")?;
    h.wait_held(SVC)?;
    h.kill(SVC)?;
    let (r, _) = h.reply(c)?;
    check_eq!(r, String::from("ok gen=2 who=alice"), "zopakovany pozadavek");
    check!(
        h.log.iter().any(|e| *e == Happened::Reconnected { name: SVC, generation: 2 }),
        "supervisor nevydal novou relaci"
    );
    check_eq!(h.ask(c, "gen")?, String::from("ok 2"), "generace u klienta");
    Ok(())
}

fn hold_effect(s: &Startup) -> TestResult {
    let mut h = Harness::new(s);
    h.add(SVC, &[], "normal")?;
    let c = h.client(SVC, "alice")?;
    h.send(c, "call effect bump")?;
    h.wait_held(SVC)?;
    h.kill(SVC)?;
    let (r, _) = h.reply(c)?;
    check_eq!(r, String::from("err SERVICE_RESTARTED"), "pozadavek s ucinkem");
    // Relace uz je obnovena: dalsi pozadavek jde na novou instanci.
    check_eq!(h.ask(c, "call idem who")?, String::from("ok gen=2 who=alice"), "po chybe");
    Ok(())
}

fn narrowed(s: &Startup) -> TestResult {
    let mut h = Harness::new(s);
    h.add(SVC, &[], "normal")?;
    let c = h.client(SVC, "alice")?;
    // Zuzeni, ktere by identitu rozsirilo, supervisor odmitne.
    check_eq!(h.ask(c, "narrow ..")?, String::from("err ACCESS_DENIED"), "zuzeni ..");
    check_eq!(h.ask(c, "narrow child")?, String::from("ok "), "zuzeni");
    check_eq!(h.ask(c, "call idem who")?, String::from("ok gen=1 who=alice/child"), "zuzena relace");
    h.kill_and_restart(SVC)?;
    check_eq!(
        h.ask(c, "call idem who")?,
        String::from("ok gen=2 who=alice/child"),
        "zuzena relace po padu"
    );
    Ok(())
}

fn gave_up(s: &Startup) -> TestResult {
    let mut h = Harness::new(s);
    // Kazda dalsi instance hned skonci: po 5 restartech to supervisor vzda.
    h.add(SVC, &[], "crash")?;
    let c = h.client(SVC, "alice")?;
    check_eq!(h.ask(c, "wait 60000")?, String::from("ok "), "lhuta klienta");
    h.kill(SVC)?;
    h.send(c, "call idem who")?;
    let (r, ms) = h.reply(c)?;
    check_eq!(r, String::from("err SERVICE_UNAVAILABLE"), "vzdana sluzba");
    // Prodlevy 100 + 200 + 400 + 800 + 1600 ms; lhuta klienta je 60 s.
    check!(ms < 15_000, &format!("klient cekal {ms} ms, ne do vzdani"));
    let st = h.sup.status_of(SVC).ok_or("sluzba neni v registru")?;
    check_eq!(st.state, State::GaveUp, "stav sluzby");
    check!(st.reason.is_some(), "chybi duvod vzdani");
    Ok(())
}

fn slow_restart(s: &Startup) -> TestResult {
    let mut h = Harness::new(s);
    // Nova instance potvrdi start az po 3 s.
    h.add(SVC, &[], "slow:3000")?;
    let c = h.client(SVC, "alice")?;
    check_eq!(h.ask(c, "wait 300")?, String::from("ok "), "lhuta klienta");
    h.kill(SVC)?;
    h.send(c, "call idem who")?;
    let (r, ms) = h.reply(c)?;
    check_eq!(r, String::from("err SERVICE_UNAVAILABLE"), "pomaly restart");
    check!(ms < 2_500, &format!("klient cekal {ms} ms misto 300"));
    // Dalsi volani s delsi lhutou se pripoji k nove instanci.
    check_eq!(h.ask(c, "wait 10000")?, String::from("ok "), "lhuta klienta");
    check_eq!(h.ask(c, "call idem who")?, String::from("ok gen=2 who=alice"), "po startu");
    Ok(())
}

fn deliberate_close(s: &Startup) -> TestResult {
    let mut h = Harness::new(s);
    h.add(SVC, &[], "normal")?;
    let c = h.client(SVC, "alice")?;
    check_eq!(h.ask(c, "call idem close-me")?, String::from("err PEER_CLOSED"), "umyslne zavreni");
    check_eq!(h.sup.generation(SVC), Some(1), "sluzba se nerestartovala");
    check!(
        !h.log.iter().any(|e| matches!(e, Happened::Reconnected { .. })),
        "supervisor vydal novou relaci po umyslnem zavreni"
    );
    // Listek zanikl: odebrani prav nejde obejit dalsim pokusem ani NARROW.
    check_eq!(h.sup.lease_count(), 0, "listky po umyslnem zavreni");
    check_eq!(h.ask(c, "call idem who")?, String::from("err SERVICE_UNAVAILABLE"), "dalsi pokus");
    check_eq!(h.ask(c, "narrow child")?, String::from("err SERVICE_UNAVAILABLE"), "zuzeni po odebrani");
    Ok(())
}

const DEP_RESTART: &[Dep] = &[Dep::restart(SVC)];
const DEP_NOTIFY: &[Dep] = &[Dep::notify(SVC)];

fn dependencies(s: &Startup) -> TestResult {
    let mut h = Harness::new(s);
    h.add(SVC, &[], "normal")?;
    h.add("svcdep", DEP_RESTART, "normal")?;
    let note = h.add("svcnote", DEP_NOTIFY, "normal")?;
    let c = h.client("svcdep", "bob")?;
    h.kill(SVC)?;
    let got = h.drive_until(|e| e.contains(&Happened::Restarted("svcdep")))?;
    let pos = |x: &Happened| got.iter().position(|e| e == x);
    let stopped = pos(&Happened::Stopped { name: "svcdep", because: SVC }).ok_or("svcdep se neukoncila")?;
    let restarted = pos(&Happened::Restarted(SVC)).ok_or("svctest se nerestartovala")?;
    let dep = pos(&Happened::Restarted("svcdep")).ok_or("svcdep se nespustila")?;
    check!(stopped < restarted && restarted < dep, &format!("spatne poradi: {got:?}"));
    check!(
        got.contains(&Happened::Notified { name: "svcnote", dep: SVC }),
        "svcnote nedostala oznameni"
    );
    check_eq!(note.lock().notified.clone(), alloc::vec![(String::from(SVC), 2)], "oznameni");
    check_eq!(h.sup.generation("svcnote"), Some(1), "svcnote bezi dal");
    check_eq!(h.sup.generation("svcdep"), Some(2), "svcdep nova generace");
    // Pad zavislosti se svcdep nepocita jako jeji pad.
    let st = h.sup.status_of("svcdep").ok_or("svcdep neni v registru")?;
    check_eq!(st.state, State::Running, "stav svcdep");
    // Klient svcdep se pripoji k nove instanci.
    check_eq!(h.ask(c, "call idem who")?, String::from("ok gen=2 who=bob"), "klient svcdep");
    Ok(())
}

fn dep_restarts_not_counted(s: &Startup) -> TestResult {
    let mut h = Harness::new(s);
    h.add(SVC, &[], "normal")?;
    h.add("svcdep", DEP_RESTART, "normal")?;
    // Zavislost spadne RESTART_MAX-krat; svcdep se pokazde restartuje s ni.
    for _ in 0..crate::supervisor::RESTART_MAX {
        h.kill(SVC)?;
        h.drive_until(|e| e.contains(&Happened::Restarted("svcdep")))?;
    }
    // Vlastni pad svcdep je teprve prvni: restart, ne vzdani.
    let got = h.kill_and_restart("svcdep")?;
    check!(!got.contains(&Happened::GaveUp("svcdep")), "svcdep vzdana po prvnim vlastnim padu");
    let st = h.sup.status_of("svcdep").ok_or("svcdep neni v registru")?;
    check_eq!(st.state, State::Running, "stav svcdep");
    Ok(())
}

const DEP_ON_DEP: &[Dep] = &[Dep::restart("svcdep")];

fn revive_transitive(s: &Startup) -> TestResult {
    let mut h = Harness::new(s);
    // Kazda dalsi instance svctest hned skonci: supervisor to vzda a s ni
    // i svcdep a svcdep2 (C -> B -> A).
    h.add(SVC, &[], "crash")?;
    h.add("svcdep", DEP_RESTART, "normal")?;
    h.add("svcdep2", DEP_ON_DEP, "normal")?;
    h.kill(SVC)?;
    h.drive_until(|e| e.contains(&Happened::GaveUp("svcdep2")))?;
    for name in [SVC, "svcdep", "svcdep2"] {
        let st = h.sup.status_of(name).ok_or("sluzba neni v registru")?;
        check_eq!(st.state, State::GaveUp, &format!("stav {name} po vzdani"));
    }
    // Dalsi instance svctest uz bezi normalne.
    h.shared(SVC)?.lock().mode_after = "normal";
    check!(h.sup.revive(SVC), "revive");
    h.drive_until(|e| e.contains(&Happened::Restarted("svcdep2")))?;
    for name in [SVC, "svcdep", "svcdep2"] {
        let st = h.sup.status_of(name).ok_or("sluzba neni v registru")?;
        check_eq!(st.state, State::Running, &format!("stav {name} po revive"));
    }
    Ok(())
}
