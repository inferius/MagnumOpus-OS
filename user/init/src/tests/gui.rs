//! GUI (milnik 6c) v systemu: ovladac virtio-gpu, kompozitor `compd`,
//! aplikace nad knihovnou `ui`, strom pristupnosti, umely a fyzicky vstup
//! (QMP), systemove dialogy, akce aplikaci, okno terminalu.
//!
//! Sada bezi posledni: kompozitor prevezme fokus vstupu od initu.
//!
//! Fyzicky vstup posila xtask pres QMP po znackach v logu
//! (`xtask/src/qmp.rs`, modul `gui`): Tab a Enter do dialogu, Ctrl+Alt+Del
//! a Enter na bezpecne obrazovce.

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use core::time::Duration;

use guiproto::a11y::{Node, Role, actions};
use guiproto::action::Value;
use guiproto::control::*;
use inputproto::code::*;
use inputproto::device::{self, RawEvent};
use libos::spawn::StartHandle;
use libos::{Channel, Startup, println, time};

use super::{Test, TestResult, devices, fail};
use crate::gui::{self, Gui};
use crate::{check, check_eq};

pub const TESTS: &[Test] = &[
    ("compd: displej, okno ukazky, strom pristupnosti", window_and_a11y),
    ("compd: umely vstup klika v aplikaci", injected_click),
    ("compd: akce aplikace: overeni argumentu a potvrzeni", app_actions),
    ("compd: systemovy dialog prijme jen fyzicky vstup (QMP)", dialog_physical_only),
    ("compd: bezpecna obrazovka po Ctrl+Alt+Del (QMP)", secure_screen),
    ("compd: okno terminalu s vlastnim titulkem", terminal_window),
    ("compd: spravce uloh a ikona uloh v panelu", task_manager),
    ("compd: skladani na GPU (Venus) dava stejne pixely jako CPU", super::gui_gpu::gpu_backend),
    // Posledni: kompozitor sady se ukonci.
    ("compd: pad kompozitoru uprostred prace, okno se vrati", super::gui_recovery::crash_mid_work),
];

/// Znacky pro xtask (musi sedet s `xtask/src/qmp.rs`, modul `gui`).
pub const QEMU_DIALOG: &str = "init: test gui: dialog ceka na klavesy z QEMU";
pub const QEMU_SECURE: &str = "init: test gui: cekam na zkratku z QEMU";
pub const QEMU_SECURE_ENTER: &str = "init: test gui: bezpecna obrazovka ceka na Enter";

const TIMEOUT: Duration = Duration::from_secs(20);

/// Barvy, ktere test ocekava na obrazovce (`libs/ui` a `libs/comp`).
pub(super) const UI_BG: u32 = 0xFF1E_232A;
pub(super) const BADGE: u32 = 0xFF1F_6F4E;

pub(super) struct State {
    pub(super) gui: Gui,
    pub(super) inspect: Channel,
    /// Okno ukazky (id kompozitoru).
    pub(super) demo: u32,
}

pub(super) static STATE: spin::Mutex<Option<State>> = spin::Mutex::new(None);

pub(super) fn deadline() -> Option<u64> {
    Some(time::deadline_after(TIMEOUT))
}

pub(super) fn ask(ch: &Channel, r: InspectRequest) -> Result<InspectReply, String> {
    ch.write(&r.encode(), Vec::new()).map_err(|e| fail("inspekce: zapis", e))?;
    let m = ch.read(deadline()).map_err(|e| fail("inspekce: odpoved", e))?;
    InspectReply::decode(&m.data).map_err(|e| fail("inspekce: zprava", e))
}

pub(super) fn settle(ch: &Channel) -> TestResult {
    match ask(ch, InspectRequest::Settle)? {
        InspectReply::Done => Ok(()),
        o => Err(alloc::format!("settle: {o:?}")),
    }
}

pub(super) fn windows(ch: &Channel) -> Result<Vec<WindowInfo>, String> {
    match ask(ch, InspectRequest::Windows)? {
        InspectReply::Windows(w) => Ok(w),
        o => Err(alloc::format!("windows: {o:?}")),
    }
}

pub(super) fn tree(ch: &Channel, window: u32) -> Result<Vec<Node>, String> {
    match ask(ch, InspectRequest::Tree { window })? {
        InspectReply::Tree(n) => Ok(n),
        o => Err(alloc::format!("tree: {o:?}")),
    }
}

