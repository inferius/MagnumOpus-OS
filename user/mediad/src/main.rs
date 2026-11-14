//! mediad: sluzba medii (`docs/cs/multimedia.md`).
//!
//! Bootstrap kanal = ridici kanal od initu (`mediaproto`, jen `CONNECT`):
//! po startu posle `ok`, pak na kazde `CONNECT` vrati novy kanal klienta.
//! Kanal klienta = jedna relace ve vlastnim vlakne ([`session`]). Relace
//! spousti proces dekoderu (dnes testovaci `mediatest`) ve vlastnim jobu
//! s limity, bez jakychkoli handlu krome ridiciho kanalu; vstup mu
//! posila jen jako bajty ([`source`]).
//!
//! Handly od initu: initfs (`kind::INITFS`) - obrazy dekoderu: `ffdec`
//! (FFmpeg, program PIE se zavadecem; mediad ma relaci s libd) a testovaci
//! `mediatest` (format `MTM1`). Dekoder se voli podle prvnich bajtu zdroje.

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

/// Obrazy dekoderu z initfs (`ffdec` muze chybet v obrazu bez FFmpeg).
#[derive(Clone, Copy)]
pub struct Images {
    pub test: &'static [u8],
    pub ffmpeg: Option<&'static [u8]>,
}

fn main(startup: Startup) -> i64 {
    let Some(fs) = Initfs::open(&startup).ok() else {
        println!("mediad: chybi initfs");
        return 1;
    };
    let Some(test) = fs.find("mediatest") else {
        println!("mediad: chybi obraz dekoderu mediatest v initfs");
        return 1;
    };
    let images = Images { test, ffmpeg: fs.find("ffdec") };
    if images.ffmpeg.is_none() {
        println!("mediad: obraz ffdec chybi, jen testovaci format MTM1");
    }
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
            Ok(Request::Connect) => match connect(images) {
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
fn connect(images: Images) -> libos::Result<Channel> {
    let (ours, theirs) = Channel::create()?;
    // Vlakno se odpoji: konci samo se zavrenim kanalu klienta.
    drop(libos::thread::spawn("mediad-relace", move || session::run(ours, images))?);
    Ok(theirs)
}
