//! Textova a zachranna konzole (`user/console`) v systemu: framebuffer
//! z firmware, fokus od inputd, termd jako sluzba relaci.
//!
//! Test hraje roli supervisoru: preda konzoli displej (`display`) a fokus
//! (`focus`, stejne predani jako kompozitoru), umelym vstupem napise
//! `echo ahoj` + Enter a z textu obrazovky (`dump`) overi radek, blok
//! a vystup. Pak displej vezme zpet (`release`). Bezi jako posledni:
//! fokus zustane u konzole a kanal fokusu initu se zavre.

use alloc::string::String;
use alloc::vec::Vec;
use core::time::Duration;

use inputproto::code::*;
use inputproto::device::{self, RawEvent};
use libos::initfs::Initfs;
use libos::spawn::StartHandle;
use libos::{Channel, Handle, Startup, println, time};

use super::{TestResult, devices, fail};

/// Ridici prikaz konzoli; odpoved a handly.
fn command(ctl: &Channel, text: &str, handles: Vec<Handle>) -> Result<(String, Vec<Handle>), String> {
    ctl.write(text.as_bytes(), handles).map_err(|e| fail("prikaz konzoli", e))?;
    let m = ctl.read(Some(time::deadline_after(Duration::from_secs(10)))).map_err(|e| fail("odpoved konzole", e))?;
    Ok((String::from_utf8_lossy(&m.data).into_owned(), m.handles))
}

fn ok(ctl: &Channel, text: &str, handles: Vec<Handle>) -> Result<Vec<Handle>, String> {
    let (reply, hs) = command(ctl, text, handles)?;
    if !reply.starts_with("ok") {
        return Err(alloc::format!("konzole na '{text}': {reply}"));
    }
    Ok(hs)
}

fn tap(code: u16) -> [RawEvent; 4] {
    [RawEvent::key(code, true), RawEvent::syn(), RawEvent::key(code, false), RawEvent::syn()]
}

pub(super) fn console(s: &Startup) -> TestResult {
    let Some(fb) = s.framebuffer.as_ref() else {
        println!("init: test konzole: firmware nedal framebuffer, kresleni se neoveri");
        return Ok(());
    };
    let inp = super::input::input()?;
    let fs = devices().and_then(|d| d.fs.as_ref()).ok_or("sluzba fsd nebezi")?;
    let initfs = Initfs::open(s).map_err(|e| fail("initfs", e))?;
    let vmo = s.initfs.as_ref().ok_or("initfs chybi")?;
    // termd bez registru: okno = konzole.
    let own = fs.connect().map_err(|e| fail("nova relace fsd", e))?;
    let handles = alloc::vec![
        StartHandle::user(own.into_channel().into_handle()),
        StartHandle::new(libos::abi::bootstrap::kind::INITFS, 0, vmo.handle().duplicate(None).map_err(|e| fail("initfs", e))?),
    ];
    let image = initfs.find("termd").ok_or("termd neni v initfs")?;
    let (_termd, window) = libos::spawn("termd", image, handles).map_err(|e| fail("spusteni termd", e))?;
    let image = initfs.find("console").ok_or("console neni v initfs")?;
    let (_console, ctl) = libos::spawn("console", image, alloc::vec![StartHandle::user(window.into_handle())])
        .map_err(|e| fail("spusteni console", e))?;

    // Displej a fokus preda vlastnik (tady test, pozdeji supervisor).
    let fbh = fb.handle().duplicate(None).map_err(|e| fail("framebuffer", e))?;
    ok(&ctl, "display", alloc::vec![fbh])?;
    let (mine, theirs) = Channel::create().map_err(|e| fail("kanal fokusu", e))?;
    inp.command(inputproto::control::FOCUS, alloc::vec![theirs.into_handle()]).map_err(|e| fail("focus", e))?;
    ok(&ctl, "focus", alloc::vec![mine.into_handle()])?;

    // Az konzole ma relaci (termd ji zaklada ve vlastnim procesu).
    let end = time::deadline_after(Duration::from_secs(20));
    loop {
        let (st, _) = command(&ctl, "status", Vec::new())?;
        if !st.contains("session=0") {
            break;
        }
        if time::now() > end {
            return Err(alloc::format!("konzole se nepripojila k relaci: {st}"));
        }
        time::sleep(Duration::from_millis(100));
    }
    // Umely vstup: `echo ahoj` + Enter.
    let inj = inp.inject().map_err(|e| fail("inject", e))?;
    inj.write(&device::encode_hello(device::caps::KEYBOARD, "test konzole"), Vec::new())
        .map_err(|e| fail("predstaveni", e))?;
    let mut ev = Vec::new();
    for k in [KEY_E, KEY_C, KEY_H, KEY_O, KEY_SPACE, KEY_A, KEY_H, KEY_O, KEY_J, KEY_ENTER] {
        ev.extend(tap(k));
    }
    inj.write(&device::encode_events(&ev), Vec::new()).map_err(|e| fail("zapis vstupu", e))?;

    // Obrazovka: blok s prikazem a jeho vystup.
    let end = time::deadline_after(Duration::from_secs(20));
    let mut screen = String::new();
    while time::now() < end {
        let (reply, _) = command(&ctl, "dump", Vec::new())?;
        screen = reply;
        if screen.contains("| echo ahoj") && screen.lines().any(|l| l.trim() == "ahoj") {
            break;
        }
        time::sleep(Duration::from_millis(200));
    }
    if !(screen.contains("| echo ahoj") && screen.lines().any(|l| l.trim() == "ahoj")) {
        return Err(alloc::format!("na obrazovce konzole chybi prikaz nebo vystup:\n{screen}"));
    }
    if !screen.contains("relace 1") {
        return Err(alloc::format!("stavovy radek bez relace:\n{screen}"));
    }
    let (st, _) = command(&ctl, "status", Vec::new())?;
    if !st.contains("drawing=1") || !st.contains("focus=1") || !st.contains("connected=1") {
        return Err(alloc::format!("stav konzole: {st}"));
    }
    // Vraceni displeje: odpoved nese framebuffer pro dalsiho vlastnika.
    let hs = ok(&ctl, "release", Vec::new())?;
    if hs.len() != 1 {
        return Err("release nevratil framebuffer".into());
    }
    let (st, _) = command(&ctl, "status", Vec::new())?;
    if !st.contains("drawing=0") || !st.contains("focus=0") {
        return Err(alloc::format!("stav po release: {st}"));
    }
    Ok(())
}