pub(super) fn pixel(ch: &Channel, x: i32, y: i32) -> Result<u32, String> {
    match ask(ch, InspectRequest::Pixel { x: x as u32, y: y as u32 })? {
        InspectReply::Pixel(p) => Ok(p),
        o => Err(alloc::format!("pixel: {o:?}")),
    }
}

/// Opakuje dotaz na strom, dokud `pred` neplati (aplikace pracuje
/// asynchronne).
fn wait_tree(ch: &Channel, window: u32, what: &str, pred: impl Fn(&[Node]) -> bool) -> Result<Vec<Node>, String> {
    let end = time::deadline_after(TIMEOUT);
    loop {
        let t = tree(ch, window)?;
        if pred(&t) {
            return Ok(t);
        }
        if time::now() > end {
            return Err(alloc::format!("{what}: strom {:?}", t.iter().map(|n| (n.role.name(), n.name.clone(), n.value.clone())).collect::<Vec<_>>()));
        }
        time::sleep(Duration::from_millis(50));
    }
}

fn wait_window(ch: &Channel, app: &str) -> Result<WindowInfo, String> {
    let end = time::deadline_after(TIMEOUT);
    loop {
        if let Some(w) = windows(ch)?.into_iter().find(|w| w.app == app) {
            return Ok(w);
        }
        if time::now() > end {
            return Err(alloc::format!("okno aplikace {app} se neobjevilo"));
        }
        time::sleep(Duration::from_millis(50));
    }
}

pub(super) fn has(t: &[Node], role: Role, name: &str) -> bool {
    t.iter().any(|n| n.role == role && n.name == name)
}

pub(super) fn node<'a>(t: &'a [Node], role: Role, name: &str) -> Result<&'a Node, String> {
    t.iter().find(|n| n.role == role && n.name == name).ok_or_else(|| alloc::format!("uzel {} '{name}' chybi", role.name()))
}

pub(super) fn with<R>(f: impl FnOnce(&mut State) -> Result<R, String>) -> Result<R, String> {
    let mut g = STATE.lock();
    let s = g.as_mut().ok_or("kompozitor nebezi (selhal prvni test)")?;
    f(s)
}

fn window_and_a11y(startup: &Startup) -> TestResult {
    let mut gui = gui::start(startup).map_err(|e| fail("start kompozitoru", e))?;
    check!(gui.size.0 >= 640 && gui.size.1 >= 400, "maly displej");
    let input = devices().and_then(|d| d.input.as_ref()).ok_or("sluzba vstupu nebezi")?;
    gui.take_input(input).map_err(|e| fail("predani vstupu", e))?;
    let inspect = gui.channel(INSPECT.as_bytes()).map_err(|e| fail("inspect", e))?;
    gui.launch(startup, "gui-demo").map_err(|e| fail("spusteni gui-demo", e))?;
    let w = wait_window(&inspect, "org.magnumopus.demo")?;
    check!(w.focused, "nove okno ma fokus");
    let t = wait_tree(&inspect, w.id, "prvni snimek ukazky", |t| has(t, Role::Label, "Počítadlo: 0"))?;
    let b = node(&t, Role::Button, "Přičíst")?;
    check!(b.bounds.x >= w.rect.x && b.bounds.y >= w.rect.y, "uzel v souradnicich obrazovky");
    // Klik pres strom pristupnosti (jako ctecka obrazovky nebo agent).
    let r = ask(&inspect, InspectRequest::Act { window: w.id, node: b.id, action: actions::ACTIVATE, value: String::new() })?;
    check_eq!(r, InspectReply::Done, "akce pres uzel");
    wait_tree(&inspect, w.id, "pocitadlo po kliku", |t| has(t, Role::Label, "Počítadlo: 1"))?;
    // Obraz: pozadi okna aplikace a odznak duvery systemu nad titulkem.
    settle(&inspect)?;
    let bg = pixel(&inspect, w.rect.x + 3, w.rect.y + w.rect.h as i32 - 4)?;
    check_eq!(bg, UI_BG, "pozadi okna aplikace na obrazovce");
    let badge = pixel(&inspect, w.rect.x + 9, w.rect.y - 16)?;
    check_eq!(badge, BADGE, "odznak duvery v titulku");
    let st = gui.status().map_err(|e| fail("status", e))?;
    check!(st.clients == 1 && st.windows == 1 && st.presented > 0, "stav kompozitoru");
    println!("init: test gui: okno {}x{} na {},{}", w.rect.w, w.rect.h, w.rect.x, w.rect.y);
    *STATE.lock() = Some(State { gui, inspect, demo: w.id });
    Ok(())
}

