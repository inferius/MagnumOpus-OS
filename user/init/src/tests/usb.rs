//! USB: radic xHCI, virtualni root hub, ovladac hubu, HID boot protokol,
//! hotplug a overeni nove klavesnice (volba proti BadUSB).
//!
//! Testy s udalostmi z QEMU se synchronizuji znackami v logu jako testy
//! vstupu: xtask (`xtask/src/qmp.rs`) na kazdou znacku posle udalosti nebo
//! pripoji / odpoji zarizeni (`device_add`, `device_del`).

use alloc::string::String;
use alloc::vec::Vec;
use core::time::Duration;

use inputproto::code::*;
use inputproto::control::{self, SourceInfo};
use inputproto::{Action, Button, Event, DeviceTrust, Record};
use libos::{Startup, println, time};

use super::input::{deadline, failr, has, input, text};
use super::{Test, TestResult, devices, fail};
use crate::input::{Input, drain, pump_hotplug, read_until};
use crate::{check, check_eq};

pub const TESTS: &[Test] = &[
    ("root hub nasel klavesnici USB", boot_keyboard),
    ("QEMU: klavesnice USB az do sluzby", qemu_keyboard),
    ("hotplug: mys USB pripojena a odpojena", hotplug_mouse),
    ("nova klavesnice ceka na overeni (BadUSB)", verify_new_keyboard),
    ("nova mys ceka na overeni (BadUSB)", verify_new_mouse),
    ("externi hub: klavesnice za hubem", external_hub),
];

// Znacky (musi sedet s `xtask/src/qmp.rs`).
pub const MARKER_KEYS: &str = "init: test usb: stisky na video1";
pub const MARKER_MOUSE_PLUG: &str = "init: test usb: pripoj mys";
pub const MARKER_MOUSE_READY: &str = "init: test usb: udalosti mysi";
pub const MARKER_MOUSE_UNPLUG: &str = "init: test usb: odpoj mys";
pub const MARKER_KBD_PLUG: &str = "init: test usb: pripoj klavesnici";
pub const MARKER_KBD_READY: &str = "init: test usb: klavesy nove klavesnice";
pub const MARKER_KBD_TRUSTED: &str = "init: test usb: klavesy overene klavesnice";
pub const MARKER_KBD_UNPLUG: &str = "init: test usb: odpoj klavesnici";
pub const MARKER_MOUSE2_PLUG: &str = "init: test usb: pripoj druhou mys";
pub const MARKER_MOUSE2_PENDING: &str = "init: test usb: pohyb neoverene mysi";
pub const MARKER_MOUSE2_TRUSTED: &str = "init: test usb: pohyb overene mysi";
pub const MARKER_MOUSE2_UNPLUG: &str = "init: test usb: odpoj druhou mys";
pub const MARKER_HUB_PLUG: &str = "init: test usb: pripoj hub";
pub const MARKER_HUB_KEYS: &str = "init: test usb: klavesy za hubem";
pub const MARKER_HUB_UNPLUG: &str = "init: test usb: odpoj hub";

/// Jmeno klavesnice QEMU usb-kbd (vyrobce 0627, vyrobek 0001).
const USB_KBD: &str = "USB klavesnice 0627:0001";
const USB_MOUSE: &str = "USB mys 0627:0001";
/// Na udalosti a zmeny zarizeni z QEMU (TCG muze byt pomaly).
const TIMEOUT: Duration = Duration::from_secs(20);

fn sources(inp: &Input) -> Result<Vec<(u16, DeviceTrust, String)>, String> {
    let (text, _) = inp
        .command(control::SOURCES, Vec::new())
        .map_err(|e| fail("sources", e))?;
    Ok(SourceInfo::parse_list(&text)
        .filter(|s| !s.synthetic)
        .map(|s| (s.id, s.trust, String::from(s.name)))
        .collect())
}

/// Pocka, az se objevi zdroj se jmenem `name`, ktery nema id z `old`
/// (mezitim predava sluzbe vstupu zarizeni pripojena za behu).
fn wait_new_source(inp: &Input, name: &str, old: &[u16]) -> Result<(u16, DeviceTrust), String> {
    let d = devices().ok_or("spravce zarizeni nebezi")?;
    let until = deadline(TIMEOUT);
    loop {
        pump_hotplug(&d.control, inp);
        if let Some((id, trust, _)) = sources(inp)?
            .into_iter()
            .find(|(id, _, n)| n == name && !old.contains(id))
        {
            return Ok((id, trust));
        }
        if time::now() > until {
            return Err(alloc::format!("zarizeni '{name}' se neobjevilo"));
        }
        libos::time::sleep(Duration::from_millis(50));
    }
}

/// Pocka, az zdroj `id` zmizi (zarizeni odpojeno).
fn wait_gone(inp: &Input, id: u16) -> TestResult {
    let until = deadline(TIMEOUT);
    while sources(inp)?.iter().any(|(i, _, _)| *i == id) {
        if time::now() > until {
            return Err(alloc::format!("zdroj {id} po odpojeni nezmizel"));
        }
        libos::time::sleep(Duration::from_millis(50));
    }
    Ok(())
}

