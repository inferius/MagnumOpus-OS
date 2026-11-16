//! Diagnosticky snimek (QMP): xtask po znacce stiskne Print Screen,
//! kompozitor zapise do logu blok stavu se stromy pristupnosti a logd ulozi
//! obrazek do `/system/log/snimek-<boot>-<id>-<revize>.bmp`.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
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

/// Cesta k logu tohoto bootu (nejnovejsi `boot-*.log`).
fn boot_log(fs: &fsproto::FsClient) -> Result<String, String> {
    let name = fs
        .list(crate::logd::LOG_DIR)
        .map_err(|e| fail("vypis /system/log", e))?
        .into_iter()
        .map(|e| e.name)
        .filter(|n| n.starts_with("boot-") && n.ends_with(".log"))
        .max()
        .ok_or("chybi boot-*.log")?;
    Ok(format!("{}/{name}", crate::logd::LOG_DIR))
}

pub fn diag_snapshot(_: &Startup) -> TestResult {
    let fs = fs()?;
    let before = shots(fs)?;
    // Blok se hleda v tom, co logd pripise od ted: cely novy usek (ne jen
    // konec souboru), at je log jakkoli velky nebo rychle roste.
    let log = boot_log(fs)?;
    let mut at = fs.stat(&log).map_err(|e| fail("stat logu", e))?.size;
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
    // Soubor je ve vypisu driv, nez ho logd dopise: pockat na celou
    // hlavicku (soubeh v plne sade).
    let head = loop {
        let h = fs.read(&path, 0, 54).map_err(|e| fail("cteni obrazku", e))?;
        if h.len() == 54 || time::now() > end {
            break h;
        }
        time::sleep(Duration::from_millis(100));
    };
    check!(head.len() == 54 && head.starts_with(b"BM"), "obrazek je BMP");
    // Id snimku: snimek-<boot>-<id>-<revize>.bmp (id muze obsahovat '-').
    let mid = name.trim_start_matches("snimek-").trim_end_matches(".bmp");
    let id = mid.split_once('-').and_then(|(_, r)| r.rsplit_once('-')).map(|(id, _)| id).ok_or("jmeno obrazku")?;
    // Blok stavu v logu bootu (logd ho pripisuje kazdou sekundu): cist
    // prubezne od `at`, drzet jen radky bloku (a poznamky logd o ztrate).
    let tag = format!("snimek {id}:");
    let mut block = String::new();
    let mut notes = String::new();
    let mut partial: Vec<u8> = Vec::new();
    let end = time::deadline_after(Duration::from_secs(30));
    loop {
        let size = fs.stat(&log).map_err(|e| fail("stat logu", e))?.size;
        while at < size {
            let n = ((size - at) as usize).min(256 << 10);
            let data = fs.read(&log, at, n).map_err(|e| fail("cteni logu", e))?;
            if data.is_empty() {
                break;
            }
            at += data.len() as u64;
            partial.extend_from_slice(&data);
            let Some(e) = partial.iter().rposition(|&c| c == b'\n') else { continue };
            let whole: Vec<u8> = partial.drain(..=e).collect();
            for line in String::from_utf8_lossy(&whole).lines() {
                if line.contains(tag.as_str()) {
                    block.push_str(line);
                    block.push('\n');
                } else if line.starts_with("[logd:") && notes.len() < 300 {
                    notes.push_str(line);
                    notes.push(' ');
                }
            }
        }
        let done = block.contains(&format!("{tag} konec"));
        let trace = block.contains(&format!("{tag} trasovani vstupu"));
        if done && trace {
            break;
        }
        if time::now() > end {
            // Duvod do hlaseni: co z bloku doslo a kam az se cetlo.
            return Err(format!(
                "blok '{tag}' v {log} chybi: konec {done}, trasovani {trace}, radku bloku {}, precteno do {at} B z {size} B{}{}",
                block.lines().count(),
                if notes.is_empty() { "" } else { ", logd: " },
                notes.trim_end()
            ));
        }
        time::sleep(Duration::from_millis(300));
    }
    let text = block.as_str();
    check!(text.contains(&format!("{tag}   ")) && text.contains("] vstup: id="), "trasovani vstupu v bloku snimku");
    check!(text.contains(&format!("{tag} okno z=0")), "okna v bloku snimku");
    check!(text.contains(&format!("{tag}   uzel")) && text.contains("checkbox 'Zobrazit dokončené'"), "strom pristupnosti v bloku snimku");
    check!(text.contains(&format!("{tag} fokus okno")), "fokus v bloku snimku");
    check!(text.contains(&format!("{tag} duvod zkratka")), "duvod snimku");
    check!(text.contains("viditelny"), "viditelnost prvku v bloku snimku");
    check!(text.contains("posuvna oblast vrstva"), "posuvna oblast v bloku snimku");
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
