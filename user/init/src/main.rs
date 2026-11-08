//! init: prvni proces v userspace.
//!
//! Spusti sluzbu casu, spravce zarizeni (ten spusti ovladace), sluzbu
//! souboroveho systemu (s ulozistem korenovych certifikatu), sluzbu vstupu,
//! sluzbu site s firewallem a synchronizaci casu a GUI (kompozitor
//! a ukazkove aplikace, `gui.rs`) a zvukovou sluzbu (`audio.rs`). Pak bezi
//! jako supervisor (`supervisor.rs`: zarizeni pripojena za behu, restart
//! spadlych sluzeb). V testovacim behu (priznak `TEST_RUN` od jadra) misto
//! toho spusti testy userspace (`tests/`) a skonci s jejich vysledkem.
//! Pozdeji bude spoustet systemove sluzby podle konfigurace.

#![no_std]
#![no_main]

extern crate alloc;

mod apps;
mod audio;
mod certs;
mod dns;
mod fsd;
mod gui;
mod input;
mod ntp;
mod revocation;
mod supervisor;
mod system;
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
            match system::start_fs(&startup, &mut devices) {
                Ok(Some((process, adapter))) => {
                    // Po padu ho supervisor spusti znovu nad stejnym diskem;
                    // relace initu se obnovi pres listek.
                    sup.register(
                        supervisor::Spec {
                            name: "fsd",
                            deps: &[],
                            service: Some(alloc::boxed::Box::new(adapter)),
                        },
                        Some(&process),
                    );
                    match sup.session("fsd", fsd::Recipe::System.encode()) {
                        Ok((ch, lease)) => devices.fs = Some(fsproto::FsClient::with_lease(ch, lease)),
                        Err(e) => println!("init: relace FS nelze vytvorit: {e}"),
                    }
                    if devices.fs.is_some()
                        && let Err(e) = certs::seed(&startup, &devices)
                    {
                        // Bez uloziste se spojeni TLS nenavazou (zadna duveryhodna CA).
                        println!("init: uloziste certifikatu nelze pripravit: {e}");
                    }
                }
                Ok(None) => {}
                Err(e) => println!("init: souborovy system nelze spustit: {e}"),
            }
            match input::start(&startup, core::mem::take(&mut devices.inputs)) {
                Ok(i) => devices.input = Some(i),
                Err(e) => println!("init: sluzbu vstupu nelze spustit: {e}"),
            }
            sup.watch("devmgr", &devices.devmgr, None);
            // Zvuk: audiod prevezme ovladace zvukovych karet.
            match audio::start(&startup, core::mem::take(&mut devices.audios)) {
                Ok(a) => {
                    sup.watch("audiod", &a.process, None);
                    audio::AUDIO.call_once(|| a);
                }
                Err(e) => println!("init: zvukovou sluzbu nelze spustit: {e}"),
            }
            if let Some(i) = &devices.input {
                sup.watch("inputd", &i.process, None);
            }
            // Sit az po FS: firewall uklada konfiguraci na FS.
            match system::start_net(&startup, &mut devices, &mut sup) {
                Ok(mut n) => {
                    let dnsd_end = n.dnsd_end.take();
                    // Restart zatim neni (jako u fsd): netd drzi kanaly
                    // vsech socketu, fwd stav dotazu.
                    sup.watch("netd", &n.process, None);
                    if let Some((p, _)) = &n.fwd {
                        sup.watch("fwd", p, None);
                    }
                    let n = system::NET.call_once(|| n);
                    // Testy: zalozni neovereny resolver jen na pomocny
                    // server hostitele, nikdy skutecne DNS (nestabilni).
                    if startup.flags & libos::abi::bootstrap::flags::TEST_RUN != 0 {
                        tests::net::local_dns(&startup, &n.root);
                    }
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
    if startup.flags & libos::abi::bootstrap::flags::TEST_RUN == 0 {
        // GUI: kompozitor prevezme vstup a obrazovku (testy si ho spusti samy).
        // Kompozitor obnovuje supervisor (okna se pripoji pres listky).
        let input = system::devices().and_then(|d| d.input.as_ref());
        gui::start_desktop(&startup, input, &mut sup);
        if gui::GUI.lock().is_none()
            && let Err(e) = terminal::TERMINALS.lock().start_console(&startup)
        {
            // Bez GUI: terminal v zalozni konzoli nad framebufferem.
            println!("init: zalozni konzoli nelze spustit: {e}");
        }
        if let Some(p) = gui::FBDISPLAY.lock().as_ref() {
            sup.watch("fbdisplay", p, None);
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
