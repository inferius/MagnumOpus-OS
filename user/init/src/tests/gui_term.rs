//! Okno terminalu a fyzicka mys a klavesnice (QMP): tah mysi vybere radek
//! vystupu, Ctrl+C ho zkopiruje do schranky, Ctrl+V vlozi do radku vstupu.
//! Schranka chce fyzicke gesto (umely vstup ani pristupnost ji neotevrou),
//! proto QMP: xtask tahne mysi po znacce `QEMU_TERM_DRAG` a posle klavesy
//! po `QEMU_TERM_KEYS` (`xtask/src/qmp.rs`, modul `gui`).

use alloc::vec;

use guiproto::a11y::{Node, Role, actions};
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

const NAME: &str = "gui-term (vyber)";
const WORD: &str = "vyber-z-mysi";

fn term(t: &[Node]) -> Option<&Node> {
    t.iter().find(|n| n.role == Role::Terminal)
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
            // Radek hodnoty = radek mrizky, plocha uzlu = mrizka (meze
            // z inspekce jsou v souradnicich obrazovky).
            let n = term(&t).ok_or("terminal chybi")?;
            let rows = n.value.lines().count().max(1);
            let row = n.value.lines().position(|l| l.trim() == WORD).ok_or("radek vystupu")?;
            let ch = n.bounds.h as i32 / rows as i32;
            let y = n.bounds.y + row as i32 * ch + ch / 2;
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
            Ok(())
        })();
        close_terminal(startup, s, app, NAME)?;
        r
    })
}