fn injected_click(_: &Startup) -> TestResult {
    let input = devices().and_then(|d| d.input.as_ref()).ok_or("sluzba vstupu nebezi")?;
    with(|s| {
        let t = tree(&s.inspect, s.demo)?;
        let b = node(&t, Role::Button, "Přičíst")?.bounds;
        let before = t.iter().find(|n| n.name.starts_with("Počítadlo")).map(|n| n.name.clone()).unwrap_or_default();
        let (sw, sh) = s.gui.size;
        let inj = input.inject().map_err(|e| fail("inject", e))?;
        inj.write(&device::encode_hello(device::caps::POINTER, "test gui"), Vec::new()).map_err(|e| fail("predstaveni", e))?;
        // Ukazatel nejdriv do leveho horniho rohu (pohyb se orizne), pak na
        // stred tlacitka.
        let (tx, ty) = (b.x + b.w as i32 / 2, b.y + b.h as i32 / 2);
        let frames = [
            vec![RawEvent::rel(REL_X, -(sw as i32) * 2), RawEvent::rel(REL_Y, -(sh as i32) * 2), RawEvent::syn()],
            vec![RawEvent::rel(REL_X, tx), RawEvent::rel(REL_Y, ty), RawEvent::syn()],
            vec![RawEvent::key(BTN_LEFT, true), RawEvent::syn()],
            vec![RawEvent::key(BTN_LEFT, false), RawEvent::syn()],
        ];
        for f in frames {
            inj.write(&device::encode_events(&f), Vec::new()).map_err(|e| fail("umely vstup", e))?;
        }
        let want = alloc::format!("Počítadlo: {}", before.trim_start_matches("Počítadlo: ").parse::<i64>().unwrap_or(0) + 1);
        wait_tree(&s.inspect, s.demo, "klik umelym vstupem", |t| has(t, Role::Label, &want))?;
        // Panel ukazuje, ze vstup ovlada automat.
        check!(s.gui.status().is_ok(), "status");
        drop(inj);
        Ok(())
    })
}

fn app_actions(_: &Startup) -> TestResult {
    with(|s| {
        let ch = s.gui.channel(b"actions shell").map_err(|e| fail("kanal akci", e))?;
        let call = |m: CallerMsg| -> Result<CallerReply, String> {
            ch.write(&m.encode(), Vec::new()).map_err(|e| fail("volani", e))?;
            let r = ch.read(deadline()).map_err(|e| fail("odpoved akce", e))?;
            CallerReply::decode(&r.data).map_err(|e| fail("zprava akce", e))
        };
        match call(CallerMsg::List)? {
            CallerReply::Actions { list } => {
                check!(list.iter().any(|(app, a)| app == "org.magnumopus.demo" && a.id == "note.add"), "akce z manifestu");
            }
            o => return Err(alloc::format!("seznam akci: {o:?}")),
        }
        // Spatny typ argumentu aplikace nedostane.
        let bad = call(CallerMsg::Call {
            call: 1,
            app: "org.magnumopus.demo".into(),
            action: "counter.add".into(),
            args: vec![("by".into(), Value::Text("pet".into()))],
            synthetic: false,
        })?;
        check!(matches!(bad, CallerReply::Result { call: 1, ok: false, .. }), "spatny argument odmitnut");
        let ok = call(CallerMsg::Call {
            call: 2,
            app: "org.magnumopus.demo".into(),
            action: "note.add".into(),
            args: vec![("text".into(), Value::Text("Mléko".into())), ("important".into(), Value::Bool(true))],
            synthetic: false,
        })?;
        check!(matches!(ok, CallerReply::Result { call: 2, ok: true, value: Some(Value::Int(1)), .. }), "akce note.add");
        wait_tree(&s.inspect, s.demo, "poznamka z akce", |t| has(t, Role::ListItem, "! Mléko"))?;
        // Agent (umely zdroj): akce ceka na potvrzeni systemovym dialogem.
        ch.write(
            &CallerMsg::Call { call: 3, app: "org.magnumopus.demo".into(), action: "counter.add".into(), args: vec![("by".into(), Value::Int(5))], synthetic: true }
                .encode(),
            Vec::new(),
        )
        .map_err(|e| fail("volani agenta", e))?;
        let end = time::deadline_after(TIMEOUT);
        while s.gui.status().map_err(|e| fail("status", e))?.dialogs == 0 {
            check!(time::now() < end, "dialog potvrzeni se neukazal");
            time::sleep(Duration::from_millis(20));
        }
        // Bez fyzickeho vstupu nic: zadna odpoved.
        check!(ch.read(Some(time::deadline_after(Duration::from_millis(300)))).is_err(), "akce agenta probehla bez potvrzeni");
        // Zavreni kanalu volajiciho dialog zrusi (volani zmizi).
        drop(ch);
        Ok(())
    })
}

