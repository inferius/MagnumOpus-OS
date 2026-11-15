//! Okno terminalu a fyzicka mys a klavesnice (QMP): tah mysi vybere radek
//! vystupu, Ctrl+C ho zkopiruje do schranky, Ctrl+V vlozi do radku vstupu.
//! Schranka chce fyzicke gesto (umely vstup ani pristupnost ji neotevrou),
//! proto QMP: xtask tahne mysi po znacce `QEMU_TERM_DRAG` a posle klavesy
//! po `QEMU_TERM_KEYS` (`xtask/src/qmp.rs`, modul `gui`). Pak nabidka
//! doplnovani: po `QEMU_TERM_TAB` Ctrl+U, "e" a Tab otevrou seznam navrhu
//! nad radkem vstupu, po `QEMU_TERM_DOWN` sipka dolu vybere dalsi polozku;
//! polozku prijme akce pristupnosti. Nakonec tah za volne misto vlastniho
//! titulku (vpravo od karet a mezi odznakem a kartou) okno posune
//! (`QEMU_TERM_TITLE_RIGHT`, `QEMU_TERM_TITLE_LEFT`).

use alloc::vec;

use guiproto::a11y::{Node, Role, actions, state};
use guiproto::control::InspectRequest;
use libos::spawn::StartHandle;
use libos::{Startup, println};

use super::TestResult;
use super::fail;
use super::gui::{ask, close_terminal, settle, wait_tree, wait_window, with};
use crate::check;

/// Tah mysi: za znackou "x0 y0 x1 y1" (souradnice obrazovky).
pub const QEMU_TERM_DRAG: &str = "init: test gui: terminal tah mysi ";
/// Ctrl+C a Ctrl+V z fyzicke klavesnice.
pub const QEMU_TERM_KEYS: &str = "init: test gui: terminal ceka na Ctrl+C a Ctrl+V";
/// Ctrl+U, "e" a Tab z fyzicke klavesnice.
pub const QEMU_TERM_TAB: &str = "init: test gui: terminal ceka na doplneni Tab";
/// Sipka dolu (az kdyz je seznam otevreny).
pub const QEMU_TERM_DOWN: &str = "init: test gui: terminal ceka na sipku v seznamu navrhu";
/// Tah za volne misto titulku vpravo od karet (pred prepinacem B / C):
/// za znackou "x0 y0 x1 y1".
pub const QEMU_TERM_TITLE_RIGHT: &str = "init: test gui: terminal tah titulku vpravo ";
/// Tah za volne misto titulku mezi odznakem a kartou zpet.
pub const QEMU_TERM_TITLE_LEFT: &str = "init: test gui: terminal tah titulku vlevo ";

const NAME: &str = "gui-term (vyber)";
const WORD: &str = "vyber-z-mysi";

fn term(t: &[Node]) -> Option<&Node> {
    t.iter().find(|n| n.role == Role::Terminal)
}

/// Plocha radku `k` mrizky terminalu `id` (potomek `Row` c. `k`; schovany
/// radek nema plochu).
fn row_rect(t: &[Node], id: u32, k: usize) -> Option<guiproto::Rect> {
    t.iter().filter(|n| n.parent == id && n.role == Role::Row).nth(k).map(|n| n.bounds).filter(|b| b.h > 0)
}

