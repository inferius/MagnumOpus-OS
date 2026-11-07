//! init: prvni proces v userspace.
//!
//! Spusti sluzbu casu, spravce zarizeni (ten spusti ovladace), sluzbu
//! souboroveho systemu, sluzbu vstupu, sluzbu site s firewallem a GUI
//! (kompozitor a ukazkove aplikace, `gui.rs`; displej je virtio-gpu od
//! spravce zarizeni, jinak `fbdisplay` nad framebufferem z firmwaru). Pak bezi
//! jako supervisor (`supervisor.rs`: zarizeni pripojena za behu, restart
//! spadlych sluzeb). V testovacim behu (priznak `TEST_RUN` od jadra) misto
//! toho spusti testy userspace (`tests/`) a skonci s jejich vysledkem.
//! Pozdeji bude spoustet systemove sluzby podle konfigurace.

#![no_std]
#![no_main]

extern crate alloc;

mod apps;
mod gui;
mod input;
mod supervisor;
mod system;
mod tests;

use libos::{Startup, println};

libos::entry!(main);

fn main(mut startup: Startup) -> i64 {
    println!("init: bezim v userspace");
    let mut sup = supervisor::SUPERVISOR.lock();
    match system::start_time(&startup) {
        Ok(t) => {
            sup.watch("timed", &t.process, Some(system::TIME_RESTART));
            *system::TIME.lock() = Some(t);
        }
        Err(e) => println!("init: sluzbu casu nelze spustit: {e}"),
    }
    match system::start_devices(&startup) {
        Ok(mut devices) => {
            if let Err(e) = system::start_fs(&startup, &mut devices) {
                println!("init: souborovy system nelze spustit: {e}");
            }
            match input::start(&startup, core::mem::take(&mut devices.inputs)) {
                Ok(i) => devices.input = Some(i),
                Err(e) => println!("init: sluzbu vstupu nelze spustit: {e}"),
            }
            sup.watch("devmgr", &devices.devmgr, None);
            if let Some((p, _)) = &devices.fsd {
                sup.watch("fsd", p, None);
            }
            if let Some(i) = &devices.input {
                sup.watch("inputd", &i.process, None);
            }
            // Sit az po FS: firewall uklada konfiguraci na FS.
            match system::start_net(&startup, &mut devices) {
                Ok(n) => {
                    // Restart zatim neni (jako u fsd): netd drzi kanaly
                    // vsech socketu, fwd stav dotazu.
                    sup.watch("netd", &n.process, None);
                    if let Some((p, _)) = &n.fwd {
                        sup.watch("fwd", p, None);
                    }
                    system::NET.call_once(|| n);
                }
                Err(e) => println!("init: sluzbu site nelze spustit: {e}"),
            }
            *gui::DISPLAY.lock() = devices.displays.pop();
            system::DEVICES.call_once(|| devices);
        }
        Err(e) => println!("init: spravce zarizeni nelze spustit: {e}"),
    }
    // Bez virtio-gpu (ma prednost) displej nad framebufferem z firmwaru.
    let framebuffer = startup.framebuffer.take();
    if gui::DISPLAY.lock().is_none()
        && let Some((vmo, info)) = framebuffer
    {
        match gui::start_fbdisplay(&startup, vmo, info) {
            Ok((p, ch)) => {
                sup.watch("fbdisplay", &p, None);
                *gui::FBDISPLAY.lock() = Some(p);
                *gui::DISPLAY.lock() = Some(ch);
            }
            Err(e) => println!("init: fbdisplay nelze spustit: {e}"),
        }
    }
    if startup.flags & libos::abi::bootstrap::flags::TEST_RUN == 0 {
        // GUI: kompozitor prevezme vstup a obrazovku (testy si ho spusti samy).
        let input = system::devices().and_then(|d| d.input.as_ref());
        gui::start_desktop(&startup, input);
        if let Some(g) = gui::GUI.lock().as_ref() {
            // Restart zatim neni: okna aplikaci patri kompozitoru.
            sup.watch("compd", &g.process, None);
        }
        // Bezny beh: init je supervisor a nekonci.
        sup.run(&startup);
    }
    // Testovaci beh (jadro s qemu-test): testy userspace, pak konec
    // s vysledkem (jadro ukonci QEMU). Testy supervisor ridi samy.
    drop(sup);
    let failed = tests::run_all(&startup);
    if failed == 0 {
        println!("init: vsechny testy userspace prosly");
        0
    } else {
        println!("init: {failed} testu userspace selhalo");
        1
    }
}
