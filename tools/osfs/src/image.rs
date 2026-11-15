//! Soubor s obrazem svazku: vytvoreni, otevreni a pomocne funkce.
//!
//! Skutecne disky zatim zamerne nepodporujeme: formatovani disku je
//! nevratne (smaze vse, co na nem je) a potrebuje ochrany (odmitnout
//! systemovy disk, potvrzeni cislem disku), ktere jeste nejsou hotove.

use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use fs::{Algo, FileDevice, FormatOptions, Fs, Kind, MountOptions};

use crate::lock::{ImageLock, LockMode, try_lock};
use crate::names::{Hidden, to_windows_checked};

/// Vychozi velikost noveho obrazu (256 MiB).
pub const DEFAULT_SIZE: u64 = 256 << 20;

pub type Volume = Fs<FileDevice>;

/// Cas pro `Fs::set_time`: realny cas hostitele (i pred rokem 1970, kdyby
/// mel hostitel spatne nastavene hodiny).
pub fn now() -> fs::Timestamp {
    match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(d) => fs::Timestamp::new(d.as_secs() as i64, d.subsec_nanos() as i64),
        Err(e) => {
            let d = e.duration();
            fs::Timestamp::new(-(d.as_secs() as i64), -(d.subsec_nanos() as i64))
        }
    }
}

/// Rozdil mezi 1. 1. 1601 (FILETIME) a 1. 1. 1970 v sekundach.
const EPOCH_DIFF_SECS: i64 = 11_644_473_600;

/// Cas FS -> FILETIME (100 ns od 1601). Cas pred rokem 1601 -> 0, za
/// rokem ~30828 (konec FILETIME) -> nejvetsi hodnota.
pub fn filetime(t: fs::Timestamp) -> u64 {
    let ticks = (t.secs as i128 + EPOCH_DIFF_SECS as i128) * 10_000_000 + (t.nanos / 100) as i128;
    ticks.clamp(0, i64::MAX as i128) as u64
}

/// FILETIME -> cas FS (presne, FILETIME ma rozliseni 100 ns).
pub fn from_filetime(ft: u64) -> fs::Timestamp {
    let secs = (ft / 10_000_000) as i64 - EPOCH_DIFF_SECS;
    fs::Timestamp {
        secs,
        nanos: (ft % 10_000_000) as u32 * 100,
    }
}

/// Vypada cesta jako skutecny disk nebo oddil (ne soubor)? Takove cesty
/// zatim odmitame: `\\.\PhysicalDrive1`, `\\?\Volume{...}`, `E:`, `E:\`,
/// `/dev/sdb`. Obycejny soubor, ktery jen ma v ceste slovo `harddisk`,
/// projde.
pub fn looks_like_device(path: &str) -> bool {
    let p = path.replace('/', "\\");
    let lower = p.to_ascii_lowercase();
    // Jmenne prostory zarizeni. `\.\` je i to, co z `\\.\` udela git bash.
    if ["\\\\.\\", "\\\\?\\", "\\??\\", "\\.\\", "\\dev\\"]
        .iter()
        .any(|pre| lower.starts_with(pre))
    {
        return true;
    }
    // Slozka cesty, ktera je presne jmenem zarizeni.
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    if lower.split('\\').any(|c| {
        c == "globalroot"
            || c.strip_prefix("physicaldrive").is_some_and(digits)
            || c.strip_prefix("harddiskvolume").is_some_and(digits)
            || c.strip_prefix("harddisk").is_some_and(digits)
            || (c.starts_with("volume{") && c.ends_with('}'))
    }) {
        return true;
    }
    let b = lower.as_bytes();
    // `E:` nebo `E:\` = cela jednotka.
    b.len() >= 2
        && b[0].is_ascii_alphabetic()
        && b[1] == b':'
        && (b.len() == 2 || &lower[2..] == "\\")
}

/// Velikost s volitelnou priponou K, M, G, T (nasobky 1024).
pub fn parse_size(s: &str) -> Result<u64, String> {
    let s = s.trim();
    let (num, mul) = match s.chars().last().map(|c| c.to_ascii_uppercase()) {
        Some('K') => (&s[..s.len() - 1], 1u64 << 10),
        Some('M') => (&s[..s.len() - 1], 1 << 20),
        Some('G') => (&s[..s.len() - 1], 1 << 30),
        Some('T') => (&s[..s.len() - 1], 1 << 40),
        _ => (s, 1),
    };
    num.parse::<u64>()
        .ok()
        .and_then(|n| n.checked_mul(mul))
        .ok_or_else(|| format!("neplatna velikost: {s}"))
}

/// Vytvori novy soubor s obrazem a naformatuje ho. Existujici soubor se
/// neprepise (smazani obrazu je na uzivateli).
pub fn create_image(path: &Path, size: u64, label: &str, algo: Algo) -> Result<(), String> {
    if looks_like_device(&path.to_string_lossy()) {
        return Err(format!(
            "{}: vypada to jako disk nebo oddil. Formatovani skutecneho disku je \
             NEVRATNE a osfs ho zatim nepodporuje - pouzij soubor s obrazem.",
            path.display()
        ));
    }
    if label.len() > 32 {
        return Err("jmenovka svazku muze mit nejvys 32 bajtu UTF-8".into());
    }
    let size = size / fs::BLOCK_SIZE as u64 * fs::BLOCK_SIZE as u64;
    let file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|e| format!("{}: {e} (existujici obraz se neprepisuje)", path.display()))?;
    file.set_len(size)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    drop(file);
    let result = (|| {
        let dev = FileDevice::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let opts = FormatOptions {
            label: label.into(),
            algo,
            time: now(),
            ..FormatOptions::default()
        };
        let mut dev = Fs::format(dev, &opts).map_err(|e| format!("formatovani: {e}"))?;
        fs::BlockDevice::flush(&mut dev).map_err(|e| format!("flush: {e}"))?;
        Ok(())
    })();
    if result.is_err() {
        // Nedodelany obraz nenechavat (vytvorili jsme ho prave ted).
        let _ = std::fs::remove_file(path);
    }
    result
}

