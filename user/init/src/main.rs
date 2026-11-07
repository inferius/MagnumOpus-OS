//! init: prvni proces v userspace.
//!
//! Spusti sluzbu casu, spravce zarizeni (ten spusti ovladace), sluzbu
//! souboroveho systemu, sluzbu vstupu a sluzbu site s firewallem. Pak bezi
//! jako supervisor (`supervisor.rs`: zarizeni pripojena za behu, restart
//! spadlych sluzeb). V testovacim behu (priznak `TEST_RUN` od jadra) misto
//! toho spusti testy userspace (`tests/`) a skonci s jejich vysledkem.
//! Pozdeji bude spoustet systemove sluzby podle konfigurace.

#![no_std]
#![no_main]

extern crate alloc;

mod apps;
mod input;
mod supervisor;
mod system;
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
            system::DEVICES.call_once(|| devices);
        }
        Err(e) => println!("init: spravce zarizeni nelze spustit: {e}"),
    }
    if startup.flags & libos::abi::bootstrap::flags::TEST_RUN == 0 {
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
