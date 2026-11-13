//! gui-install: graficky instalator (`design/instalace.html`). Pruvodce
//! a rozhrani jsou v knihovne `installview` (testuje se na hostiteli),
//! zapis dela sluzba `installd`.
//!
//! Startovni zprava (kind USER): 1. kanal okna ke kompozitoru, 2. listek
//! k supervisoru, 3. kanal k installd (razi init, jen na zivem USB),
//! 4. volitelne kanal s jednou zpravou: soubor odpovedi (testovaci beh,
//! `cargo xtask test --only instalace-gui`).
//!
//! Testovaci beh: autopilot vyplni kroky podle odpovedi a posune
//! pruvodce kazdych [`AUTO_EVERY`] (mezitim xtask udela snimek obrazovky
//! podle znacky kroku v logu). Odpovedi installd a kroky autopilota jdou
//! do okna jednim kanalem (vlakno je preposila).

#![no_std]
#![no_main]

extern crate alloc;

use alloc::vec::Vec;
use core::time::Duration;

use guiproto::window::Mode;
use installview::{AUTO_STEP, Installer};
use libos::{Channel, Startup, println};

libos::entry!(main);

/// Odstup kroku autopilota: ladici build pod TCG kresli okno nekolik
/// sekund a snimek kroku vznika tesne pred dalsim krokem. Prvni krok ceka
/// dele (okno se teprve otevira).
const AUTO_EVERY: Duration = Duration::from_secs(8);
const AUTO_FIRST: Duration = Duration::from_secs(12);

fn main(startup: Startup) -> i64 {
    let mut handles = startup.handles.into_iter();
    let Some(link) = ui::client::Link::from_handles(&mut handles) else {
        println!("gui-install: chybi kanal okna");
        return 1;
    };
    let Some(installd) = handles.next().map(Channel::from_handle) else {
        println!("gui-install: chybi kanal k instalatoru (installd bezi jen na zivem USB)");
        return 1;
    };
    let answers = handles.next().map(Channel::from_handle).and_then(|c| c.try_read().ok()).map(|m| m.data);
    let writer = match installd.handle().duplicate(None) {
        Ok(d) => Channel::from_handle(d),
        Err(e) => {
            println!("gui-install: kanal installd: {e}");
            return 3;
        }
    };
    let mut app = Installer::new(move |data| {
        let _ = writer.write(&data, Vec::new());
    });
    app.on_log(|line| println!("{line}"));
    let extra = match answers {
        None => {
            app.mark();
            installd
        }
        Some(text) => match autopilot(&mut app, installd, &text) {
            Ok(c) => c,
            Err(e) => {
                println!("gui-install: autopilot: {e}");
                return 3;
            }
        },
    };
    match ui::client::run_with(link, app, Mode::Standard, 900, 640, Some(extra)) {
        Ok(()) => 0,
        Err(e) => {
            println!("gui-install: {e}");
            2
        }
    }
}

/// Testovaci beh: kanal pro okno, do ktereho vlakna preposilaji odpovedi
/// installd a kroky autopilota.
fn autopilot(app: &mut Installer, installd: Channel, text: &[u8]) -> libos::Result<Channel> {
    let text = core::str::from_utf8(text).map_err(|_| libos::Error::InvalidArgs)?;
    let answers = installplan::answers::parse(text).map_err(|e| {
        println!("gui-install: odpovedi: {e}");
        libos::Error::InvalidArgs
    })?;
    app.autopilot(answers);
    let (window_end, fwd) = Channel::create()?;
    let ticks = Channel::from_handle(fwd.handle().duplicate(None)?);
    libos::thread::spawn("gui-install-fwd", move || {
        while let Ok(m) = installd.read(None) {
            if fwd.write(&m.data, Vec::new()).is_err() {
                return;
            }
        }
    })?;
    libos::thread::spawn("gui-install-auto", move || {
        // Kroky 1-7 (osmy je prubeh instalace).
        for i in 0..installplan::STEPS.len() - 1 {
            libos::time::sleep(if i == 0 { AUTO_FIRST } else { AUTO_EVERY });
            if ticks.write(AUTO_STEP, Vec::new()).is_err() {
                return;
            }
        }
    })?;
    Ok(window_end)
}
