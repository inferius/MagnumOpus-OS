//! Disky vymennych medii pro testy (docs/cs/media.md): flash disky USB
//! (`usb-storage`, `removable=on`) s FAT32 a NTFS pri startu, superfloppy
//! FAT pro pripojeni za behu (QMP `device_add`), a kontrola na hostiteli
//! po testu: co test zapsal na FAT, musi byt v obrazu.
//!
//! Obsah musi sedet s testem `user/init/src/tests/removable.rs`.

use std::io::{Cursor, Read, Write};
use std::path::{Path, PathBuf};

use crate::Result;

const MIB: usize = 1024 * 1024;
/// Prvni sektor oddilu na disku s MBR.
const PART_LBA: usize = 2048;

/// Jmenovky a obsah (stejne konstanty v testu initu).
pub const FAT_LABEL: &str = "MOTEST";
pub const HOT_LABEL: &str = "HOTPLUG";
/// Soubor s dlouhym ceskym jmenem pripraveny na hostiteli.
pub const FAT_FILE: &str = "Přílišžluťoučký kůň.txt";
pub const FAT_TEXT: &str = "ahoj z hostitele\n";
/// Soubor, ktery zapise test (overi se tady po behu).
pub const GUEST_FILE: &str = "Zapsáno v systému/z-testu.txt";
pub const GUEST_TEXT: &str = "zapis z MagnumOpus\n";
/// Znacka v logu: test soubor zapsal a medium bezpecne odpojil.
pub const GUEST_MARK: &str = "media: zapsano a odpojeno";

fn label11(s: &str) -> [u8; 11] {
    let mut l = [b' '; 11];
    for (d, c) in l.iter_mut().zip(s.bytes()) {
        *d = c;
    }
    l
}

/// MBR s jednim oddilem typu `t` od sektoru 2048 do konce disku.
fn mbr(disk: &mut [u8], t: u8) {
    let sectors = (disk.len() / 512) as u32;
    let e = 446;
    disk[e + 4] = t;
    disk[e + 8..e + 12].copy_from_slice(&(PART_LBA as u32).to_le_bytes());
    disk[e + 12..e + 16].copy_from_slice(&(sectors - PART_LBA as u32).to_le_bytes());
    disk[510] = 0x55;
    disk[511] = 0xAA;
}

fn fat_format(part: &mut [u8], label: &str) -> Result<()> {
    let opts = fatfs::FormatVolumeOptions::new().volume_label(label11(label));
    fatfs::format_volume(Cursor::new(part), opts)?;
    Ok(())
}

/// Flash disk s MBR a oddilem FAT32 (0x0C), s pripravenym souborem.
pub fn fat(path: &Path) -> Result<PathBuf> {
    let mut d = vec![0u8; 64 * MIB];
    mbr(&mut d, 0x0C);
    let part = &mut d[PART_LBA * 512..];
    fat_format(part, FAT_LABEL)?;
    {
        let fs = fatfs::FileSystem::new(Cursor::new(&mut *part), fatfs::FsOptions::new())?;
        let mut f = fs.root_dir().create_file(FAT_FILE)?;
        f.write_all(FAT_TEXT.as_bytes())?;
        drop(f);
        fs.root_dir().create_dir("Složka")?.create_file("vnořený.txt")?.write_all(b"vnoreny\n")?;
        fs.unmount()?;
    }
    std::fs::write(path, &d)?;
    Ok(path.to_path_buf())
}

/// Flash disk s MBR a oddilem NTFS (0x07) z testovaciho obrazu crate ntfs.
pub fn ntfs(path: &Path) -> Result<PathBuf> {
    let img = std::fs::read(crate::workspace_root().join("third_party/test-images/ntfs/testfs1"))?;
    let mut d = vec![0u8; PART_LBA * 512 + img.len()];
    mbr(&mut d, 0x07);
    d[PART_LBA * 512..].copy_from_slice(&img);
    std::fs::write(path, &d)?;
    Ok(path.to_path_buf())
}

/// Flash disk bez tabulky oddilu (superfloppy) s FAT, pro pripojeni za behu.
pub fn hot(path: &Path) -> Result<PathBuf> {
    let mut d = vec![0u8; 16 * MIB];
    fat_format(&mut d, HOT_LABEL)?;
    {
        let fs = fatfs::FileSystem::new(Cursor::new(&mut d[..]), fatfs::FsOptions::new())?;
        fs.root_dir().create_file("hot.txt")?.write_all(b"za behu\n")?;
        fs.unmount()?;
    }
    std::fs::write(path, &d)?;
    Ok(path.to_path_buf())
}

/// Po behu testu: soubor zapsany systemem je v obrazu FAT (cte ho
/// nezavisle crate fatfs na hostiteli) a FS je cisty.
pub fn verify_fat(path: &Path) -> Result<()> {
    let mut d = std::fs::read(path)?;
    let part = &mut d[PART_LBA * 512..];
    let fs = fatfs::FileSystem::new(Cursor::new(part), fatfs::FsOptions::new())?;
    let mut text = String::new();
    fs.root_dir()
        .open_file(GUEST_FILE)
        .map_err(|e| format!("medium FAT: soubor {GUEST_FILE} zapsany systemem neni v obrazu: {e}"))?
        .read_to_string(&mut text)?;
    if text != GUEST_TEXT {
        return Err(format!("medium FAT: obsah {GUEST_FILE} nesedi: {text:?}").into());
    }
    let mut orig = String::new();
    fs.root_dir().open_file(FAT_FILE)?.read_to_string(&mut orig)?;
    if orig != FAT_TEXT {
        return Err(format!("medium FAT: puvodni soubor zmenen: {orig:?}").into());
    }
    println!("xtask: medium FAT: zapis systemu overen na hostiteli");
    Ok(())
}