/// Otevre obraz a pripoji svazek. Vrati i zamek obrazu, ktery je potreba
/// drzet po celou dobu prace se svazkem (viz `lock`): zapis vylucne,
/// cteni sdilene (kdyz obraz prave nekdo zapisuje, cteni jen upozorni).
pub fn open(path: &Path, read_only: bool) -> Result<(Volume, Option<ImageLock>), String> {
    if looks_like_device(&path.to_string_lossy()) {
        return Err(format!(
            "{}: skutecne disky osfs zatim nepodporuje, jen soubory s obrazem",
            path.display()
        ));
    }
    let mode = if read_only {
        LockMode::Shared
    } else {
        LockMode::Exclusive
    };
    let lock = try_lock(path, mode).map_err(|e| format!("{}: {e}", path.display()))?;
    if lock.is_none() {
        if !read_only {
            return Err(format!(
                "{}: obraz ma otevreny jiny proces osfs (mount, put, ...) - dva \
                 zapisovatele by svazek poskodili",
                path.display()
            ));
        }
        eprintln!(
            "osfs: POZOR - obraz prave zapisuje jiny proces osfs; vysledek muze byt \
             o par sekund stary (stav po poslednim commitu)"
        );
    }
    Ok((mount_image(path, read_only)?, lock))
}

/// Pripoji svazek z obrazu (zamek resi volajici).
fn mount_image(path: &Path, read_only: bool) -> Result<Volume, String> {
    let dev = FileDevice::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut fs = Fs::mount_with(
        dev,
        MountOptions {
            read_only,
            ..MountOptions::default()
        },
    )
    .map_err(|e| format!("pripojeni {}: {e}", path.display()))?;
    fs.set_time(now());
    Ok(fs)
}

/// Otevre obraz jen tehdy, kdyz ho nema otevreny zadny jiny proces osfs
/// (ani pro cteni). Pro prikazy s datasety a verzemi: dokud nejde rizeni
/// pres bezici `osfs mount`, nad pripojenym obrazem bezpecne odmitnou
/// (zapis by svazek poskodil, cteni by videlo stary stav).
pub fn open_exclusive(path: &Path, read_only: bool) -> Result<(Volume, ImageLock), String> {
    if looks_like_device(&path.to_string_lossy()) {
        return Err(format!(
            "{}: skutecne disky osfs zatim nepodporuje, jen soubory s obrazem",
            path.display()
        ));
    }
    let mode = if read_only {
        LockMode::Shared
    } else {
        LockMode::Exclusive
    };
    match try_lock(path, mode).map_err(|e| format!("{}: {e}", path.display()))? {
        Some(l) => Ok((mount_image(path, read_only)?, l)),
        None => Err(format!(
            "{}: obraz je pripojeny nebo otevreny jinym procesem osfs - nejdriv \
             ho odpoj (osfs unmount X:)",
            path.display()
        )),
    }
}

/// Polozka, kterou Windows neuvidi.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HiddenEntry {
    pub path: String,
    pub why: HiddenWhy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HiddenWhy {
    Name(Hidden),
    /// Symbolicke odkazy prvni verze ovladace nezobrazuje.
    Symlink,
}

impl std::fmt::Display for HiddenWhy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HiddenWhy::Name(h) => h.fmt(f),
            HiddenWhy::Symlink => f.write_str("symbolicky odkaz (zatim se nezobrazuje)"),
        }
    }
}

/// Cesta polozky `name` v adresari `dir` (`/` + `a` = `/a`).
pub fn join(dir: &str, name: &str) -> String {
    let dir = dir.trim_end_matches('/');
    format!("{dir}/{name}")
}

/// Projde cely strom a vrati polozky, ktere se ve Windows nezobrazi.
/// Do skrytych adresaru nevstupuje (z Windows nejsou dosazitelne).
pub fn hidden_entries<D: fs::BlockDevice>(fs: &mut Fs<D>) -> Result<Vec<HiddenEntry>, fs::Error> {
    let mut out = Vec::new();
    let mut stack = vec![String::from("/")];
    while let Some(dir) = stack.pop() {
        for e in fs.list(&dir)? {
            let path = join(&dir, &e.name);
            if let Err(h) = to_windows_checked(&e.name) {
                out.push(HiddenEntry {
                    path,
                    why: HiddenWhy::Name(h),
                });
                continue;
            }
            match e.kind {
                Kind::Symlink => out.push(HiddenEntry {
                    path,
                    why: HiddenWhy::Symlink,
                }),
                // Dataset se ve Windows zobrazi jako slozka (cesta do nej
                // prejde na jeho koren), projde se stejne.
                Kind::Dir | Kind::Dataset => stack.push(path),
                Kind::File | Kind::Socket => {}
            }
        }
    }
    out.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(out)
}
