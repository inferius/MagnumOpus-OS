//! fsrtest: klient FS pro test "zabij fsd uprostred prace"
//! (`user/init/src/tests/fs_recovery.rs`, `docs/cs/obnova-sluzeb.md`).
//!
//! Handly startovni zpravy: kanal relace `fsd` a listek. Prikazy na
//! bootstrap (odpoved `ok <text>` nebo `err <JMENO_CHYBY>`):
//!
//! - `write <cesta> <text>` - vytvori (kdyz neni) a zapise od zacatku,
//! - `read <cesta>` - cely obsah (do 4 KiB),
//! - `open <cesta>` - otevre pro cteni a zapis, vrati cislo handlu,
//! - `readh <handle>` - obsah pres handle (do 4 KiB),
//! - `writeh <handle> <text>` - zapis pres handle od zacatku,
//! - `lock <handle>` - vylucny zamek celeho souboru,
//! - `close <handle>`,
//! - `gen` - generace instance, se kterou spojeni naposledy obnovilo relaci.
//!
//! Klient bezi v jinem procesu nez supervisor (ten v testu ridi init).

#![no_std]
#![no_main]

extern crate alloc;

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use fsproto::FsClient;
use fsproto::v2::{access, share};
use libos::{Channel, Error, Startup};

libos::entry!(main);

fn main(startup: Startup) -> i64 {
    let mut h = startup.handles.into_iter();
    let (Some(session), Some(lease)) = (h.next(), h.next()) else {
        return 7;
    };
    let fs = FsClient::with_lease(Channel::from_handle(session), Channel::from_handle(lease));
    let control = startup.bootstrap;
    loop {
        let m = match control.read(None) {
            Ok(m) => m,
            Err(_) => return 0,
        };
        let cmd = String::from_utf8_lossy(&m.data).into_owned();
        let text = match command(&fs, &cmd) {
            Ok(t) => format!("ok {t}"),
            Err(e) => format!("err {}", e.name()),
        };
        if control.write(text.as_bytes(), Vec::new()).is_err() {
            return 0;
        }
    }
}

fn num(s: Option<&str>) -> libos::Result<u64> {
    s.and_then(|x| x.parse().ok()).ok_or(Error::InvalidArgs)
}

fn command(fs: &FsClient, cmd: &str) -> libos::Result<String> {
    let mut w = cmd.splitn(3, ' ');
    match (w.next(), w.next(), w.next()) {
        (Some("write"), Some(path), Some(text)) => {
            match fs.create(path, fsproto::kind::FILE) {
                Ok(()) | Err(Error::AlreadyExists) => {}
                Err(e) => return Err(e),
            }
            fs.write(path, 0, text.as_bytes())?;
            Ok(String::new())
        }
        (Some("read"), Some(path), None) => Ok(String::from_utf8_lossy(&fs.read(path, 0, 4096)?).into_owned()),
        (Some("open"), Some(path), None) => {
            let o = fs.open(path, access::READ | access::WRITE, share::ALL, 0)?;
            Ok(format!("{}", o.handle))
        }
        (Some("readh"), h, None) => Ok(String::from_utf8_lossy(&fs.read_h(num(h)?, 0, 4096)?).into_owned()),
        (Some("writeh"), h, Some(text)) => {
            let n = fs.write_h(num(h)?, 0, text.as_bytes()).map_err(|(_, e)| e)?;
            Ok(format!("{n}"))
        }
        (Some("lock"), h, None) => {
            fs.lock(num(h)?, 0, 0, fsproto::v2::lock::EXCLUSIVE)?;
            Ok(String::new())
        }
        (Some("close"), h, None) => {
            fs.close(num(h)?)?;
            Ok(String::new())
        }
        (Some("gen"), None, None) => Ok(format!("{}", fs.conn().generation())),
        _ => Err(Error::InvalidArgs),
    }
}
