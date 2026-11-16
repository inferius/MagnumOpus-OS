//! Hlidani startu firmwaru: pozna, ze po startu QEMU nebo po restartu
//! uvnitr QEMU (test zaznamu padu) nenabehl nas loader.
//!
//! Videno na aarch64 (edk2-stable202408, TCG): po panice a restartu
//! firmware vypise uvod, smaze obrazovku a v BDS (pripojovani zarizeni
//! pred `Boot0001`) se zasekne; nas loader se nespusti. Testy pak cekaji
//! do celkoveho limitu a vysledek vypada jako selhani testu. Hlidac to
//! rozlisi: kazdy start (zacatek behu, kazda `KERNEL PANIC` v testu padu)
//! musi do [`BOOT_GRACE`] vypsat radek loaderu, jinak je to "firmware/QEMU
//! nenabehl", ne chyba testu.

use std::path::Path;
use std::time::{Duration, Instant};

/// Radek loaderu na seriove lince (`loader/src`), jeden za start.
pub const LOADER_LINE: &str = "loader: os loader";
/// Znacka restartu uvnitr QEMU (test zaznamu padu restartuje po panice).
pub const RESET_LINE: &str = "KERNEL PANIC";
/// Kolik muze trvat firmware od startu/restartu k loaderu. Bezne pod TCG
/// 5-30 s (aarch64 nejdele), s rezervou na zatizeny hostitel.
pub const BOOT_GRACE: Duration = Duration::from_secs(150);
/// Jak casto cist log (log hlavniho behu ma MB, cte se jen do prvniho
/// startu, u testu padu je maly).
const POLL: Duration = Duration::from_secs(1);

/// Kolik startu log ocekava a kolik jich loader potvrdil.
pub fn starts(log: &str, resets: bool) -> (usize, usize) {
    let expected = 1 + if resets { log.matches(RESET_LINE).count() } else { 0 };
    (expected, log.matches(LOADER_LINE).count())
}

/// Zaseknuti firmwaru v tomto behu xtask (hlaseni a cesta k ulozenemu
/// logu); vypisou se v zaverecnem souhrnu (`summary`).
static STUCK: std::sync::Mutex<Vec<(String, std::path::PathBuf)>> = std::sync::Mutex::new(Vec::new());

/// Zapise zaseknuti a ulozi log zaseknuteho behu vedle (`<log>.firmware-
/// zaseknuti-<n>.log`, nemaze se); vrati cestu k nemu.
pub fn record(serial_log: &Path, message: &str) -> std::path::PathBuf {
    let mut stuck = STUCK.lock().unwrap_or_else(|e| e.into_inner());
    let kept = serial_log.with_extension(format!("firmware-zaseknuti-{}.log", stuck.len() + 1));
    if std::fs::rename(serial_log, &kept).is_err() {
        let _ = std::fs::copy(serial_log, &kept);
    }
    stuck.push((message.to_string(), kept.clone()));
    kept
}

/// Zaverecny souhrn (konec `main`, i po chybe): prazdny bez zaseknuti.
pub fn summary() -> Vec<String> {
    let stuck = STUCK.lock().unwrap_or_else(|e| e.into_inner());
    if stuck.is_empty() {
        return Vec::new();
    }
    let mut out = vec![format!("xtask: souhrn: !!! FIRMWARE/QEMU NENABEHL {}x (krok opakovan, neni to selhani testu)", stuck.len())];
    out.extend(stuck.iter().map(|(m, p)| format!("xtask: souhrn:   {m}; log {}", p.display())));
    out
}

pub struct Watch {
    /// Sleduje restarty (test padu); jinak jen prvni start.
    resets: bool,
    /// Od kdy se ceka na loader (`None` = vsechny starty potvrzene).
    waiting: Option<Instant>,
    last_poll: Option<Instant>,
    done: bool,
}

impl Watch {
    pub fn new(resets: bool, now: Instant) -> Self {
        Self { resets, waiting: Some(now), last_poll: None, done: false }
    }

    /// Zkontroluje log; `Some(popis)` = start se zasekl ve firmwaru.
    pub fn poll(&mut self, serial_log: &Path, now: Instant) -> Option<String> {
        if self.done || self.last_poll.is_some_and(|t| now.duration_since(t) < POLL) {
            return None;
        }
        self.last_poll = Some(now);
        let text = std::fs::read(serial_log).map(|b| String::from_utf8_lossy(&b).into_owned()).unwrap_or_default();
        self.step(&text, now)
    }

    /// Jadro `poll` nad textem logu (host testy).
    pub fn step(&mut self, text: &str, now: Instant) -> Option<String> {
        let (expected, seen) = starts(text, self.resets);
        if seen >= expected {
            self.waiting = None;
            // Bez restartu uz neni co hlidat (neprocitat velky log dal).
            self.done = !self.resets;
            return None;
        }
        let since = *self.waiting.get_or_insert(now);
        if now.duration_since(since) <= BOOT_GRACE {
            return None;
        }
        let what = if expected == 1 { "po startu QEMU" } else { "po restartu v QEMU" };
        let tail: String = text.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("").chars().take(120).collect();
        Some(format!(
            "firmware/QEMU nenabehl {what}: start {expected} bez radku loaderu do {} s (posledni radek: {tail:?})",
            BOOT_GRACE.as_secs()
        ))
    }
}

#[cfg(test)]
#[path = "bootwatch_tests.rs"]
mod tests;
