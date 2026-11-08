//! Diagnosticky snimek (QMP): xtask po znacce stiskne Print Screen,
//! kompozitor zapise do logu blok stavu se stromy pristupnosti a logd ulozi
//! obrazek do `/system/log/snimek-<boot>-<id>-<revize>.bmp`.

use alloc::format;
use alloc::string::String;
use core::time::Duration;

use libos::{Startup, println, time};

use super::{TestResult, fail};
use crate::check;

/// Znacka pro xtask (musi sedet s `xtask/src/qmp.rs`, modul `gui`).
pub const QEMU_SNAPSHOT: &str = "init: test gui: diagnosticky snimek ceka na Print Screen";

fn fs() -> Result<&'static fsproto::FsClient, String> {
    crate::system::devices().and_then(|d| d.fs.as_ref()).ok_or_else(|| String::from("FS neni"))
}

/// Jmena obrazku snimku v `/system/log`.
fn shots(fs: &fsproto::FsClient) -> Result<alloc::vec::Vec<String>, String> {
    let list = fs.list(crate::logd::LOG_DIR).map_err(|e| fail("vypis /system/log", e))?;
    Ok(list.into_iter().map(|e| e.name).filter(|n| n.starts_with("snimek-") && n.ends_with(".bmp")).collect())
}

pub fn diag_snapshot(_: &Startup) -> TestResult {
    let fs = fs()?;
    let before = shots(fs)?;
    println!("{QEMU_SNAPSHOT}");
    // Obrazek ulozi logd (kazdou sekundu vyrizuje zpravy).
    let end = time::deadline_after(Duration::from_secs(30));
    let name = loop {
        if let Some(n) = shots(fs)?.into_iter().find(|n| !before.contains(n)) {
            break n;
        }
        if time::now() > end {
            return Err(String::from("obrazek snimku se v /system/log neobjevil"));
        }
        time::sleep(Duration::from_millis(200));
    };
    let path = format!("{}/{name}", crate::logd::LOG_DIR);
    let head = fs.read(&path, 0, 54).map_err(|e| fail("cteni obrazku", e))?;
    check!(head.len() == 54 && head.starts_with(b"BM"), "obrazek je BMP");
    // Id snimku: snimek-<boot>-<id>-<revize>.bmp (id muze obsahovat '-').
    let mid = name.trim_start_matches("snimek-").trim_end_matches(".bmp");
    let id = mid.split_once('-').and_then(|(_, r)| r.rsplit_once('-')).map(|(id, _)| id).ok_or("jmeno obrazku")?;
    // Blok stavu v logu bootu (logd ho pripisuje kazdou sekundu).
    let log = fs
        .list(crate::logd::LOG_DIR)
        .map_err(|e| fail("vypis /system/log", e))?
        .into_iter()
        .map(|e| e.name)
        .filter(|n| n.starts_with("boot-") && n.ends_with(".log"))
        .max()
        .ok_or("chybi boot-*.log")?;
    let log = format!("{}/{log}", crate::logd::LOG_DIR);
    let end = time::deadline_after(Duration::from_secs(15));
    loop {
        let size = fs.stat(&log).map_err(|e| fail("stat logu", e))?.size;
        let from = size.saturating_sub(1 << 20);
        let data = fs.read(&log, from, (size - from) as usize).map_err(|e| fail("cteni logu", e))?;
        let text = String::from_utf8_lossy(&data);
        let tag = format!("snimek {id}:");
        if text.contains(&format!("{tag} konec")) {
            check!(text.contains(&format!("{tag} okno z=0")), "okna v bloku snimku");
            check!(text.contains(&format!("{tag}   uzel")) && text.contains("checkbox 'Jen úlohy agenta'"), "strom pristupnosti v bloku snimku");
            check!(text.contains(&format!("{tag} fokus okno")), "fokus v bloku snimku");
            check!(text.contains(&format!("{tag} duvod zkratka")), "duvod snimku");
            check!(text.contains("viditelny"), "viditelnost prvku v bloku snimku");
            check!(text.contains("posuvna oblast vrstva"), "posuvna oblast v bloku snimku");
            break;
        }
        if time::now() > end {
            return Err(format!("blok '{tag}' v logu chybi"));
        }
        time::sleep(Duration::from_millis(300));
    }
    // Prikaz `snapshot` (jako shell `diag snap` pres init): dalsi obrazek.
    let before = shots(fs)?;
    super::gui::with(|s| s.gui.command(guiproto::control::SNAPSHOT.as_bytes(), alloc::vec::Vec::new()).map_err(|e| fail("prikaz snapshot", e)))?;
    let end = time::deadline_after(Duration::from_secs(30));
    while !shots(fs)?.iter().any(|n| !before.contains(n)) {
        if time::now() > end {
            return Err(String::from("obrazek po prikazu snapshot chybi"));
        }
        time::sleep(Duration::from_millis(200));
    }
    Ok(())
}