fn dialog_physical_only(_: &Startup) -> TestResult {
    let input = devices().and_then(|d| d.input.as_ref()).ok_or("sluzba vstupu nebezi")?;
    with(|s| {
        let d = s.gui.channel(DIALOG.as_bytes()).map_err(|e| fail("kanal dialogu", e))?;
        d.write(&DialogRequest::Phrase { text: "modrý slon".into() }.encode(), Vec::new()).map_err(|e| fail("fraze", e))?;
        let ask = DialogRequest::Ask {
            id: 7,
            kind: DialogKind::Permission,
            app: "Ukázka (org.magnumopus.demo)".into(),
            text: "Povolit aplikaci přístup k síti?".into(),
            options: vec!["Povolit".into(), "Zamítnout".into()],
        };
        d.write(&ask.encode(), Vec::new()).map_err(|e| fail("dialog", e))?;
        // Umely Enter (automat, agent) dialog nepotvrdi.
        let inj = input.inject().map_err(|e| fail("inject", e))?;
        inj.write(&device::encode_hello(device::caps::KEYBOARD, "test gui"), Vec::new()).map_err(|e| fail("predstaveni", e))?;
        let enter = [RawEvent::key(KEY_ENTER, true), RawEvent::syn(), RawEvent::key(KEY_ENTER, false), RawEvent::syn()];
        inj.write(&device::encode_events(&enter), Vec::new()).map_err(|e| fail("umely Enter", e))?;
        check!(d.read(Some(time::deadline_after(Duration::from_millis(500)))).is_err(), "dialog prijal umely vstup");
        drop(inj);
        // Fyzicka klavesnice (QMP): Tab + Enter = druha volba.
        println!("{QEMU_DIALOG}");
        let m = d.read(deadline()).map_err(|e| fail("odpoved dialogu (QMP)", e))?;
        let r = DialogReply::decode(&m.data).map_err(|e| fail("zprava dialogu", e))?;
        check_eq!(r, DialogReply::Answer { id: 7, choice: 1, text: String::new() }, "odpoved dialogu");
        Ok(())
    })
}

fn secure_screen(_: &Startup) -> TestResult {
    with(|s| {
        println!("{QEMU_SECURE}");
        let end = time::deadline_after(TIMEOUT);
        while !s.gui.status().map_err(|e| fail("status", e))?.secure {
            check!(time::now() < end, "bezpecna obrazovka se neukazala");
            time::sleep(Duration::from_millis(20));
        }
        // Obrazovka je ztlumena a nad ni systemovy dialog: aplikace pod ni
        // uz nema svou barvu pozadi.
        settle(&s.inspect)?;
        let w = windows(&s.inspect)?.into_iter().find(|w| w.id == s.demo).ok_or("okno ukazky zmizelo")?;
        check!(pixel(&s.inspect, w.rect.x + 3, w.rect.y + w.rect.h as i32 - 4)? != UI_BG, "pod bezpecnou obrazovkou je ztlumeno");
        // Dialog prijima vstup az chvili po zobrazeni.
        time::sleep(Duration::from_millis(400));
        println!("{QEMU_SECURE_ENTER}");
        while s.gui.status().map_err(|e| fail("status", e))?.secure {
            check!(time::now() < end, "Enter bezpecnou obrazovku nezavrel");
            time::sleep(Duration::from_millis(20));
        }
        Ok(())
    })
}

