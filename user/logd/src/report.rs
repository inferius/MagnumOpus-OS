//! Balicek pro vyvojare (`report` v shellu): jeden textovy soubor s logem
//! tohoto bootu, koncem logu minuleho bootu a zaznamy padu.
//!
//! Osobni data: balicek obsahuje jen logy systemu a zaznamy padu (jmena
//! procesu, zarizeni, adresy, hlaseni sluzeb). Obsah souboru uzivatele ani
//! hesla do logu nejdou; nahled vypise, co v balicku je.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use libos::Result;

use crate::{Logd, boot_tag, write_file};

/// Kolik zaznamu padu (nejnovejsich) pribalit.
const MAX_CRASHES: usize = 10;
/// Konec logu minuleho bootu.
const PREV_TAIL: u64 = 256 << 10;

/// Sestavi balicek; vraci (cesta, nahled obsahu).
pub fn build(d: &mut Logd) -> Result<(String, String)> {
    let mut out = String::new();
    let mut preview = String::new();
    out.push_str("MagnumOpus: balicek pro vyvojare\n");
    out.push_str(&format!("boot: {}\n", d.boot));
    out.push_str("obsah: zaznamy padu, log tohoto bootu, konec logu minuleho bootu\n");
    out.push_str("adresy v zasobnicich prelozi: cargo xtask symbolize <tento soubor>\n\n");

    // Zaznamy padu (nejnovejsi podle jmena: zacina cislem bootu).
    let mut crashes: Vec<String> = d.fs.list("/pady").map(|l| l.into_iter().map(|e| e.name).collect()).unwrap_or_default();
    crashes.sort();
    let skip = crashes.len().saturating_sub(MAX_CRASHES);
    for name in &crashes[skip..] {
        let data = read_all(d, &format!("/pady/{name}"), u64::MAX)?;
        preview.push_str(&format!("  zaznam padu {name} ({} B)\n", data.len()));
        out.push_str(&format!("===== /system/pady/{name} =====\n"));
        out.push_str(&String::from_utf8_lossy(&data));
        out.push('\n');
    }
    if crashes.is_empty() {
        preview.push_str("  zadny zaznam padu\n");
    }

    // Log tohoto bootu (zapsana cast; fronta se pred balickem zapsala).
    let cur = format!("/log/{}", d.log_name());
    let data = read_all(d, &cur, u64::MAX).unwrap_or_default();
    // Diagnostika: zarizeni, ktera jadro po konci ovladace nedokazalo
    // zastavit (jejich pamet pro DMA zustava v karantene).
    let text = String::from_utf8_lossy(&data);
    let stuck: Vec<&str> = text.lines().filter(|l| l.contains("karantena DMA")).collect();
    if !stuck.is_empty() {
        preview.push_str(&format!("  POZOR: karantena DMA ({} hlaseni): zarizeni nejde zastavit, cast pameti zustava zabrana\n", stuck.len()));
        out.push_str("===== karantena DMA (zarizeni, ktera jadro nezastavilo) =====\n");
        for l in &stuck {
            out.push_str(l);
            out.push('\n');
        }
        out.push('\n');
    }
    preview.push_str(&format!("  log tohoto bootu {} ({} B)\n", d.log_name(), data.len()));
    out.push_str(&format!("===== /system{cur} =====\n"));
    out.push_str(&String::from_utf8_lossy(&data));
    out.push('\n');

    // Konec logu minuleho bootu (pred padem nebo restartem).
    if d.boot > 1 {
        let prev = format!("/log/boot-{}.log", boot_tag(d.boot - 1));
        if let Ok(data) = read_all(d, &prev, PREV_TAIL) {
            preview.push_str(&format!("  konec logu minuleho bootu ({} B)\n", data.len()));
            out.push_str(&format!("===== /system{prev} (konec) =====\n"));
            out.push_str(&String::from_utf8_lossy(&data));
            out.push('\n');
        }
    }

    let mut n = 1;
    let path = loop {
        let p = format!("/log/report-{}-{n}.txt", boot_tag(d.boot));
        if d.fs.stat(&p).is_err() {
            break p;
        }
        n += 1;
    };
    write_file(&d.fs, &path, out.as_bytes())?;
    if let Some(f) = d.fat.as_mut() {
        f.report(out.as_bytes());
    }
    d.mirror();
    preview.push_str(&crate::fatlog::note(&d.fat));
    let shown = format!("/system{path}");
    libos::println!("logd: balicek pro vyvojare {shown} ({} B)", out.len());
    Ok((shown, format!("{preview}  celkem {} B", out.len())))
}

/// Precte soubor (nejvys poslednich `tail` bajtu).
fn read_all(d: &Logd, path: &str, tail: u64) -> Result<Vec<u8>> {
    let size = d.fs.stat(path)?.size;
    let mut at = size.saturating_sub(tail);
    let mut out = Vec::new();
    while at < size {
        let n = ((size - at) as usize).min(32 << 10);
        let chunk = d.fs.read(path, at, n)?;
        if chunk.is_empty() {
            break;
        }
        at += chunk.len() as u64;
        out.extend_from_slice(&chunk);
    }
    Ok(out)
}
