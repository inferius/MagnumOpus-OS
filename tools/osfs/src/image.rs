//! Soubor s obrazem svazku: vytvoreni, otevreni a pomocne funkce.
//!
//! Skutecne disky zatim zamerne nepodporujeme: formatovani disku je
//! nevratne (smaze vse, co na nem je) a potrebuje ochrany (odmitnout
//! systemovy disk, potvrzeni cislem disku), ktere jeste nejsou hotove.

use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use fs::{Algo, FileDevice, FormatOptions, Fs, Kind, MountOptions};

use crate::names::{Hidden, to_windows_checked};

/// Vychozi velikost noveho obrazu (256 MiB).
pub const DEFAULT_SIZE: u64 = 256 << 20;

pub type Volume = Fs<FileDevice>;

/// Cas pro `Fs::set_time`: ns od 1. 1. 1970 (UTC).
pub fn now_ns() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0)
}

/// Vypada cesta jako skutecny disk nebo oddil (ne soubor)? Takove cesty
/// zatim odmitame: `\\.\PhysicalDrive1`, `\\?\Volume{...}`, `E:`, `E:\`,
/// `/dev/sdb`.
pub fn looks_like_device(path: &str) -> bool {
    let p = path.replace('/', "\\");
    let lower = p.to_ascii_lowercase();
    if lower.starts_with("\\\\.\\") || lower.starts_with("\\\\?\\") {
        return true;
    }
    if lower.starts_with("\\dev\\") || lower.starts_with("\\.\\") || lower.starts_with("\\??\\") {
        return true;
    }
    // Jmena zarizeni kdekoli v ceste (i zkomolena shellem, napr. git bash
    // udela z `\\.\PhysicalDrive1` jen `\.\PhysicalDrive1`).
    if ["physicaldrive", "harddisk", "volume{", "globalroot"]
        .iter()
        .any(|d| lower.contains(d))
    {
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

/// Otevre obraz a pripoji svazek.
pub fn open(path: &Path, read_only: bool) -> Result<Volume, String> {
    if looks_like_device(&path.to_string_lossy()) {
        return Err(format!(
            "{}: skutecne disky osfs zatim nepodporuje, jen soubory s obrazem",
            path.display()
        ));
    }
    let dev = FileDevice::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut fs = Fs::mount_with(
        dev,
        MountOptions {
            read_only,
            ..MountOptions::default()
        },
    )
    .map_err(|e| format!("pripojeni {}: {e}", path.display()))?;
    fs.set_time(now_ns());
    Ok(fs)
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

fn join(dir: &str, name: &str) -> String {
    if dir == "/" {
        format!("/{name}")
    } else {
        format!("{dir}/{name}")
    }
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
                Kind::Dir => stack.push(path),
                Kind::File => {}
            }
        }
    }
    out.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(out)
}
