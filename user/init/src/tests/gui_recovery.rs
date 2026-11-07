//! Test "zabij kompozitor uprostred prace" (sada `gui`, posledni test;
//! kostra `recovery_kit.rs`, protokol `docs/cs/obnova-sluzeb.md`).
//!
//! Kompozitor sady se ukonci a jeho displej (drzak koncu) prevezme
//! kompozitor pod supervisorem testu. Aplikace `gui-demo` (jiny proces)
//! dostane relaci a listek od supervisoru. Kompozitor zabijeme hned po
//! akci v okne; supervisor ho spusti znovu nad stejnym koncem kanalu
//! k ovladaci displeje, aplikace se pripoji pres listek a posle znovu
//! stav okna. Okno se vrati se stejnou identitou (odznak duvery,
//! id aplikace), aplikace dal reaguje a nic nevisi.

use alloc::boxed::Box;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use guiproto::a11y::{Node, Role, actions};
use guiproto::control::{INSPECT, InspectReply, InspectRequest, WindowInfo};
use libos::{Channel, Signals, Startup, println, time};

use super::gui::{BADGE, STATE, UI_BG, ask, node, pixel, settle, tree, windows};
use super::recovery_kit::{Harness, TIMEOUT};
use super::{TestResult, fail};
use crate::gui::{self, CompdService, Gui};
use crate::supervisor::Happened;
use crate::{check, check_eq};

const DEMO: &str = "org.magnumopus.demo";

/// Ridi supervisor testu (obnova pres listky), dokud `f` nevrati `Some`.
fn drive<T>(h: &mut Harness, what: &str, mut f: impl FnMut() -> Result<Option<T>, String>) -> Result<T, String> {
    let end = time::deadline_after(TIMEOUT);
    loop {
        if let Some(v) = f()? {
            return Ok(v);
        }
        if time::now() > end {
            return Err(format!("{what}: do lhuty nic (supervisor: {:?})", h.log));
        }
        h.step(30);
    }
}

fn window(h: &mut Harness, inspect: &Channel) -> Result<WindowInfo, String> {
    drive(h, "okno ukazky", || Ok(windows(inspect)?.into_iter().find(|w| w.app == DEMO)))
}

/// Pocitadlo ukazky ve strome pristupnosti.
fn counter(t: &[Node]) -> Option<i64> {
    t.iter()
        .find(|n| n.role == Role::Label && n.name.starts_with("Počítadlo: "))
        .and_then(|n| n.name.trim_start_matches("Počítadlo: ").parse().ok())
}

fn wait_counter(h: &mut Harness, inspect: &Channel, win: u32, what: &str, ok: impl Fn(i64) -> bool) -> Result<(Vec<Node>, i64), String> {
    drive(h, what, || {
        let t = tree(inspect, win)?;
        Ok(counter(&t).filter(|&c| ok(c)).map(|c| (t, c)))
    })
}

fn click(inspect: &Channel, win: u32, t: &[Node]) -> TestResult {
    let b = node(t, Role::Button, "Přičíst")?;
    let r = ask(inspect, InspectRequest::Act { window: win, node: b.id, action: actions::ACTIVATE, value: String::new() })?;
    check_eq!(r, InspectReply::Done, "akce pres uzel");
    Ok(())
}

pub fn crash_mid_work(startup: &Startup) -> TestResult {
    // Kompozitor sady (bez supervisoru) ukoncit; displej zustane v drzaku.
    let (shared, size) = {
        let mut g = STATE.lock();
        let s = g.take().ok_or("kompozitor nebezi (selhal prvni test)")?;
        let shared = s.gui.shared.clone();
        if let Some(p) = shared.lock().process.as_ref() {
            let _ = p.kill();
            let _ = libos::wait_one(p.handle(), Signals::TERMINATED, Some(time::deadline_after(TIMEOUT)));
        }
        for a in &s.gui.apps {
            let _ = a.process.kill();
        }
        (shared, s.gui.size)
    };
    let mut h = Harness::new(startup);
    h.add_service(gui::NAME, gui::DEPS, Box::new(CompdService::new(shared.clone(), true)))?;
    let mut g = Gui { shared, apps: Vec::new(), size };
    // Aplikace v jinem procesu s relaci a listkem od supervisoru testu.
    let info = gui::recipe(startup, "gui-demo").map_err(|e| fail("identita gui-demo", e))?;
    let (ch, lease) = h.sup.session(gui::NAME, info.encode()).map_err(|e| fail("relace okna", e))?;
    g.launch_with(startup, "gui-demo", ch, lease, Vec::new()).map_err(|e| fail("spusteni gui-demo", e))?;
    let r = run(&mut h, &g);
    for a in &g.apps {
        let _ = a.process.kill();
    }
    if let Some(p) = h.sup.process(gui::NAME) {
        let _ = libos::sys::process_kill(p.raw());
    }
    r
}

fn run(h: &mut Harness, g: &Gui) -> TestResult {
    let inspect = g.channel(INSPECT.as_bytes()).map_err(|e| fail("inspect", e))?;
    let w = window(h, &inspect)?;
    let (t, _) = wait_counter(h, &inspect, w.id, "prvni snimek", |c| c == 0)?;
    click(&inspect, w.id, &t)?;
    let (t, _) = wait_counter(h, &inspect, w.id, "pocitadlo po kliku", |c| c == 1)?;
    // Uprostred prace: akce a hned pad (aplikace zrovna kresli novy snimek).
    click(&inspect, w.id, &t)?;
    let gen_app = g.apps[0].process.handle().duplicate(None).map_err(|e| fail("handle aplikace", e))?;
    h.kill(gui::NAME)?;
    h.drive_until(|e| e.contains(&Happened::Restarted(gui::NAME)))?;
    // Nova instance: aplikace se pripoji sama pres listek.
    let inspect = g.channel(INSPECT.as_bytes()).map_err(|e| fail("inspect po padu", e))?;
    let w2 = window(h, &inspect)?;
    check!(
        h.log.iter().any(|e| *e == Happened::Reconnected { name: gui::NAME, generation: 2 }),
        "aplikace nedostala novou relaci pres listek"
    );
    // Stav aplikace zustal (1, nebo 2, kdyz akce stihla probehnout).
    let (t, c) = wait_counter(h, &inspect, w2.id, "obnoveny strom", |c| c == 1 || c == 2)?;
    check_eq!(w2.app, String::from(DEMO), "stejna identita");
    // Odznak duvery a obsah okna na obrazovce.
    settle(&inspect)?;
    check_eq!(pixel(&inspect, w2.rect.x + 9, w2.rect.y - 16)?, BADGE, "odznak duvery po obnove");
    check_eq!(pixel(&inspect, w2.rect.x + 3, w2.rect.y + w2.rect.h as i32 - 4)?, UI_BG, "obsah okna po obnove");
    // Jedno okno, jeden klient: nic navic, nic nevisi.
    let st = g.status().map_err(|e| fail("status", e))?;
    check!(st.clients == 1 && st.windows == 1, "stav po obnove: {st:?}");
    // Aplikace dal reaguje.
    click(&inspect, w2.id, &t)?;
    wait_counter(h, &inspect, w2.id, "klik po obnove", |n| n == c + 1)?;
    check!(
        libos::wait_one(&gen_app, Signals::TERMINATED, Some(0)).is_err(),
        "aplikace po padu kompozitoru skoncila"
    );
    println!("init: test gui: okno obnoveno po padu kompozitoru (pocitadlo {c})");
    Ok(())
}
