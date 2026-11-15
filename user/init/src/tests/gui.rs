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
    ("compd: volba vzhledu: plynuly prechod barev, omezit pohyb", super::gui_look::fade),
    ("init: volba vzhledu se ulozi jen pro system", super::gui_look::store),
    ("compd: umely vstup klika v aplikaci", injected_click),
    ("compd: akce aplikace: overeni argumentu a potvrzeni", app_actions),
    ("compd: systemovy dialog prijme jen fyzicky vstup (QMP)", dialog_physical_only),
    ("compd: bezpecna obrazovka po Ctrl+Alt+Del (QMP)", secure_screen),
    ("compd: okno terminalu s vlastnim titulkem", terminal_window),
    ("compd: terminal: tah mysi, Ctrl+C a Ctrl+V pres schranku, nabidka doplnovani (QMP)", super::gui_term::select_copy_paste),
    ("compd: spravce uloh a ikona uloh v panelu", task_manager),
    ("compd: diagnosticky snimek po Print Screen (QMP)", super::gui_snapshot::diag_snapshot),
    ("compd: paleta prikazu z klavesnice: okno a akce aplikace (QMP)", super::gui_palette::palette_keys),
    ("compd: vzhled podle navrhu, snimky obrazovky (QMP)", look_shots),
    ("files-copy: uloha s konfliktem prezije restart procesu", super::gui_files::copier_resume),
    ("compd: spravce souboru: kopie jako uloha, konflikt, kos, 10 000 polozek, snimek (QMP)", super::gui_files::files_window),
    ("compd: video NV12 a I420 s ploty ziskani a uvolneni", super::gui_video::nv12_with_fences),
    ("compd: 100 soucasnych klientu (smycka pres port)", super::gui_clients::many_clients),
    ("prehravac: plocha videa z mediad, prehrat a pozastavit", super::gui_player::play_pause),
    ("compd: skladani na GPU (Venus) dava stejne pixely jako CPU", super::gui_gpu::gpu_backend),
    ("role Gui: dite s MO_GUI okno otevre, bez ni ne", super::gui_role::inherit_and_opt_out),
    ("role Gui: odvolani identity zavre okna i potomkum", super::gui_role::revoke_reaches_children),
    // Posledni: kompozitor sady se ukonci.
    ("compd: pad kompozitoru uprostred prace, okno se vrati", super::gui_recovery::crash_mid_work),
];

/// Znacky pro xtask (musi sedet s `xtask/src/qmp.rs`, modul `gui`).
pub const QEMU_DIALOG: &str = "init: test gui: dialog ceka na klavesy z QEMU";
pub const QEMU_SECURE: &str = "init: test gui: cekam na zkratku z QEMU";
pub const QEMU_SECURE_ENTER: &str = "init: test gui: bezpecna obrazovka ceka na Enter";
/// Fyzicky klik (QMP): za znackou poloha na obrazovce "x y". xtask
/// prisune ukazatel do rohu a pak posle pohyb a stisk v jednom snimku
/// (jako tablet VMware), pusteni zvlast.
pub const QEMU_CLICK: &str = "init: test gui: klik mysi na ";
/// Snimek obrazovky: za znackou jmeno (`light`, `dark`, `contrast`);
/// xtask porizuje `screendump` do `target/shots/`.
pub const QEMU_SHOT: &str = "init: test gui: snimek ";
/// Poradi snimku a rezim motivu (musi sedet s `xtask/src/qmp.rs`).
pub const SHOTS: &[&str] = &["light", "dark", "contrast"];

const TIMEOUT: Duration = Duration::from_secs(20);

/// Barvy, ktere test ocekava na obrazovce (`libs/ui` a `libs/comp`).
/// Pozadi okna aplikace a odznak duvery (tokeny, svetly motiv).
pub(super) fn ui_bg() -> u32 {
    theme::Theme::builtin(theme::Mode::Light).color.surface
}