pub fn select_copy_paste(startup: &Startup) -> TestResult {
    with(|s| {
        let conn = crate::terminal::TERMINALS.lock().client(NAME).map_err(|e| fail("kanal terminalu", e))?;
        let app = s.gui.launch_extra(startup, "gui-term", vec![StartHandle::user(conn.into_handle())]).map_err(|e| fail("spusteni gui-term", e))?;
        let pump = || crate::terminal::TERMINALS.lock().poll(startup);
        let r = (|| -> TestResult {
            let w = wait_window(&s.inspect, "org.magnumopus.terminal")?;
            let t = wait_tree(&s.inspect, w.id, "uvitani terminalu", |t| {
                pump();
                term(t).is_some_and(|n| n.value.contains("Napište help"))
            })?;
            let id = term(&t).ok_or("terminal chybi")?.id;
            ask(&s.inspect, InspectRequest::Act { window: w.id, node: id, action: actions::SET_VALUE, value: alloc::format!("echo {WORD}") })?;
            let t = wait_tree(&s.inspect, w.id, "vystup echo", |t| {
                pump();
                term(t).is_some_and(|n| n.value.lines().any(|l| l.trim() == WORD))
            })?;
            // Radek hodnoty = radek mrizky, jeho plocha = potomek `Row`
            // (radky maji ruznou vysku; meze z inspekce jsou v souradnicich
            // obrazovky).
            let n = term(&t).ok_or("terminal chybi")?;
            let row = n.value.lines().position(|l| l.trim() == WORD).ok_or("radek vystupu")?;
            let r = row_rect(&t, n.id, row).ok_or("plocha radku vystupu")?;
            let y = r.y + r.h as i32 / 2;
            let (x0, x1) = (n.bounds.x + 1, n.bounds.x + n.bounds.w as i32 - 2);
            settle(&s.inspect)?;
            println!("{QEMU_TERM_DRAG}{x0} {y} {x1} {y}");
            wait_tree(&s.inspect, w.id, "vyber tahem mysi", |t| {
                pump();
                term(t).is_some_and(|n| n.name.starts_with("Výstup, vybráno: ") && n.name.contains(WORD))
            })?;
            println!("{QEMU_TERM_KEYS}");
            // Ctrl+C vyber zkopiruje (a zrusi), Ctrl+V ho vlozi do radku
            // vstupu (za vyzvu, nic se nespusti).
            let t = wait_tree(&s.inspect, w.id, "vlozeni do radku vstupu", |t| {
                pump();
                term(t).is_some_and(|n| n.name == "Výstup" && n.value.lines().any(|l| l.contains('\u{276F}') && l.contains(WORD)))
            })?;
            let v = term(&t).map(|n| n.value.clone()).unwrap_or_default();
            check!(v.lines().filter(|l| l.trim() == WORD).count() == 1, alloc::format!("vlozeni nic nespustilo: {v}"));
            completion(&s.inspect, w.id, &pump)?;
            title_drag(&s.inspect, w.id, &pump)
        })();
        close_terminal(startup, s, app, NAME)?;
        r
    })
}

/// Nabidka doplnovani: seznam navrhu (ListBox) nad radkem vstupu, sipka
/// dolu vybere druhou polozku (ACTIVE), ACTIVATE na "explain" ji doplni
/// a seznam zavre.
fn completion<R>(inspect: &libos::Channel, w: u32, pump: &dyn Fn() -> R) -> TestResult {
    // Chyba bez celeho stromu (hodnota terminalu je dlouha): jen uzly
    // seznamu a radek vstupu.
    let short = |e: alloc::string::String| -> alloc::string::String { e.chars().take(400).collect() };
    let active = |t: &[Node], k: usize| t.iter().filter(|n| n.role == Role::ListOption).nth(k).is_some_and(|n| n.state & state::ACTIVE != 0);
    println!("{QEMU_TERM_TAB}");
    wait_tree(inspect, w, "seznam navrhu po Tab", |t| {
        pump();
        t.iter().filter(|n| n.role == Role::ListOption).count() > 2 && active(t, 0)
    })
    .map_err(short)?;
    println!("{QEMU_TERM_DOWN}");
    let t = wait_tree(inspect, w, "sipka dolu v seznamu navrhu", |t| {
        pump();
        active(t, 1)
    })
    .map_err(short)?;
    let n = term(&t).ok_or("terminal chybi")?;
    check!(n.state & state::EXPANDED != 0 && n.state & state::AUTOCOMPLETE != 0, alloc::format!("stav radku vstupu {:#x}", n.state));
    let list = t.iter().find(|n| n.role == Role::ListBox).ok_or("ListBox chybi")?;
    check!(list.name == "Příkazy", alloc::format!("zahlavi seznamu: {}", list.name));
    // Seznam lezi nad radkem vstupu (radek s vyzvou).
    let row = n.value.lines().position(|l| l.contains('\u{276F}')).ok_or("radek vstupu")?;
    let input_y = row_rect(&t, n.id, row).ok_or("plocha radku vstupu")?.y;
    check!(list.bounds.h > 0 && list.bounds.y + list.bounds.h as i32 <= input_y, alloc::format!("seznam {:?} neni nad radkem vstupu y={input_y}", list.bounds));
    let opt = t.iter().find(|n| n.role == Role::ListOption && n.name == "explain").ok_or("polozka explain chybi")?;
    ask(inspect, InspectRequest::Act { window: w, node: opt.id, action: actions::ACTIVATE, value: alloc::string::String::new() })?;
    wait_tree(inspect, w, "doplneno explain", |t| {
        pump();
        !t.iter().any(|n| n.role == Role::ListBox)
            && term(t).is_some_and(|n| n.state & state::EXPANDED == 0 && n.value.lines().any(|l| l.contains('\u{276F}') && l.contains("explain")))
    })
    .map_err(short)?;
    Ok(())
}