fn ids(inp: &Input) -> Result<Vec<u16>, String> {
    Ok(sources(inp)?.into_iter().map(|(id, _, _)| id).collect())
}

fn boot_keyboard(_: &Startup) -> TestResult {
    let inp = input()?;
    let list = sources(inp)?;
    let found = list.iter().find(|(_, _, n)| n == USB_KBD);
    check!(found.is_some(), "klavesnice USB ve zdrojich chybi");
    check_eq!(
        found.map(|(_, t, _)| *t),
        Some(DeviceTrust::Trusted),
        "klavesnice pripojena pri startu je duveryhodna"
    );
    Ok(())
}

fn qemu_keyboard(_: &Startup) -> TestResult {
    let inp = input()?;
    let id = inp.source_id(USB_KBD).map_err(|e| fail("zdroj klavesnice USB", e))?;
    drain(&inp.focus);
    println!("{MARKER_KEYS}");
    // xtask: u, Shift+3 na konzoli video1 (klavesnice USB).
    let got = read_until(&inp.focus, deadline(TIMEOUT), |r| text(r).contains("u#"))
        .map_err(|e| failr("klavesy USB z QEMU", e))?;
    check!(
        got.iter()
            .filter(|r| matches!(r.event, Event::Key { .. } | Event::Text(_)))
            .all(|r| r.source == id),
        "klavesy prisly z jineho zdroje nez USB"
    );
    // MSC_SCAN = HID usage 0x7xxxx (u = 0x18).
    check!(
        got.iter().any(|r| matches!(
            r.event,
            Event::Key {
                code: KEY_U,
                scancode: 0x70018,
                action: Action::Press,
                ..
            }
        )),
        "klavesa U nema scan kod HID 0x70018"
    );
    Ok(())
}

fn hotplug_mouse(_: &Startup) -> TestResult {
    let inp = input()?;
    let old = ids(inp)?;
    println!("{MARKER_MOUSE_PLUG}");
    let (id, _) = wait_new_source(inp, USB_MOUSE, &old)?;
    drain(&inp.focus);
    println!("{MARKER_MOUSE_READY}");
    // xtask: pohyb (4, 6), prostredni tlacitko, kolecko dolu (bez zarizeni:
    // QEMU je posle naposled aktivni mysi = USB).
    let release = Event::Button {
        button: Button::Middle,
        action: Action::Release,
    };
    let got = read_until(&inp.focus, deadline(TIMEOUT), |r| {
        has(r, release) && r.iter().any(|x| matches!(x.event, Event::Scroll { .. }))
    })
    .map_err(|e| failr("udalosti mysi USB", e))?;
    let mine: Vec<&Record> = got.iter().filter(|r| r.source == id).collect();
    let (dx, dy) = mine.iter().fold((0, 0), |(x, y), r| match r.event {
        Event::Motion { dx, dy } => (x + dx, y + dy),
        _ => (x, y),
    });
    check_eq!((dx, dy), (4, 6), "pohyb mysi USB");
    let wheel: i32 = mine
        .iter()
        .map(|r| match r.event {
            Event::Scroll { dy, .. } => dy,
            _ => 0,
        })
        .sum();
    check_eq!(wheel, -1, "kolecko mysi USB dolu");
    println!("{MARKER_MOUSE_UNPLUG}");
    wait_gone(inp, id)
}

fn verify_new_keyboard(_: &Startup) -> TestResult {
    let inp = input()?;
    inp.command("policy verify-new-devices on", Vec::new())
        .map_err(|e| fail("zapnuti overeni", e))?;
    let r = verify_inner(inp);
    let _ = inp.command("policy verify-new-devices off", Vec::new());
    r
}

