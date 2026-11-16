//! Volba vzhledu (`appearance`): plynuly prechod barev na obrazovce,
//! okamzita zmena s Omezit pohyb, aplikace se prekresli, aplikace
//! spustena v tmavem rezimu je tmava od prvniho snimku, ulozeni volby.

use alloc::vec::Vec;
use core::time::Duration;

use libos::{Startup, time};
use theme::{Mode, Pref, Scheme, Theme};

use guiproto::a11y::{Role, actions};
use guiproto::control::InspectRequest;

use super::TestResult;
use super::gui::{pixel, settle, tree, wait_tree, windows, with};
use crate::check;

fn cmd(s: &super::gui::State, text: &str) -> TestResult {
    s.gui.command(text.as_bytes(), Vec::new()).map(|_| ()).map_err(|e| super::fail(text, e))
}

pub fn fade(_: &Startup) -> TestResult {
    with(|s| {
        let w = windows(&s.inspect)?.into_iter().find(|w| w.id == s.demo).ok_or("okno ukazky")?;
        // Odznak duvery kresli kompozitor kazdy snimek; obsah okna aplikace.
        let (bx, by) = (w.rect.x + 9, w.rect.y - 16);
        let (ax, ay) = (w.rect.x + w.rect.w as i32 / 2, w.rect.y + w.rect.h as i32 - 4);
        settle(&s.inspect)?;
        let (light, dark) = (Theme::builtin(Mode::Light), Theme::builtin(Mode::Dark));
        check!(pixel(&s.inspect, bx, by)? == light.color.trust_bg, "na zacatku svetly odznak");
        // Prechod: behem nej odznak projde mezibarvami.
        let st0 = s.gui.status().map_err(|e| super::fail("status", e))?;
        let t0 = time::now();
        cmd(s, "appearance system tmavy ne ne")?;
        // Behem prechodu se da pracovat: klik pres pristupnost a nove
        // prekresleni aplikace (pocitadlo) se objevi jeste v prolnuti.
        let t = tree(&s.inspect, s.demo)?;
        let b = t.iter().find(|n| n.role == Role::Button && n.name == "Přičíst").ok_or("tlacitko Pricist")?;
        let before = t.iter().find(|n| n.name.starts_with("Počítadlo")).map(|n| n.name.clone()).unwrap_or_default();
        super::gui::ask(&s.inspect, InspectRequest::Act { window: s.demo, node: b.id, action: actions::ACTIVATE, value: alloc::string::String::new() })?;
        let clicked_ms = (time::now() - t0) / 1_000_000;
        let mut seen: Vec<u32> = Vec::new();
        let end = time::deadline_after(Duration::from_secs(10));
        loop {
            let p = pixel(&s.inspect, bx, by)?;
            if !seen.contains(&p) {
                seen.push(p);
            }
            if p == dark.color.trust_bg {
                break;
            }
            check!(time::now() < end, alloc::format!("odznak neprosel do tmave: {seen:08x?}"));
        }
        wait_tree(&s.inspect, s.demo, "pocitadlo po kliku behem prechodu", |t| t.iter().any(|n| n.name.starts_with("Počítadlo") && n.name != before))?;
        let mid = seen.iter().filter(|&&p| p != light.color.trust_bg && p != dark.color.trust_bg).count();
        let ms = (time::now() - t0) / 1_000_000;
        let st1 = s.gui.status().map_err(|e| super::fail("status", e))?;
        let frames = st1.frames - st0.frames;
        libos::println!(
            "init: test vzhledu: prechod {ms} ms (token {} ms), snimku {frames} (prumer {} ms), mezibarev odznaku {mid}, klik poslan v {clicked_ms} ms prechodu",
            dark.motion.theme,
            ms / frames.max(1)
        );
        check!(mid >= 1, alloc::format!("zadna mezibarva: {seen:08x?}"));
        // Aplikace se prekresli tmave (konecny `Theme`).
        let end = time::deadline_after(Duration::from_secs(10));
        while pixel(&s.inspect, ax, ay)? != dark.color.surface {
            check!(time::now() < end, "okno aplikace se neprekreslilo do tmave");
            time::sleep(Duration::from_millis(50));
        }
        wait_tree(&s.inspect, s.demo, "strom po zmene vzhledu", |t| !t.is_empty())?;
        // Omezit pohyb: hned, bez mezibarev.
        cmd(s, "appearance system svetly ne ano")?;
        settle(&s.inspect)?;
        check!(pixel(&s.inspect, bx, by)? == light.color.trust_bg, "omezit pohyb: svetly hned");
        check!(s.gui.command(b"appearance system modry ne ne", Vec::new()).is_err(), "neplatna volba");
        check!(s.gui.command(b"appearance user x svetly ne ne", Vec::new()).is_err(), "neplatne uid");
        let _ = tree(&s.inspect, s.demo)?;
        // Zpet jako na zacatku sady (dalsi testy ocekavaji svetly).
        cmd(s, "appearance system svetly ne ano")?;
        Ok(())
    })
}

/// Znacka snimku obrazovky po startu aplikaci v tmavem rezimu (xtask
/// `qmp.rs`, `gui::DARK_START`).
pub const QEMU_SHOT_DARK_START: &str = "init: test gui: snimek tmavy-start";

