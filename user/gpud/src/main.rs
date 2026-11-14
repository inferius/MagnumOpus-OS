//! gpud: sluzba render (kostra kroku 6a-3). Rozdava render uzly ovladace GPU
//! klientum, misto aby je rozdaval vlastnik displeje.
//!
//! Startovni zprava (kind USER): kanal poskytovatele render uzlu
//! (`renderproto::node`, ovladac ho vyda jednou na `OPEN_PROVIDER`). Init
//! drzi jeho konec a po padu gpud preda nove instanci dalsi duplikat;
//! render uzly uz vydane jdou primo k ovladaci, pad gpud je nepreruse.
//!
//! Bootstrap kanal (od initu) mluvi stejnym protokolem: `OPEN` -> gpud
//! posle `OPEN` poskytovateli a vrati odpoved `OPENED` s kanalem render
//! uzlu (nebo `ERROR`). Po startu posle initu `OPENED` bez handlu (= bezi).
//!
//! Zatim jen kostra: povoli kazdy pozadavek od initu. Rozhodovani podle
//! manifestu aplikace (`[gpu] render = true`), kvoty na klienta a spojeni
//! pres listky pro aplikace prijdou s launchd (`docs/cs/render.md`).

#![no_std]
#![no_main]

extern crate alloc;

use alloc::vec::Vec;
use core::time::Duration;

use libos::{Channel, Error, Handle, Startup, println, time};
use renderproto::node::{Reply, Request};

libos::entry!(main);

/// Jak dlouho cekat na odpoved ovladace.
const PROVIDER_TIMEOUT: Duration = Duration::from_secs(5);

fn main(startup: Startup) -> i64 {
    let Some(provider) = startup.handles.into_iter().next().map(Channel::from_handle) else {
        println!("gpud: chybi kanal poskytovatele render uzlu");
        return 1;
    };
    let init = startup.bootstrap;
    if init.write(&Reply::Opened.encode(), Vec::new()).is_err() {
        return 2;
    }
    // Odpovedi stare instance, ktere poskytovatel poslal po jejim padu,
    // nepatri k nasim pozadavkum: zahodit.
    while provider.try_read().is_ok() {}
    loop {
        let m = match init.read(None) {
            Ok(m) => m,
            Err(Error::PeerClosed) => return 0,
            Err(e) => {
                println!("gpud: cteni od initu: {e}");
                return 3;
            }
        };
        let (reply, handles) = match Request::decode(&m.data) {
            Ok(Request::Open) => open(&provider),
            Err(code) => (Reply::Error { code }, Vec::new()),
        };
        if init.write(&reply.encode(), handles).is_err() {
            return 0;
        }
    }
}

/// Novy render uzel od ovladace.
fn open(provider: &Channel) -> (Reply, Vec<Handle>) {
    let r = provider.write(&Request::Open.encode(), Vec::new()).and_then(|()| {
        let m = provider.read(Some(time::deadline_after(PROVIDER_TIMEOUT)))?;
        Ok((Reply::decode(&m.data)?, m.handles))
    });
    match r {
        Ok((Reply::Opened, h)) if h.len() == 1 => (Reply::Opened, h),
        Ok((Reply::Error { code }, _)) => (Reply::Error { code }, Vec::new()),
        Ok(_) => (Reply::Error { code: Error::Corrupted }, Vec::new()),
        Err(code) => {
            println!("gpud: poskytovatel render uzlu: {code}");
            (Reply::Error { code }, Vec::new())
        }
    }
}