fn terminal_window(startup: &Startup) -> TestResult {
    with(|s| {
        // Jako v beznem behu: kanal pripojeni, termd spusti init
        // (`terminal.rs`); smycku supervisoru tu nahrazuje `poll`.
        let conn = crate::terminal::TERMINALS.lock().client("gui-term (test)").map_err(|e| fail("kanal terminalu", e))?;
        s.gui
            .launch_extra(startup, "gui-term", vec![StartHandle::user(conn.into_handle())])
            .map_err(|e| fail("spusteni gui-term", e))?;
        let pump = || crate::terminal::TERMINALS.lock().poll(startup);
        let w = wait_window(&s.inspect, "org.magnumopus.terminal")?;
        let t = wait_tree(&s.inspect, w.id, "uvitani terminalu", |t| {
            pump();
            t.iter().any(|n| n.role == Role::Terminal && n.value.contains("Napište help"))
        })?;
        check!(has(&t, Role::Titlebar, "Terminál"), "vlastni titulek je v pristupnosti");
        let term = t.iter().find(|n| n.role == Role::Terminal).unwrap();
        // Radek pres pristupnost (SET_VALUE = napsat a odeslat) jde do shellu.
        ask(&s.inspect, InspectRequest::Act { window: w.id, node: term.id, action: actions::SET_VALUE, value: "echo ahoj z GUI".into() })?;
        wait_tree(&s.inspect, w.id, "vystup prikazu", |t| {
            pump();
            t.iter().any(|n| n.role == Role::Terminal && n.value.lines().any(|l| l.trim() == "ahoj z GUI"))
        })?;
        // Odznak duvery kresli system i nad vlastnim titulkem aplikace.
        settle(&s.inspect)?;
        check_eq!(pixel(&s.inspect, w.rect.x + 9, w.rect.y + 16)?, BADGE, "odznak nad vlastnim titulkem");
        Ok(())
    })
}

fn task_manager(startup: &Startup) -> TestResult {
    with(|s| {
        // Ikona "bezi N uloh" v panelu (do pripojeni k taskd ji nastavuje init).
        s.gui.command(b"tasks 4 1", Vec::new()).map_err(|e| fail("tasks", e))?;
        check!(s.gui.command(b"tasks x", Vec::new()).is_err(), "neplatny prikaz tasks");
        s.gui.launch(startup, "gui-tasks").map_err(|e| fail("spusteni gui-tasks", e))?;
        let w = wait_window(&s.inspect, "org.magnumopus.tasks")?;
        let t = wait_tree(&s.inspect, w.id, "seznam uloh", |t| t.iter().any(|n| n.role == Role::ListItem))?;
        // Uloha agenta je vyrazne oznacena a nahore.
        let first = t.iter().find(|n| n.role == Role::ListItem).unwrap();
        check!(first.name.starts_with("[AGENT AI]"), "uloha agenta nahore a oznacena");
        check!(t.iter().any(|n| n.role == Role::Label && n.name.starts_with("Agent AI:")), "stitek agenta v zahlavi");
        // Systemova uloha bez opravneni: Zrusit neaktivni.
        let scrub = t.iter().find(|n| n.role == Role::ListItem && n.name.contains("Scrub")).ok_or("uloha Scrub chybi")?;
        ask(&s.inspect, InspectRequest::Act { window: w.id, node: scrub.id, action: actions::ACTIVATE, value: String::new() })?;
        let t = wait_tree(&s.inspect, w.id, "detail systemove ulohy", |t| t.iter().any(|n| n.role == Role::Label && n.name.contains("chybí oprávnění")))?;
        let cancel = node(&t, Role::Button, "Zrušit")?;
        check!(cancel.state & guiproto::a11y::state::DISABLED != 0, "Zrusit bez prava neaktivni");
        // Uloha agenta: pozastavit pres pristupnost.
        let agent = t.iter().find(|n| n.role == Role::ListItem && n.name.starts_with("[AGENT AI]")).unwrap();
        ask(&s.inspect, InspectRequest::Act { window: w.id, node: agent.id, action: actions::ACTIVATE, value: String::new() })?;
        let t = wait_tree(&s.inspect, w.id, "detail ulohy agenta", |t| has(t, Role::Label, "Spustil agent AI (asistent)"))?;
        let pause = node(&t, Role::Button, "Pozastavit")?;
        ask(&s.inspect, InspectRequest::Act { window: w.id, node: pause.id, action: actions::ACTIVATE, value: String::new() })?;
        wait_tree(&s.inspect, w.id, "pozastaveni ulohy agenta", |t| t.iter().any(|n| n.role == Role::Label && n.name == "Stav: pozastaveno 15 %"))?;
        Ok(())
    })
}