pub(super) fn badge() -> u32 {
    theme::Theme::builtin(theme::Mode::Light).color.trust_bg
}

/// Odznak okna bez fokusu: nizsi sytost (jako `saturate(.6)` v prototypu,
/// stejny vzorec jako kompozitor).
pub(super) fn badge_inactive() -> u32 {
    let c = badge();
    let (r, g, b) = (c >> 16 & 0xFF, c >> 8 & 0xFF, c & 0xFF);
    let l = (r * 54 + g * 183 + b * 19) >> 8;
    let m = |v: u32| (l * 102 + v * 153) / 255;
    (c & 0xFF00_0000) | m(r) << 16 | m(g) << 8 | m(b)
}

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
    // Opozdena odpoved na drivejsi dotaz (ten skoncil TIMED_OUT) by se
    // sparovala s timto dotazem a selhal by i dalsi test: zahodit ji.
    while let Ok(stale) = ch.try_read() {
        println!("init: test gui: zahozena opozdena odpoved inspekce ({} B)", stale.data.len());
    }
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
pub(super) fn wait_tree(ch: &Channel, window: u32, what: &str, pred: impl Fn(&[Node]) -> bool) -> Result<Vec<Node>, String> {
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

pub(super) fn wait_window(ch: &Channel, app: &str) -> Result<WindowInfo, String> {
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
    gui::connect_snapshots(&gui);
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
    let bg = pixel(&inspect, w.rect.x + w.rect.w as i32 / 2, w.rect.y + w.rect.h as i32 - 4)?;
    check_eq!(bg, ui_bg(), "pozadi okna aplikace na obrazovce");
    let px = pixel(&inspect, w.rect.x + 9, w.rect.y - 16)?;
    check_eq!(px, badge(), "odznak duvery v titulku");
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
        // Dialog prijima vstup az INPUT_DELAY po zobrazeni snimku s nim
        // (kompozitor na CPU kresli snimek i pres 1 s): klavesy z QEMU az
        // po ustaleni obrazovky a ochranne dobe, jinak je dialog zahodi.
        settle(&s.inspect)?;
        time::sleep(Duration::from_millis(400));
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
        check!(pixel(&s.inspect, w.rect.x + w.rect.w as i32 / 2, w.rect.y + w.rect.h as i32 - 4)? != ui_bg(), "pod bezpecnou obrazovkou je ztlumeno");
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
        let app = s
            .gui
            .launch_extra(startup, "gui-term", vec![StartHandle::user(conn.into_handle())])
            .map_err(|e| fail("spusteni gui-term", e))?;
        let pump = || crate::terminal::TERMINALS.lock().poll(startup);
        let w = wait_window(&s.inspect, "org.magnumopus.terminal")?;
        let t = wait_tree(&s.inspect, w.id, "uvitani terminalu", |t| {
            pump();
            t.iter().any(|n| n.role == Role::Terminal && n.value.contains("Napište help"))
        })?;
        check!(has(&t, Role::Titlebar, "Terminál"), "vlastni titulek je v pristupnosti");
        // Prepinac zapisu B / C v titulku: B zvoleny, ACTIVATE na C prepne.
        let checked = |t: &[guiproto::a11y::Node], name: &str| {
            t.iter().any(|n| n.role == Role::Button && n.name.starts_with(name) && n.state & guiproto::a11y::state::CHECKED != 0)
        };
        check!(checked(&t, "Zápis B"), "zapis B zvoleny");
        let zc = node(&t, Role::Button, "Zápis C (výrazový)")?;
        ask(&s.inspect, InspectRequest::Act { window: w.id, node: zc.id, action: actions::ACTIVATE, value: String::new() })?;
        let t = wait_tree(&s.inspect, w.id, "prepnuti na zapis C", |t| checked(t, "Zápis C"))?;
        let zb = node(&t, Role::Button, "Zápis B (příkazový)")?;
        ask(&s.inspect, InspectRequest::Act { window: w.id, node: zb.id, action: actions::ACTIVATE, value: String::new() })?;
        let term = t.iter().find(|n| n.role == Role::Terminal).unwrap();
        // Radek pres pristupnost (SET_VALUE = napsat a odeslat) jde do shellu.
        ask(&s.inspect, InspectRequest::Act { window: w.id, node: term.id, action: actions::SET_VALUE, value: "echo ahoj z GUI".into() })?;
        wait_tree(&s.inspect, w.id, "vystup prikazu", |t| {
            pump();
            t.iter().any(|n| n.role == Role::Terminal && n.value.lines().any(|l| l.trim() == "ahoj z GUI"))
        })?;
        // `help`: nadpis skupiny i kazdy prikaz na vlastnim radku (text
        // s konci radku se nesmi spojit do odstavce). Vidi se konec vypisu.
        ask(&s.inspect, InspectRequest::Act { window: w.id, node: term.id, action: actions::SET_VALUE, value: "help".into() })?;
        wait_tree(&s.inspect, w.id, "help po radcich", |t| {
            pump();
            t.iter().any(|n| {
                n.role == Role::Terminal && {
                    let l: Vec<&str> = n.value.lines().map(str::trim).collect();
                    l.contains(&"Systém")
                        && l.iter().any(|x| x.starts_with("shutdown "))
                        && l.iter().any(|x| x.starts_with("reboot "))
                }
            })
        })?;
        // Relace hlasi ulohy do prehledu uloh (kanaly razi init pres
        // registr): `tasks` vidi bezici ulohu `async`.
        ask(&s.inspect, InspectRequest::Act { window: w.id, node: term.id, action: actions::SET_VALUE, value: "let ulohy = async { sleep 30s }".into() })?;
        ask(&s.inspect, InspectRequest::Act { window: w.id, node: term.id, action: actions::SET_VALUE, value: "tasks | where state == \"running\" | count".into() })?;
        wait_tree(&s.inspect, w.id, "tasks v terminalu", |t| {
            pump();
            t.iter().any(|n| n.role == Role::Terminal && n.value.lines().any(|l| l.trim() == "1"))
        })?;
        // Karty relaci: nova relace (+), pak zpet na prvni; ta prehraje sve
        // bloky (vysledek posledniho bloku, `count` = 1, je zase videt).
        let t = wait_tree(&s.inspect, w.id, "karta relace", |t| {
            pump();
            t.iter().any(|n| n.role == Role::Button && n.name == "Nová relace")
        })?;
        let plus = node(&t, Role::Button, "Nová relace")?;
        ask(&s.inspect, InspectRequest::Act { window: w.id, node: plus.id, action: actions::ACTIVATE, value: String::new() })?;
        let t = wait_tree(&s.inspect, w.id, "nova relace v karte", |t| {
            pump();
            t.iter().any(|n| n.role == Role::Button && n.name == "Relace 1, odpojená")
        })?;
        let first = node(&t, Role::Button, "Relace 1, odpojená")?;
        ask(&s.inspect, InspectRequest::Act { window: w.id, node: first.id, action: actions::ACTIVATE, value: String::new() })?;
        wait_tree(&s.inspect, w.id, "navrat na relaci 1", |t| {
            pump();
            t.iter().any(|n| n.role == Role::Terminal && n.value.lines().any(|l| l.trim() == "1"))
                && t.iter().any(|n| n.role == Role::Button && n.name == "Relace 2, odpojená")
        })?;
        // Akce bloku (pristupnost = stejna cesta jako klik): Spustit znovu
        // prida druhy blok s prikazem, Sbalit ho sbali.
        let cmd = "tasks | where state == \"running\" | count";
        let t = wait_tree(&s.inspect, w.id, "akce bloku", |t| {
            pump();
            t.iter().any(|n| n.role == Role::Button && n.name == alloc::format!("Spustit znovu {cmd}"))
        })?;
        let again = node(&t, Role::Button, &alloc::format!("Spustit znovu {cmd}"))?;
        ask(&s.inspect, InspectRequest::Act { window: w.id, node: again.id, action: actions::ACTIVATE, value: String::new() })?;
        let t = wait_tree(&s.inspect, w.id, "blok spusteny znovu", |t| {
            pump();
            t.iter().any(|n| n.role == Role::Terminal && n.value.matches("tasks | where").count() >= 2)
        })?;
        let fold = t.iter().rev().find(|n| n.role == Role::Button && n.name == alloc::format!("Sbalit blok {cmd}")).ok_or("akce Sbalit chybi")?;
        ask(&s.inspect, InspectRequest::Act { window: w.id, node: fold.id, action: actions::ACTIVATE, value: String::new() })?;
        wait_tree(&s.inspect, w.id, "sbaleny blok", |t| {
            pump();
            t.iter().any(|n| n.role == Role::Button && n.name == alloc::format!("Rozbalit blok {cmd}"))
        })?;
        // Odznak duvery kresli system i nad vlastnim titulkem aplikace.
        settle(&s.inspect)?;
        let px = pixel(&s.inspect, w.rect.x + 9, w.rect.y + 16)?;
        check!(px == badge() || px == badge_inactive(), alloc::format!("odznak nad vlastnim titulkem: {px:#x}"));
        close_terminal(startup, s, app, "gui-term (test)")
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
        check!(first.name.starts_with("agent: "), "uloha agenta nahore a oznacena");
        check!(t.iter().any(|n| n.role == Role::Label && n.name.starts_with("agent: Agent AI:")), "stitek agenta v zahlavi");
        // Systemova uloha bez opravneni: Zrusit neaktivni.
        let scrub = t.iter().find(|n| n.role == Role::ListItem && n.name.contains("Scrub")).ok_or("uloha Scrub chybi")?;
        ask(&s.inspect, InspectRequest::Act { window: w.id, node: scrub.id, action: actions::ACTIVATE, value: String::new() })?;
        let t = wait_tree(&s.inspect, w.id, "detail systemove ulohy", |t| t.iter().any(|n| n.role == Role::Label && n.name.contains("chybí oprávnění")))?;
        let cancel = node(&t, Role::Button, "Zrušit")?;
        check!(cancel.state & guiproto::a11y::state::DISABLED != 0, "Zrusit bez prava neaktivni");
        // Uloha agenta: pozastavit pres pristupnost.
        let agent = t.iter().find(|n| n.role == Role::ListItem && n.name.starts_with("agent: ")).unwrap();
        ask(&s.inspect, InspectRequest::Act { window: w.id, node: agent.id, action: actions::ACTIVATE, value: String::new() })?;
        let t = wait_tree(&s.inspect, w.id, "detail ulohy agenta", |t| has(t, Role::Label, "agent: Spustil agent AI (asistent)"))?;
        let pause = node(&t, Role::Button, "Pozastavit")?;
        ask(&s.inspect, InspectRequest::Act { window: w.id, node: pause.id, action: actions::ACTIVATE, value: String::new() })?;
        wait_tree(&s.inspect, w.id, "pozastaveni ulohy agenta", |t| t.iter().any(|n| n.role == Role::Label && n.name == "Stav: pozastaveno 15 %"))?;
        // Fyzicky klik mysi na filtr "Jen úlohy agenta": seznam ukaze jen
        // ulohy agenta (klik dojde tam, kde je ukazatel, i kdyz pohyb
        // a stisk prijdou v jednom hlaseni zarizeni).
        let only = node(&t, Role::Checkbox, "Jen úlohy agenta")?;
        // Meze z inspekce jsou uz v souradnicich obrazovky.
        let (x, y) = (only.bounds.x + 8, only.bounds.y + only.bounds.h as i32 / 2);
        println!("{QEMU_CLICK}{x} {y}");
        let t = wait_tree(&s.inspect, w.id, "filtr po kliku mysi", |t| {
            let rows: Vec<&Node> = t.iter().filter(|n| n.role == Role::ListItem).collect();
            !rows.is_empty() && rows.iter().all(|n| n.name.starts_with("agent: "))
        })?;
        // Zpet vsechny ulohy (snimky vzhledu).
        let only = node(&t, Role::Checkbox, "Jen úlohy agenta")?;
        ask(&s.inspect, InspectRequest::Act { window: w.id, node: only.id, action: actions::TOGGLE, value: String::new() })?;
        wait_tree(&s.inspect, w.id, "filtr zpet", |t| t.iter().any(|n| n.role == Role::ListItem && !n.name.starts_with("agent: ")))?;
        Ok(())
    })
}

