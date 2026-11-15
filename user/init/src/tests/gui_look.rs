//! Volba vzhledu (`appearance`): plynuly prechod barev na obrazovce,
//! okamzita zmena s Omezit pohyb, aplikace se prekresli, ulozeni volby.

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
