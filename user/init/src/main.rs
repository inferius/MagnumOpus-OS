//! init: prvni proces v userspace.
//!
//! Spusti sluzbu casu, spravce zarizeni (ten spusti ovladace), sluzbu
//! souboroveho systemu (s ulozistem korenovych certifikatu), sluzbu vstupu,
//! sluzbu site s firewallem a synchronizaci casu a GUI (kompozitor
//! a ukazkove aplikace, `gui.rs`). Pak bezi
//! jako supervisor (`supervisor.rs`: zarizeni pripojena za behu, restart
//! spadlych sluzeb). V testovacim behu (priznak `TEST_RUN` od jadra) misto
//! toho spusti testy userspace (`tests/`) a skonci s jejich vysledkem.
//! Pozdeji bude spoustet systemove sluzby podle konfigurace.

#![no_std]
#![no_main]

extern crate alloc;

mod apps;
mod certs;
mod dns;
mod gui;
mod input;
mod ntp;
mod revocation;
mod supervisor;
mod system;
mod tasks;
mod terminal;
mod tests;

use libos::{Startup, println};

libos::entry!(main);

fn main(startup: Startup) -> i64 {
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
            } else if let Err(e) = certs::seed(&startup, &devices) {
                // Bez uloziste se spojeni TLS nenavazou (zadna duveryhodna CA).
                println!("init: uloziste certifikatu nelze pripravit: {e}");
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
                Ok(mut n) => {
                    let dnsd_end = n.dnsd_end.take();
                    // Restart zatim neni (jako u fsd): netd drzi kanaly
                    // vsech socketu, fwd stav dotazu.
                    sup.watch("netd", &n.process, None);
                    if let Some((p, _)) = &n.fwd {
                        sup.watch("fwd", p, None);
                    }
                    let n = system::NET.call_once(|| n);
                    // dnsd hned po netd: systemovy preklad (i pro ntpd a revocd).
                    if let Some(end) = dnsd_end {
                        match dns::start(&startup, n, end, &devices) {
                            Ok(d) => {
                                sup.watch("dnsd", &d.process, None);
                                system::DNSD.call_once(|| d);
                            }
                            Err(e) => println!("init: sifrovane DNS nelze spustit (preklad podle profilu site): {e}"),
                        }
                    }
                    // revocd pred ntpd: ntpd dostane kanal pro TLS NTS-KE.
                    match revocation::start(&startup, n, &devices) {
                        Ok(r) => {
                            sup.watch("revocd", &r.process, None);
                            system::REVOCD.call_once(|| r);
                        }
                        Err(e) => println!("init: kontrolu odvolani certifikatu nelze spustit: {e}"),
                    }
                    match ntp::start(&startup, n, &devices) {
                        Ok(t) => {
                            sup.watch("ntpd", &t.process, None);
                            system::NTP.call_once(|| t);
                        }
                        Err(e) => println!("init: synchronizaci casu nelze spustit: {e}"),
                    }
                }
                Err(e) => println!("init: sluzbu site nelze spustit: {e}"),
            }
            *gui::DISPLAY.lock() = devices.displays.pop();
            system::DEVICES.call_once(|| devices);
        }
        Err(e) => println!("init: spravce zarizeni nelze spustit: {e}"),
    }
    // Prehled uloh pred GUI a terminalem (okna a relace se v nem registruji).
    match tasks::start(&startup) {
        Ok(()) => {
            if let Some(t) = tasks::TASKS.lock().as_ref() {
                sup.watch("taskd", &t.process, None);
            }
            // fsd hlasi scrub a udrzbu v necinnosti.
            if let Some((_, ctl)) = system::devices().and_then(|d| d.fsd.as_ref())
                && let Err(e) = tasks::give_producer(ctl, "fsd")
            {
                println!("init: prehled uloh pro fsd: {e}");
            }
        }
        Err(e) => println!("init: sluzbu uloh (taskd) nelze spustit: {e}"),
    }
    if startup.flags & libos::abi::bootstrap::flags::TEST_RUN == 0 {
        // GUI: kompozitor prevezme vstup a obrazovku (testy si ho spusti samy).
        let input = system::devices().and_then(|d| d.input.as_ref());
        gui::start_desktop(&startup, input);
        if let Some(g) = gui::GUI.lock().as_ref() {
            // Restart zatim neni: okna aplikaci patri kompozitoru. Po padu
            // supervisor spusti zalozni konzoli.
            sup.watch("compd", &g.process, None);
        } else if let Err(e) = terminal::TERMINALS.lock().start_console(&startup) {
            // Bez GUI: terminal v zalozni konzoli nad framebufferem.
            println!("init: zalozni konzoli nelze spustit: {e}");
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
