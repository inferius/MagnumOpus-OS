//! Texty hodnot pro rozhrani: velikost, pocty, datum a tri stavy hodnot
//! z FS (`fsproto::avail`): znama, skryta (chybi pravo), nedostupna.

use alloc::format;
use alloc::string::String;

use abi::time::WallTime;
use tz::Zone;

/// Znak pro hodnotu, ktera neni (skryta nebo nedostupna). Duvod je
/// v popisu (pristupnost) a v nahledu.
pub const DASH: &str = "\u{2014}";

/// Stav jedne hodnoty.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Avail {
    Known,
    /// Existuje, ale relace ji nesmi videt (bez prava `MANAGE`).
    Hidden,
    /// FS ji nema (napr. souhrn hranice datasetu).
    Unavailable,
}

/// Stav bitu `bit` (`fsproto::avail::*`) podle masek.
pub fn avail(known: u16, hidden: u16, bit: u16) -> Avail {
    if known & bit != 0 {
        Avail::Known
    } else if hidden & bit != 0 {
        Avail::Hidden
    } else {
        Avail::Unavailable
    }
}

/// Vysvetleni chybejici hodnoty (prazdne pro znamou).
pub fn reason(a: Avail) -> &'static str {
    match a {
        Avail::Known => "",
        Avail::Hidden => "skryto: chybí právo Spravovat",
        Avail::Unavailable => "není k dispozici",
    }
}

/// Velikost v jednotkach SI s desetinnou carkou (`1,2 MB`).
pub fn size(b: u64) -> String {
    const UNITS: [&str; 6] = ["B", "kB", "MB", "GB", "TB", "PB"];
    if b < 1000 {
        return format!("{b} B");
    }
    let mut v = b;
    let mut i = 0;
    // Desetiny bez plovouci carky: hodnota v desetinach jednotky.
    while v >= 1_000_000 && i + 2 < UNITS.len() {
        v /= 1000;
        i += 1;
    }
    let tenths = (v + 50) / 100;
    let (whole, frac) = (tenths / 10, tenths % 10);
    if whole >= 1000 && i + 2 < UNITS.len() {
        return format!("1,0 {}", UNITS[i + 2]);
    }
    if whole >= 100 { format!("{whole} {}", UNITS[i + 1]) } else { format!("{whole},{frac} {}", UNITS[i + 1]) }
}

/// Cesky tvar podle poctu: 1 soubor, 2-4 soubory, 5+ souboru.
pub fn plural(n: u64, one: &str, few: &str, many: &str) -> String {
    let w = match n {
        1 => one,
        2..=4 => few,
        _ => many,
    };
    format!("{n} {w}")
}

/// Obsah slozky ze souhrnu (`tree_files`, `tree_dirs`).
pub fn contents(files: u64, dirs: u64) -> String {
    format!("{}, {}", plural(files, "soubor", "soubory", "souborů"), plural(dirs, "složka", "složky", "složek"))
}

/// Mistni datum a cas: `dnes 14:05`, `vcera 9:12`, `3. 11. 2026 14:05`.
/// Bez pasma UTC s oznacenim.
pub fn date(t: WallTime, now: Option<WallTime>, zone: Option<&Zone>) -> String {
    let local = |w: WallTime| match zone {
        Some(z) => z.to_local(w).civil,
        None => w.to_civil(),
    };
    let c = local(t);
    let utc = if zone.is_none() { " UTC" } else { "" };
    if let Some(n) = now {
        let today = local(n);
        let day = |c: &abi::time::Civil| abi::time::days_from_civil(c.year, c.month, c.day);
        match (day(&today), day(&c)) {
            (Some(a), Some(b)) if a == b => return format!("dnes {}:{:02}{utc}", c.hour, c.minute),
            (Some(a), Some(b)) if a == b + 1 => return format!("včera {}:{:02}{utc}", c.hour, c.minute),
            _ => {}
        }
    }
    format!("{}. {}. {} {}:{:02}{utc}", c.day, c.month, c.year, c.hour, c.minute)
}
