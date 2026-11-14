//! mediad: sluzba medii (`docs/cs/multimedia.md`).
//!
//! Bootstrap kanal = ridici kanal od initu (`mediaproto`, jen `CONNECT`):
//! po startu posle `ok`, pak na kazde `CONNECT` vrati novy kanal klienta.
//! Kanal klienta = jedna relace ve vlastnim vlakne ([`session`]). Relace
//! spousti proces dekoderu (dnes testovaci `mediatest`) ve vlastnim jobu
//! s limity, bez jakychkoli handlu krome ridiciho kanalu; vstup mu
//! posila jen jako bajty ([`source`]).
//!
//! Handly od initu: initfs (`kind::INITFS`) - obraz dekoderu.

#![no_std]
#![no_main]

extern crate alloc;

mod session;
mod source;

use alloc::vec::Vec;

use libos::initfs::Initfs;
use libos::{Channel, Error, Startup, println};
use mediaproto::{Request, encode_reply};

libos::entry!(main);

/// Jmeno obrazu dekoderu v initfs.
const DECODER: &str = "mediatest";

fn main(startup: Startup) -> i64 {
    let image: &'static [u8] = match Initfs::open(&startup).ok().and_then(|fs| fs.find(DECODER)) {
        Some(i) => i,
        None => {
            println!("mediad: chybi obraz dekoderu {DECODER} v initfs");
            return 1;
        }
    };
    let ctl = startup.bootstrap;
    if ctl.write(b"ok", Vec::new()).is_err() {
        return 1;
    }
    loop {
        let m = match ctl.read(None) {
            Ok(m) => m,
            Err(_) => return 0,
        };
        let (reply, hs) = match Request::decode(&m.data) {
            Ok(Request::Connect) => match connect(image) {
                Ok(ch) => (Ok(Vec::new()), alloc::vec![ch.into_handle()]),
                Err(e) => (Err(e), Vec::new()),
            },
            Ok(_) => (Err(Error::NotSupported), Vec::new()),
            Err(e) => (Err(e), Vec::new()),
        };
        if ctl.write(&encode_reply(reply), hs).is_err() {
            return 0;
        }
    }
}

/// Novy kanal klienta a vlakno jeho relace.
fn connect(image: &'static [u8]) -> libos::Result<Channel> {
    let (ours, theirs) = Channel::create()?;
    // Vlakno se odpoji: konci samo se zavrenim kanalu klienta.
    drop(libos::thread::spawn("mediad-relace", move || session::run(ours, image))?);
    Ok(theirs)
}
