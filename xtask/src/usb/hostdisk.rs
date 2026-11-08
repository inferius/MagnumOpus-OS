//! Disky hostitele pro zapis obrazu USB: vypis, kontrola bezpecnosti
//! a zapis.
//!
//! Zapis prepise cely disk, proto pravidla ([`check`]):
//! - disk musi byt vyjimatelny (sbernice USB, SD/MMC; na Linuxu i priznak
//!   `removable`),
//! - nesmi to byt systemovy ani zavadeci disk hostitele (Windows
//!   `IsSystem`/`IsBoot`, Linux pripojeny `/`, `/boot`, `/boot/efi`, `/usr`,
//!   `/home` nebo swap),
//! - musi byt aspon tak velky jako obraz,
//! - uzivatel musi zapis potvrdit opsanim textu `ANO <disk>` (zobrazi se
//!   model, velikost a sbernice).
//!
//! Zapis nejdriv vymaze posledni MiB disku (stara zalozni GPT by po
//! zapisu mensiho obrazu zustala platna a zmatla firmware i Windows),
//! pak zapise obraz po 1 MiB a uplne prvni MiB (MBR a primarni GPT) az
//! nakonec: preruseny zapis nenecha disk, ktery by vypadal jako platny
//! system.

use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

/// Disk hostitele.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostDisk {
    /// Jak ho oslovit v `--write` (Windows cislo disku, Linux `/dev/sdX`).
    pub id: String,
    /// Cesta pro zapis (`\\.\PhysicalDriveN`, `/dev/sdX`).
    pub path: String,
    pub model: String,
    pub size: u64,
    /// Sbernice (`USB`, `SD`, `NVMe`, `SATA`, `usb`, `nvme` ...).
    pub bus: String,
    /// Vyjimatelne medium (Linux `RM`; na Windows podle sbernice).
    pub removable: bool,
    /// Je na nem system hostitele (nebo zavadec, swap).
    pub system: bool,
    /// Pripojene oddily (Linux pripojne body, Windows pismena).
    pub mounted: Vec<String>,
}

/// Duvod odmitnuti zapisu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    NotFound,
    System,
    NotRemovable(String),
    TooSmall { need: u64, have: u64 },
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Refusal::NotFound => write!(f, "disk nenalezen (seznam: cargo xtask usb --list)"),
            Refusal::System => write!(f, "je to systemovy nebo zavadeci disk tohoto pocitace - odmitam"),
            Refusal::NotRemovable(bus) => write!(
                f,
                "disk na sbernici {bus} neni vyjimatelny (USB, SD) - interni disky xtask neprepisuje; pouzij Rufus nebo dd rucne"
            ),
            Refusal::TooSmall { need, have } => write!(
                f,
                "disk je mensi ({} MiB) nez obraz ({} MiB)",
                have / (1024 * 1024),
                need / (1024 * 1024)
            ),
        }
    }
}

/// Sbernice, ktere berem jako vyjimatelne.
fn removable_bus(bus: &str) -> bool {
    matches!(bus.to_ascii_lowercase().as_str(), "usb" | "sd" | "mmc")
}

/// Smi se obraz o `image_size` bajtech zapsat na disk `id`?
pub fn check<'a>(disks: &'a [HostDisk], id: &str, image_size: u64) -> Result<&'a HostDisk, Refusal> {
    let d = disks
        .iter()
        .find(|d| d.id == id || d.path == id)
        .ok_or(Refusal::NotFound)?;
    if d.system {
        return Err(Refusal::System);
    }
    if !removable_bus(&d.bus) && !d.removable {
        return Err(Refusal::NotRemovable(d.bus.clone()));
    }
    if d.size < image_size {
        return Err(Refusal::TooSmall {
            need: image_size,
            have: d.size,
        });
    }
    Ok(d)
}

/// Text, ktery musi uzivatel opsat.
pub fn confirmation(d: &HostDisk) -> String {
    format!("ANO {}", d.id)
}

/// Popis disku pro vypis a potvrzeni.
pub fn describe(d: &HostDisk) -> String {
    let gib = d.size as f64 / (1024.0 * 1024.0 * 1024.0);
    let mut s = format!("{:<10} {:<34} {:>8.1} GiB  {:<5}", d.id, d.model, gib, d.bus);
    if d.system {
        s.push_str("  SYSTEMOVY");
    } else if removable_bus(&d.bus) || d.removable {
        s.push_str("  vyjimatelny");
    }
    if !d.mounted.is_empty() {
        s.push_str(&format!("  pripojeno: {}", d.mounted.join(", ")));
    }
    s
}

/// Zapise obraz `image` do `target` (otevreny disk o velikosti
/// `disk_size`, nebo soubor) po 1 MiB, prvni MiB nakonec; predtim vymaze
/// posledni MiB disku. `progress(hotovo, celkem)`.
pub fn write_image<T: Write + Seek>(
    image: &Path,
    target: &mut T,
    disk_size: u64,
    mut progress: impl FnMut(u64, u64),
) -> crate::Result<()> {
    const CHUNK: u64 = 1024 * 1024;
    let mut src = std::fs::File::open(image)?;
    let size = src.metadata()?.len();
    if !size.is_multiple_of(CHUNK) {
        return Err("obraz nema delku v celych MiB".into());
    }
    if disk_size < size {
        return Err("disk je mensi nez obraz".into());
    }
    let chunks = size / CHUNK;
    let mut buf = vec![0u8; CHUNK as usize];
    // Konec disku (stara zalozni GPT); disk ma delku v celych sektorech.
    let tail = disk_size.saturating_sub(CHUNK).max(size) & !511;
    if tail < disk_size {
        target.seek(SeekFrom::Start(tail))?;
        target.write_all(&buf[..(disk_size - tail) as usize])?;
    }
    let mut done = 0;
    // Bloky 1..N, blok 0 (MBR a GPT) posledni.
    for i in (1..chunks).chain(std::iter::once(0)) {
        let at = i * CHUNK;
        src.seek(SeekFrom::Start(at))?;
        src.read_exact(&mut buf)?;
        target.seek(SeekFrom::Start(at))?;
        target.write_all(&buf)?;
        done += CHUNK;
        progress(done, size);
    }
    target.flush()?;
    Ok(())
}