fn verify_inner(inp: &Input) -> TestResult {
    let old = ids(inp)?;
    drain(&inp.focus);
    drain(&inp.secure);
    println!("{MARKER_KBD_PLUG}");
    let (id, trust) = wait_new_source(inp, USB_KBD, &old)?;
    check_eq!(trust, DeviceTrust::Pending, "nova klavesnice ceka na overeni");
    let pending = Event::NewDevice {
        source: id,
        state: DeviceTrust::Pending,
    };
    read_until(&inp.focus, deadline(TIMEOUT), |r| has(r, pending))
        .map_err(|e| failr("oznameni nove klavesnice (fokus)", e))?;
    read_until(&inp.secure, deadline(TIMEOUT), |r| has(r, pending))
        .map_err(|e| failr("oznameni nove klavesnice (secure)", e))?;
    drain(&inp.focus);
    println!("{MARKER_KBD_READY}");
    // xtask: x na nove klavesnici - vidi ho jen system (kanal secure).
    let got = read_until(&inp.secure, deadline(TIMEOUT), |r| {
        r.iter().any(|x| x.source == id && x.event == Event::Text('x'))
    })
    .map_err(|e| failr("klavesa nove klavesnice na secure", e))?;
    check!(
        got.iter().filter(|r| r.source == id).count() >= 2,
        "chybi Key a Text od nove klavesnice"
    );
    libos::time::sleep(Duration::from_millis(300));
    while let Ok(m) = inp.focus.try_read() {
        let recs = inputproto::event::decode(&m.data).ok_or("neplatna zprava")?;
        check!(
            !recs.iter().any(|r| r.source == id),
            "neoverena klavesnice pise vlastnikovi fokusu"
        );
    }
    inp.secure
        .write(alloc::format!("trust {id}").as_bytes(), Vec::new())
        .map_err(|e| fail("trust", e))?;
    let trusted = Event::NewDevice {
        source: id,
        state: DeviceTrust::Trusted,
    };
    read_until(&inp.focus, deadline(TIMEOUT), |r| has(r, trusted))
        .map_err(|e| failr("overeni klavesnice", e))?;
    println!("{MARKER_KBD_TRUSTED}");
    read_until(&inp.focus, deadline(TIMEOUT), |r| {
        r.iter().any(|x| x.source == id && x.event == Event::Text('y'))
    })
    .map_err(|e| failr("klavesa overene klavesnice", e))?;
    println!("{MARKER_KBD_UNPLUG}");
    wait_gone(inp, id)
}

fn verify_new_mouse(_: &Startup) -> TestResult {
    let inp = input()?;
    inp.command("policy verify-new-devices on", Vec::new())
        .map_err(|e| fail("zapnuti overeni", e))?;
    let r = verify_mouse_inner(inp);
    let _ = inp.command("policy verify-new-devices off", Vec::new());
    r
}

/// Soucet pohybu zdroje `id` v zaznamech.
fn motion_of(records: &[Record], id: u16) -> (i32, i32) {
    records
        .iter()
        .filter(|r| r.source == id)
        .fold((0, 0), |(x, y), r| match r.event {
            Event::Motion { dx, dy } => (x + dx, y + dy),
            _ => (x, y),
        })
}

fn verify_mouse_inner(inp: &Input) -> TestResult {
    let old = ids(inp)?;
    drain(&inp.focus);
    drain(&inp.secure);
    println!("{MARKER_MOUSE2_PLUG}");
    let (id, trust) = wait_new_source(inp, USB_MOUSE, &old)?;
    check_eq!(trust, DeviceTrust::Pending, "nova mys ceka na overeni");
    drain(&inp.focus);
    println!("{MARKER_MOUSE2_PENDING}");
    // xtask: pohyb (2, 3) - vidi ho jen system.
    let got = read_until(&inp.secure, deadline(TIMEOUT), |r| motion_of(r, id) == (2, 3))
        .map_err(|e| failr("pohyb neoverene mysi na secure", e))?;
    check_eq!(motion_of(&got, id), (2, 3), "pohyb na secure");
    libos::time::sleep(Duration::from_millis(300));
    while let Ok(m) = inp.focus.try_read() {
        let recs = inputproto::event::decode(&m.data).ok_or("neplatna zprava")?;
        check!(
            !recs.iter().any(|r| r.source == id),
            "neoverena mys hybe ukazatelem vlastnika fokusu"
        );
    }
    inp.secure
        .write(alloc::format!("trust {id}").as_bytes(), Vec::new())
        .map_err(|e| fail("trust", e))?;
    let trusted = Event::NewDevice {
        source: id,
        state: DeviceTrust::Trusted,
    };
    read_until(&inp.focus, deadline(TIMEOUT), |r| has(r, trusted))
        .map_err(|e| failr("overeni mysi", e))?;
    println!("{MARKER_MOUSE2_TRUSTED}");
    read_until(&inp.focus, deadline(TIMEOUT), |r| motion_of(r, id) == (5, 1))
        .map_err(|e| failr("pohyb overene mysi", e))?;
    println!("{MARKER_MOUSE2_UNPLUG}");
    wait_gone(inp, id)
}

fn external_hub(_: &Startup) -> TestResult {
    let inp = input()?;
    let old = ids(inp)?;
    println!("{MARKER_HUB_PLUG}");
    // xtask: hub USB na root port a za nej klavesnice (port hubu 1).
    let (id, trust) = wait_new_source(inp, USB_KBD, &old)?;
    check_eq!(trust, DeviceTrust::Trusted, "klavesnice za hubem (overeni vypnute)");
    drain(&inp.focus);
    println!("{MARKER_HUB_KEYS}");
    read_until(&inp.focus, deadline(TIMEOUT), |r| {
        r.iter().any(|x| x.source == id && x.event == Event::Text('h'))
    })
    .map_err(|e| failr("klavesa za hubem", e))?;
    // Odpojeni hubu odpoji i klavesnici za nim.
    println!("{MARKER_HUB_UNPLUG}");
    wait_gone(inp, id)
}