/// Snimky vzhledu pro kontrolu proti prototypum (`design/`): okna ukazky,
/// terminalu a spravce uloh s listou ve svetlem, tmavem a kontrastnim
/// motivu. Snimek porizuje xtask pres QMP po znacce.
fn look_shots(startup: &Startup) -> TestResult {
    with(|s| {
        s.gui.command(b"tasks 4 1", Vec::new()).map_err(|e| fail("tasks", e))?;
        for name in SHOTS {
            let before = s.gui.status().map_err(|e| fail("status", e))?.presented;
            s.gui.command(alloc::format!("theme {name}").as_bytes(), Vec::new()).map_err(|e| fail("theme", e))?;
            // Ne `settle`: terminal s bezici ulohou animuje a kompozitor
            // neni nikdy uplne v klidu. Pockat na zobrazeni snimku po
            // zmene: kompozitor po `theme` prekresli celou obrazovku vzdy
            // (i pri stejnem motivu). Pocet dalsich snimku od aplikaci neni
            // pevny (pomaly displej je slije do jednoho, stejny motiv je
            // neprekresli; drive `+ 3` selhavalo v dalsim kole --repeat na
            // bochs-display), na ne se ceka casem nize.
            let end = time::deadline_after(TIMEOUT);
            while s.gui.status().map_err(|e| fail("status", e))?.presented < before + 1 {
                check!(time::now() < end, alloc::format!("snimek po zmene motivu {name}"));
                time::sleep(Duration::from_millis(100));
            }
            // Aplikace prekresluji samy (velke okno je v emulaci pomale).
            time::sleep(Duration::from_millis(3500));
            println!("{QEMU_SHOT}{name}");
            // xtask porizuje snimek; pockat, nez skonci.
            time::sleep(Duration::from_millis(1500));
        }
        s.gui.command(b"theme light", Vec::new()).map_err(|e| fail("theme", e))?;
        check!(s.gui.command(b"theme fialova", Vec::new()).is_err(), "neplatny motiv");
        time::sleep(Duration::from_millis(2500));
        terminal_shot(startup, s)?;
        Ok(())
    })
}

