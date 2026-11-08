//! Obrazky diagnostickeho snimku od kompozitoru (zkratka Print Screen,
//! `docs/cs/gui.md`). Kanal dava init (zprava `snapshots` s handlem),
//! kompozitor posila `snimek <id> <delka>` + VMO s BMP.
//!
//! Ulozi `/log/snimek-<boot>-<id>-<revize>.bmp` (nejvys [`KEEP`] souboru,
//! starsi smaze) a na oddil MO-LOGY `SNIMnn.BMP` (8.3, kolo [`FAT_SLOTS`]).
//! Do logu zapise, kam obrazek ulozil, se stejnym id jako blok stavu.

use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use libos::{Error, Vmo, println};

use crate::{Logd, write_file};

/// Kolik obrazku drzet v `/log` (kazdy az nekolik MiB), kdyz
/// `snimek.conf` nerika jinak (`max_souboru`).
const KEEP: usize = 10;
/// Nastaveni snimku (cte logd pri startu a posle ho kompozitoru).
const CONF: &str = "/log/snimek.conf";
const CONF_DEFAULT: &str = "# Diagnosticky snimek (docs/cs/gui.md). Nacte logd pri startu.
# Snimek vznika jen na vyzadani: zkratka, prikaz, nebo zapnuta udalost.
# zkratka = obe | print | ctrl+alt+shift+s | vypnuto
zkratka = obe
# automaticky snimek, kdyz aplikace prestane odpovidat / po padu (0 = vypnuto)
auto_nereaguje = 0
auto_pad = 0
# nejmensi odstup automatickych snimku v sekundach
interval = 60
# nejvic obrazku v /system/log (starsi se smazou)
max_souboru = 10
";
/// Pocet mist pro obrazky na MO-LOGY.
pub const FAT_SLOTS: u64 = 8;
/// Nejvetsi prijaty obrazek (4K obrazovka v BMP je ~25 MiB).
const MAX_LEN: usize = 48 << 20;

/// Revize sestaveni z radku jadra "os kernel ... sestaveni <commit> ...".
pub fn build_from_line(line: &[u8]) -> Option<String> {
    let text = core::str::from_utf8(line).ok()?;
    if !text.contains("os kernel ") {
        return None;
    }
    let rest = &text[text.find("sestaveni ")? + "sestaveni ".len()..];
    let commit: String = rest.split_whitespace().next()?.chars().filter(|c| c.is_ascii_alphanumeric()).collect();
    (!commit.is_empty()).then_some(commit)
}

impl Logd {
    /// Novy kanal od initu: nacte `snimek.conf` (chybi-li, vytvori ho)
    /// a posle nastaveni kompozitoru.
    pub fn set_shots(&mut self, ch: libos::Channel) {
        let text = match self.fs.stat(CONF) {
            Ok(st) => String::from_utf8_lossy(&self.fs.read(CONF, 0, st.size.min(4 << 10) as usize).unwrap_or_default()).into_owned(),
            Err(_) => {
                let _ = write_file(&self.fs, CONF, CONF_DEFAULT.as_bytes());
                String::from(CONF_DEFAULT)
            }
        };
        let mut pairs: Vec<String> = Vec::new();
        for line in text.lines() {
            let line = line.split('#').next().unwrap_or("").trim();
            let Some((k, v)) = line.split_once('=') else { continue };
            let (k, v) = (k.trim(), v.trim());
            if k == "max_souboru" {
                self.shot_keep = v.parse().unwrap_or(KEEP).max(1);
            } else {
                pairs.push(format!("{k}={v}"));
            }
        }
        let _ = ch.write(format!("nastaveni {}", pairs.join(" ")).as_bytes(), Vec::new());
        self.shots = Some(ch);
    }

    /// Pad procesu: kompozitor udela snimek, kdyz ma zapnute `auto_pad`.
    pub fn shots_crash(&self, name: &str) {
        if let Some(ch) = &self.shots {
            let _ = ch.write(format!("pad {name}").as_bytes(), Vec::new());
        }
    }

    /// Vyridi zpravy od kompozitoru (neblokuje).
    pub fn poll_shots(&mut self) {
        loop {
            let Some(ch) = &self.shots else { return };
            let m = match ch.try_read() {
                Ok(m) => m,
                Err(Error::ShouldWait) => return,
                Err(_) => {
                    self.shots = None;
                    return;
                }
            };
            let text = String::from_utf8_lossy(&m.data).into_owned();
            let mut w = text.split_whitespace();
            let (Some("snimek"), Some(id), Some(len)) = (w.next(), w.next(), w.next().and_then(|l| l.parse::<usize>().ok())) else {
                println!("logd: neznama zprava snimku: {text}");
                continue;
            };
            let Some(h) = m.handles.into_iter().next().filter(|_| len <= MAX_LEN) else {
                println!("logd: snimek {id}: chybi obrazek nebo je moc velky ({len} B)");
                continue;
            };
            let mut data = vec![0u8; len];
            if let Err(e) = Vmo::from_handle(h).read(0, &mut data) {
                println!("logd: snimek {id}: obrazek nejde precist ({e})");
                continue;
            }
            self.save_shot(id, &data);
        }
    }

    fn save_shot(&mut self, id: &str, data: &[u8]) {
        // Id je od kompozitoru: jen bezpecne znaky do jmena souboru.
        let id: String = id.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '.').collect();
        let rev = if self.build.is_empty() { "nezname" } else { self.build.as_str() };
        let path = format!("/log/snimek-{:06}-{id}-{rev}.bmp", self.boot);
        let fs = match write_file(&self.fs, &path, data) {
            Ok(()) => path,
            Err(e) => format!("nejde ({e})"),
        };
        self.trim_shots();
        let fat = match self.fat.as_mut().filter(|f| f.active()) {
            Some(f) => {
                let slot = self.shot_slot;
                self.shot_slot = (slot + 1) % FAT_SLOTS;
                f.snapshot(slot, data)
            }
            None => String::from("neni"),
        };
        println!("logd: snimek {id}: obrazek {} B do {fs}, MO-LOGY {fat}", data.len());
    }

    /// Smaze nejstarsi obrazky nad [`KEEP`].
    fn trim_shots(&self) {
        let Ok(list) = self.fs.list("/log") else { return };
        let mut shots: Vec<String> = list.into_iter().map(|e| e.name).filter(|n| n.starts_with("snimek-") && n.ends_with(".bmp")).collect();
        shots.sort();
        while shots.len() > self.shot_keep {
            let name = shots.remove(0);
            let _ = self.fs.remove(&format!("/log/{name}"));
        }
    }
}
