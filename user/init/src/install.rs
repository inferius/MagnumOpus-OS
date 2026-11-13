//! Instalator: start sluzby `installd` na prvni zadost pruvodce (`install`
//! v shellu) a kanaly pro pruvodce; nastaveni systemu z
//! `/system/config/system.toml` pri startu (pasmo, rozlozeni klavesnice).
//!
//! installd dostane raw kanaly vsech disku (zapis do cizich oddilu odmita
//! ovladac, `diskguard::install`). Init je predava jen jemu a jen jednou.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::time::Duration;

use installplan::config::{self, Settings};
use libos::initfs::Initfs;
use libos::spawn::StartHandle;
use libos::{Channel, Error, Process, Result, println, time};

struct Installd {
    _process: Process,
    control: Channel,
}

static INSTALLD: spin::Mutex<Option<Installd>> = spin::Mutex::new(None);
/// Startovni zprava initu (kvuli initfs).
pub static STARTUP_INITFS: spin::Once<Initfs> = spin::Once::new();

/// Kanal k instalatoru (sluzbu spusti, kdyz jeste nebezi).
pub fn open() -> Result<Channel> {
    let mut g = INSTALLD.lock();
    if g.is_none() {
        *g = Some(start()?);
    }
    let svc = g.as_ref().unwrap();
    let (ours, theirs) = Channel::create()?;
    svc.control.write(b"klient", alloc::vec![theirs.into_handle()])?;
    Ok(ours)
}

fn start() -> Result<Installd> {
    let devices = crate::system::devices().ok_or(Error::NotFound)?;
    let raw = core::mem::take(&mut *devices.raw.lock());
    if raw.is_empty() {
        println!("init: instalator: zadny disk s raw kanalem (NVMe, SATA, USB)");
    }
    let initfs = STARTUP_INITFS.get().ok_or(Error::NotFound)?;
    let image = initfs.find("installd").ok_or(Error::NotFound)?;
    let qemu = initfs.find("accounts-qemu").is_some();
    let mut cfg = String::from(if qemu { "qemu-kdf\n" } else { "hw\n" });
    let mut handles = Vec::new();
    for (ch, desc) in raw {
        cfg.push_str(&desc);
        cfg.push('\n');
        handles.push(StartHandle::user(ch.into_handle()));
    }
    let (process, control) = libos::spawn("installd", image, handles)?;
    control.write(cfg.as_bytes(), Vec::new())?;
    let ok = control.read(Some(time::deadline_after(Duration::from_secs(20))))?;
    if ok.data != b"ok" {
        return Err(Error::IoError);
    }
    println!("init: instalator (installd) bezi");
    Ok(Installd { _process: process, control })
}

/// Nastaveni z `/system/config/system.toml` (zapsal instalator): pasmo
/// a hodiny pro sluzbu casu, rozlozeni klavesnice pro sluzbu vstupu.
/// Chybi-li soubor (zive USB, starsi instalace), plati vychozi.
pub fn apply_settings(devices: &crate::system::Devices) {
    let Some(fs) = devices.fs.as_ref() else { return };
    let Ok(st) = fs.stat(config::PATH) else { return };
    let Ok(data) = fs.read(config::PATH, 0, st.size.min(16 << 10) as usize) else { return };
    let s = Settings::parse(&String::from_utf8_lossy(&data));
    if let Some(t) = crate::system::TIME.lock().as_ref() {
        let msg = format!("settings {}", s.time_settings());
        if t.channel.write(msg.as_bytes(), Vec::new()).is_ok() {
            let _ = t.channel.read(Some(time::deadline_after(Duration::from_secs(5))));
        }
    }
    if let Some(i) = devices.input.as_ref()
        && let Err(e) = i.set_layouts(&s.keyboards)
    {
        println!("init: rozlozeni klavesnice {:?}: {e}", s.keyboards);
    }
    println!("init: nastaveni systemu: pasmo {}, klavesnice {}", s.zone, s.keyboards.join(", "));
}