/// Snimek terminalu navrch se scenou jako v prototypu (design/terminal.html):
/// tabulka, chyba, prubeh. Pro porovnani vzhledu terminalu s navrhem.
fn terminal_shot(startup: &Startup, s: &mut State) -> TestResult {
    // Okna terminalu z drivejsich testu: nove se pozna podle id.
    let before: Vec<u32> = windows(&s.inspect)?.into_iter().map(|w| w.id).collect();
    let conn = crate::terminal::TERMINALS.lock().client("gui-term (snimek)").map_err(|e| fail("kanal terminalu", e))?;
    let app = s
        .gui
        .launch_extra(startup, "gui-term", vec![StartHandle::user(conn.into_handle())])
        .map_err(|e| fail("spusteni gui-term", e))?;
    let pump = || crate::terminal::TERMINALS.lock().poll(startup);
    // Nove okno je navrch; ceka se na uvitani (relace pripojena).
    let end = time::deadline_after(TIMEOUT);
    let w = loop {
        if let Some(w) = windows(&s.inspect)?.into_iter().find(|w| w.app == "org.magnumopus.terminal" && !before.contains(&w.id)) {
            break w;
        }
        check!(time::now() < end, "okno terminalu pro snimek se neobjevilo");
        time::sleep(Duration::from_millis(50));
    };
    let t = wait_tree(&s.inspect, w.id, "uvitani terminalu (snimek)", |t| {
        pump();
        t.iter().any(|n| n.role == Role::Terminal && n.value.contains("Napište help"))
    })?;
    let term = t.iter().find(|n| n.role == Role::Terminal).unwrap();
    // Slozka ve vyzve (pilulka) jina nez koren: cela cesta.
    for line in ["cd /tmp", "ls /", "open /neni-tu.txt", "range 0 5 | each x => x * x"] {
        ask(&s.inspect, InspectRequest::Act { window: w.id, node: term.id, action: actions::SET_VALUE, value: line.into() })?;
    }
    wait_tree(&s.inspect, w.id, "scena terminalu", |t| {
        pump();
        t.iter().any(|n| n.role == Role::Terminal && n.value.lines().any(|l| l.trim() == "16"))
    })?;
    // Nahled zmen s tlacitky (zamek / preview): na snimku je ramecek se
    // seznamem zmen a tlacitkem Prevzit.
    let line = "preview { echo a > /tmp/nahled-a.txt; echo b > /tmp/nahled-b.txt; echo c > /tmp/nahled-c.txt }";
    ask(&s.inspect, InspectRequest::Act { window: w.id, node: term.id, action: actions::SET_VALUE, value: line.into() })?;
    let t = wait_tree(&s.inspect, w.id, "nahled zmen v terminalu", |t| {
        pump();
        t.iter().any(|n| n.role == Role::Button && n.name.starts_with("Převzít 3 změn"))
    })?;
    time::sleep(Duration::from_millis(2500));
    println!("{QEMU_SHOT}terminal");
    time::sleep(Duration::from_millis(1500));
    // Tlacitko "Zahodit" (pristupnost = stejna cesta jako klik): nic se
    // nezmeni, blok skonci odmitnutim.
    let drop = t.iter().find(|n| n.role == Role::Button && n.name.starts_with("Zahodit")).ok_or("tlacitko Zahodit chybi")?;
    ask(&s.inspect, InspectRequest::Act { window: w.id, node: drop.id, action: actions::ACTIVATE, value: String::new() })?;
    wait_tree(&s.inspect, w.id, "odmitnuti nahledu", |t| {
        pump();
        t.iter().any(|n| n.role == Role::Terminal && n.value.contains("nebyly převzaty"))
    })?;
    close_terminal(startup, s, app, "gui-term (snimek)")
}

