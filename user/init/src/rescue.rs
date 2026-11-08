//! Rezim "vytahni logy" (priznak startu `RESCUE_LOGS`, klavesa L
//! v zavadeci zive flashky): `logrescue` zkopiruje logy a zaznamy padu
//! nainstalovaneho systemu (disky s nasim FS mimo flashku, jen pro cteni)
//! na oddil MO-LOGY flashky. Init pak nespousti GUI ani sit: na obrazovce
//! zustava textova konzole jadra s vysledkem.

use alloc::vec::Vec;
use core::time::Duration;

use libos::initfs::Initfs;
use libos::spawn::StartHandle;
use libos::{Result, Startup, println, time};

use crate::system::Devices;

/// Probehne kopie; vypise vysledek (velky ramecek pro obrazovku).
pub fn run(startup: &Startup, devices: &mut Devices) {
    let r = copy(startup, devices);
    println!("init: ==========================================================");
    match r {
        Ok(text) if text.starts_with("ok ") => {
            println!("init: HOTOVO: logy a zaznamy padu zkopirovany na flashku ({} souboru)", &text[3..]);
            println!("init: ve Windows: oddil MO-LOGY, slozka ZACHRANA (SEZNAM.TXT)");
            println!("init: pocitac muzete vypnout");
        }
        Ok(text) => println!("init: CHYBA: {}", text.strip_prefix("chyba ").unwrap_or(&text)),
        Err(e) => println!("init: CHYBA: kopie logu se nepovedla: {e}"),
    }
    println!("init: ==========================================================");
}

fn copy(startup: &Startup, devices: &mut Devices) -> Result<alloc::string::String> {
    let Some(logy) = devices.logy.lock().take() else {
        return Ok(alloc::string::String::from("chyba na flashce chybi oddil MO-LOGY (obraz z cargo xtask usb)"));
    };
    let mut handles = Vec::from([StartHandle::user(logy.into_handle())]);
    // Zbyle disky s nasim FS (FS flashky uz vzal fsd).
    for b in devices.blocks.drain(..) {
        if crate::system::has_fs(&b) {
            handles.push(StartHandle::user(b.into_channel().into_handle()));
        }
    }
    println!("init: vytahuji logy z {} disku (jen pro cteni)", handles.len() - 1);
    let fs = Initfs::open(startup)?;
    let image = fs.find("logrescue").ok_or(libos::Error::NotFound)?;
    let (_process, control) = libos::spawn("logrescue", image, handles)?;
    let msg = control.read(Some(time::deadline_after(Duration::from_secs(600))))?;
    Ok(alloc::string::String::from_utf8_lossy(&msg.data).into_owned())
}