/// Je pixel tmavy (jas pod ctvrtinou)?
fn dark_px(p: u32) -> bool {
    let (r, g, b) = ((p >> 16) & 0xff, (p >> 8) & 0xff, p & 0xff);
    (r * 299 + g * 587 + b * 114) / 1000 < 64
}

/// Nove okno aplikace `app` (jine nez `before`).
fn new_window(s: &super::gui::State, app: &str, before: &[u32]) -> Result<guiproto::control::WindowInfo, alloc::string::String> {
    let end = time::deadline_after(Duration::from_secs(20));
    loop {
        if let Some(w) = windows(&s.inspect)?.into_iter().find(|w| w.app == app && !before.contains(&w.id)) {
            return Ok(w);
        }
        check!(time::now() < end, alloc::format!("nove okno {app} se neobjevilo"));
        time::sleep(Duration::from_millis(50));
    }
}

/// Aplikace spustene az v tmavem rezimu (jako po startu systemu s ulozenou
/// tmavou volbou): rezim prijde v pozdravu kompozitoru (`THEME` hned po
/// `WELCOME`) a obsah oken je tmavy od prvniho snimku, bez dalsi zmeny.
/// Ukazka (`ui`) i terminal (vlastni barvy `term_*`); snimek obrazovky
/// `target/shots/tmavy-start.png`.
pub fn start_dark(startup: &Startup) -> TestResult {
    with(|s| {
        cmd(s, "appearance system tmavy ne ano")?;
        settle(&s.inspect)?;
        let before: Vec<u32> = windows(&s.inspect)?.into_iter().map(|w| w.id).collect();
        let first = s.gui.apps.len();
        let r = (|| -> TestResult {
            s.gui.launch(startup, "gui-demo").map_err(|e| super::fail("spusteni gui-demo", e))?;
            let demo = new_window(s, "org.magnumopus.demo", &before)?;
            wait_tree(&s.inspect, demo.id, "strom noveho okna", |t| t.iter().any(|n| n.role == Role::Button && n.name == "Přičíst"))?;
            settle(&s.inspect)?;
            // Ukazka pred spustenim terminalu (ten se otevre navrch).
            let dark = Theme::builtin(Mode::Dark);
            let p = pixel(&s.inspect, demo.rect.x + demo.rect.w as i32 / 2, demo.rect.y + demo.rect.h as i32 - 4)?;
            check!(p == dark.color.surface, alloc::format!("obsah noveho okna v tmavem rezimu: {p:08x}, cekano {:08x}", dark.color.surface));
            let conn = crate::terminal::TERMINALS.lock().client("gui-term (tmavy start)").map_err(|e| super::fail("kanal terminalu", e))?;
            s.gui
                .launch_extra(startup, "gui-term", alloc::vec![libos::spawn::StartHandle::user(conn.into_handle())])
                .map_err(|e| super::fail("spusteni gui-term", e))?;
            let pump = || crate::terminal::TERMINALS.lock().poll(startup);
            let term = new_window(s, "org.magnumopus.terminal", &before)?;
            wait_tree(&s.inspect, term.id, "uvitani terminalu", |t| {
                pump();
                t.iter().any(|n| n.role == Role::Terminal && n.value.contains("Napište help"))
            })?;
            settle(&s.inspect)?;
            // Terminal: pozadi radku dole (pod uvitanim nic neni).
            let p = pixel(&s.inspect, term.rect.x + term.rect.w as i32 / 2, term.rect.y + term.rect.h as i32 - 8)?;
            check!(dark_px(p), alloc::format!("terminal v tmavem rezimu: pozadi {p:08x}"));
            libos::println!("{QEMU_SHOT_DARK_START}");
            // xtask porizuje snimek; pockat, nez skonci.
            time::sleep(Duration::from_millis(1500));
            Ok(())
        })();
        while s.gui.apps.len() > first {
            if let Some(a) = s.gui.apps.pop() {
                let _ = a.process.kill();
                let _ = a.process.wait_exit(Some(time::deadline_after(Duration::from_secs(5))));
            }
        }
        // Zpet jako na zacatku sady (dalsi testy ocekavaji svetly).
        cmd(s, "appearance system svetly ne ano")?;
        settle(&s.inspect)?;
        r
    })
}

/// Ulozeni volby do `/system/config/vzhled` a zpet (format, prava).
pub fn store(_: &Startup) -> TestResult {
    let p = Pref { scheme: Scheme::Dark, contrast: false, reduce_motion: true };
    crate::appearance::save_for(Some(1_000_123), &p).map_err(|e| super::fail("ulozeni", e))?;
    check!(crate::appearance::user(1_000_123) == p, "volba uzivatele zpet");
    let fs = crate::system::devices().and_then(|d| d.fs.as_ref()).ok_or("FS")?;
    let perm = fs.permissions(crate::appearance::DIR).map_err(|e| super::fail("prava", e))?;
    check!(perm.acl.is_some_and(|a| a.flags & fsproto::perm::ACL_PROTECTED != 0 && a.grants.is_empty()), "adresar jen pro system");
    let _ = fs.remove(&alloc::format!("{}/1000123.toml", crate::appearance::DIR));
    Ok(())
}