/// Konec testu terminalu: okno zavrit, init pak ukonci jeho termd a relace
/// shellu skonci. Pamet procesu jadro vrati az se zavrenim posledniho
/// handlu, proto se handle aplikace i zavreny klient (nahradni kanal relace)
/// zahodi; jinak kazde okno terminalu drzi ~20 MiB do konce testu a pamet
/// v dalsich testech dojde.
pub(super) fn close_terminal(startup: &Startup, s: &mut State, app: usize, name: &str) -> TestResult {
    if let Some(a) = s.gui.apps.get(app) {
        let _ = a.process.kill();
        let _ = a.process.wait_exit(Some(time::deadline_after(TIMEOUT)));
    }
    // Indexy aplikaci jinde drzi jen relace; posledni spustena jde odebrat.
    if app + 1 == s.gui.apps.len() {
        drop(s.gui.apps.pop());
    }
    let end = time::deadline_after(TIMEOUT);
    loop {
        let mut t = crate::terminal::TERMINALS.lock();
        t.poll(startup);
        if t.is_closed(name) {
            // Smycku supervisoru tu nahrazuje test: zavrene klienty odebrat.
            t.cleanup();
            return Ok(());
        }
        drop(t);
        check!(time::now() < end, "termd po zavreni okna terminalu neskoncil");
        time::sleep(Duration::from_millis(50));
    }
}
