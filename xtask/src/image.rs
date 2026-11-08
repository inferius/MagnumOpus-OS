//! Vytvoreni bootovatelneho obrazu disku: GPT s jednim EFI System oddilem (FAT32).
//!
//! GPT zapise `diskguard::gpt` (format je jednoduchy a dobre popsany ve
//! specifikaci UEFI, kap. 5), FAT32 obsah vytvori crate `fatfs`.

use std::fs;
use std::io::{Cursor, Write};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::Result;

const SECTOR: u64 = 512;
/// Celkova velikost obrazu.
const DISK_SIZE: u64 = 128 * 1024 * 1024;
/// Oddil zacina na 1 MiB (zarovnani obvykle pro SSD a flash).
const PARTITION_START_LBA: u64 = 2048;

/// Nahodny GUID verze 4. Kryptograficka kvalita neni potreba, jen jedinecnost
/// mezi obrazy, proto staci cas + adresa na zasobniku pres SplitMix64.
fn random_guid(salt: u64) -> [u8; 16] {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    let local = 0u8;
    let mut state = nanos ^ (&local as *const u8 as u64) ^ salt.wrapping_mul(0x9E37_79B9_7F4A_7C15);
    let mut next = || {
        state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    };
    let mut bytes = [0u8; 16];
    bytes[..8].copy_from_slice(&next().to_le_bytes());
    bytes[8..].copy_from_slice(&next().to_le_bytes());
    bytes[7] = (bytes[7] & 0x0F) | 0x40; // verze 4 (horni nibble slozky c, LE)
    bytes[8] = (bytes[8] & 0x3F) | 0x80; // varianta RFC 4122
    bytes
}

/// Vytvori obraz disku na `path` a nakopiruje do ESP zadane soubory
/// (`(cesta na ESP, zdrojovy soubor)`).
pub fn create_disk_image(path: &Path, files: &[(&str, &Path)]) -> Result<()> {
    let part_end_lba = diskguard::gpt::last_usable(DISK_SIZE / SECTOR, SECTOR as usize);

    let mut disk = vec![0u8; DISK_SIZE as usize];

    // --- ESP (FAT32) ---
    let part_range =
        (PARTITION_START_LBA * SECTOR) as usize..((part_end_lba + 1) * SECTOR) as usize;
    write_fat_partition(&mut disk[part_range], files, *b"OS ESP     ")?;

    // --- Ochranny MBR, primarni a zalozni GPT (stejny kod, jakym GPT cte
    // ochrana disku v ovladacich) ---
    diskguard::gpt::write(
        &mut disk,
        SECTOR as usize,
        random_guid(2),
        &[diskguard::gpt::NewPartition {
            type_guid: diskguard::ESP_TYPE,
            guid: random_guid(1),
            first: PARTITION_START_LBA,
            last: part_end_lba,
            attrs: 0,
            name: "EFI System",
        }],
    )?;

    fs::create_dir_all(path.parent().unwrap())?;
    fs::write(path, &disk)?;
    Ok(())
}

/// Naformatuje `partition` na FAT32 se jmenovkou `label` a nakopiruje do
/// ni soubory (`(cesta v oddilu, zdrojovy soubor)`).
pub fn write_fat_partition(partition: &mut [u8], files: &[(&str, &Path)], label: [u8; 11]) -> Result<()> {
    let options = fatfs::FormatVolumeOptions::new()
        .fat_type(fatfs::FatType::Fat32)
        .volume_label(label);
    fatfs::format_volume(Cursor::new(&mut *partition), options)?;

    let fs = fatfs::FileSystem::new(Cursor::new(&mut *partition), fatfs::FsOptions::new())?;
    let root = fs.root_dir();
    for (dest, src) in files {
        let data = fs::read(src).map_err(|e| format!("nelze cist {}: {e}", src.display()))?;
        let (dir_path, file_name) = dest.rsplit_once('/').unwrap_or(("", dest));
        let mut dir = root.clone();
        for component in dir_path.split('/').filter(|c| !c.is_empty()) {
            dir = dir.create_dir(component)?;
        }
        let mut file = dir.create_file(file_name)?;
        file.truncate()?;
        file.write_all(&data)?;
    }
    drop(root);
    fs.unmount()?;
    Ok(())
}
