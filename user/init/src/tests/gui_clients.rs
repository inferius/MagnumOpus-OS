//! Kompozitor s vic nez 64 klienty: smycka compd ceka pres port
//! (`libos::WaitSet`), ne pres `OBJECT_WAIT_MANY` (limit 64 objektu).
//! Testovaci klient je primo init (surovy protokol `guiproto::window`).

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::time::Duration;

use guiproto::control::ConnectInfo;
use guiproto::window::{ClientMsg, ServerMsg};
use libos::{Channel, Startup, time};

use super::gui::with;
use super::{TestResult, fail};

const N: usize = 100;

fn hello(ch: &Channel) -> Result<(), String> {
    ch.write(&ClientMsg::Hello { version: guiproto::VERSION }.encode(), Vec::new())
        .map_err(|e| fail("HELLO", e))
}

/// Ceka na `WELCOME` (do 5 s); ostatni zpravy (motiv, ping...) zahodi.
fn welcome(ch: &Channel, i: usize) -> Result<(), String> {
    let end = time::deadline_after(Duration::from_secs(5));
    loop {
        let m = ch.read(Some(end)).map_err(|e| fail(&format!("WELCOME klienta {i}"), e))?;
        if let Ok(ServerMsg::Welcome { .. }) = ServerMsg::decode(&m.data) {
            return Ok(());
        }
    }
}

pub fn many_clients(_: &Startup) -> TestResult {
    with(|s| {
        let mut chans = Vec::new();
        for i in 0..N {
            let info = ConnectInfo {
                id: format!("test.klient{i}"),
                name: format!("Klient {i}"),
                perms: Vec::new(),
                actions: Vec::new(),
                system: false,
            };
            let ch = s.gui.channel(&info.encode()).map_err(|e| fail(&format!("klient {i}"), e))?;
            chans.push(ch);
        }
        // Pozdravit v obracenem poradi: klienti nad starym limitem prvni.
        for ch in chans.iter().rev() {
            hello(ch)?;
        }
        for (i, ch) in chans.iter().enumerate().rev() {
            welcome(ch, i)?;
        }
        // Zavrit kazdeho druheho; zbyli a novy klient dal funguji.
        let keep: Vec<Channel> = chans.into_iter().enumerate().filter(|(i, _)| i % 2 == 1).map(|(_, c)| c).collect();
        let info = ConnectInfo { id: "test.klient-po".into(), name: "Po".into(), perms: Vec::new(), actions: Vec::new(), system: false };
        let late = s.gui.channel(&info.encode()).map_err(|e| fail("klient po zavreni", e))?;
        hello(&late)?;
        welcome(&late, N)?;
        // Opakovany HELLO = dalsi WELCOME: zbyli klienti jsou dal obsluhovani.
        for ch in &keep {
            hello(ch)?;
        }
        for (i, ch) in keep.iter().enumerate() {
            welcome(ch, 2 * i + 1)?;
        }
        Ok(())
    })
}