/// Titulek okna (meze na obrazovce).
fn title(t: &[Node]) -> Option<guiproto::Rect> {
    t.iter().find(|n| n.role == Role::Titlebar).map(|n| n.bounds)
}

/// Tah za volne misto vlastniho titulku: vpravo od karet (za + nebo
/// kartou, pred prepinacem B / C) okno posune o (40, 30), mezi odznakem
/// a kartou zpet. Volne misto ma chovani titulku kompozitoru (oblast
/// `Drag` bez `NoDrag`), karty a prepinac ne.
fn title_drag<R>(inspect: &libos::Channel, w: u32, pump: &dyn Fn() -> R) -> TestResult {
    let short = |e: alloc::string::String| -> alloc::string::String { e.chars().take(400).collect() };
    let t = wait_tree(inspect, w, "titulek terminalu", |t| {
        pump();
        title(t).is_some() && t.iter().any(|n| n.name == "Zápis B (příkazový)")
    })
    .map_err(short)?;
    let bar = title(&t).ok_or("titulek chybi")?;
    // Konec karet: karta relace (uzel 3) nebo posledni karta / +.
    let tabs_end = t.iter().filter(|n| n.parent == 2 && n.role == Role::Button && !n.name.starts_with("Zápis")).map(|n| n.bounds.x + n.bounds.w as i32).max().ok_or("karta chybi")?;
    let tab_x = t.iter().find(|n| n.id == 3).map(|n| n.bounds.x).ok_or("karta relace chybi")?;
    let b = t.iter().find(|n| n.name == "Zápis B (příkazový)").ok_or("prepinac zapisu chybi")?.bounds;
    // Stred mezery (okraj prepinace kolem voleb je mensi nez polovina).
    let gap = b.x - tabs_end;
    check!(gap > 32, alloc::format!("titulek bez volneho mista: karty do {tabs_end}, B od {}", b.x));
    let y = bar.y + bar.h as i32 / 2;
    let x = tabs_end + gap / 2;
    settle(inspect)?;
    println!("{QEMU_TERM_TITLE_RIGHT}{x} {y} {} {}", x + 40, y + 30);
    let moved = wait_tree(inspect, w, "presun okna tahem za titulek vpravo", |t| {
        pump();
        title(t).is_some_and(|r| r.x == bar.x + 40 && r.y == bar.y + 30)
    })
    .map_err(short)?;
    let bar2 = title(&moved).ok_or("titulek chybi")?;
    // Mezi odznakem (system, vlevo) a kartou: kousek pred kartou.
    let x = bar2.x + (tab_x - bar.x) - 6;
    let y = bar2.y + bar2.h as i32 / 2;
    settle(inspect)?;
    println!("{QEMU_TERM_TITLE_LEFT}{x} {y} {} {}", x - 40, y - 30);
    wait_tree(inspect, w, "presun okna tahem za titulek vlevo", |t| {
        pump();
        title(t).is_some_and(|r| r.x == bar.x && r.y == bar.y)
    })
    .map_err(short)?;
    Ok(())
}
