//! Vytvoreni bootovatelneho obrazu disku: GPT s jednim EFI System oddilem (FAT32).
//!
//! GPT se sklada rucne (format je jednoduchy a dobre popsany ve specifikaci
//! UEFI, kap. 5), FAT32 obsah vytvori crate `fatfs`.

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
const GPT_ENTRY_COUNT: u64 = 128;
const GPT_ENTRY_SIZE: u64 = 128;
/// Pocet sektoru tabulky oddilu (128 * 128 B = 16 KiB = 32 sektoru).
const GPT_ENTRIES_SECTORS: u64 = GPT_ENTRY_COUNT * GPT_ENTRY_SIZE / SECTOR;

/// Typovy GUID "EFI System Partition" C12A7328-F81F-11D2-BA4B-00A0C93EC93B.
const ESP_TYPE_GUID: [u8; 16] = guid(
    0xC12A7328,
    0xF81F,
    0x11D2,
    [0xBA, 0x4B, 0x00, 0xA0, 0xC9, 0x3E, 0xC9, 0x3B],
);

/// GUID v binarni podobe GPT: prvni tri slozky little-endian, zbytek jak je.
const fn guid(a: u32, b: u16, c: u16, d: [u8; 8]) -> [u8; 16] {
    let a = a.to_le_bytes();
    let b = b.to_le_bytes();
    let c = c.to_le_bytes();
    [
        a[0], a[1], a[2], a[3], b[0], b[1], c[0], c[1], d[0], d[1], d[2], d[3], d[4], d[5], d[6],
        d[7],
    ]
}

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
    let total_lba = DISK_SIZE / SECTOR;
    let last_lba = total_lba - 1;
    let first_usable = 2 + GPT_ENTRIES_SECTORS;
    let last_usable = last_lba - GPT_ENTRIES_SECTORS - 1;
    let part_end_lba = last_usable;

    let mut disk = vec![0u8; DISK_SIZE as usize];

    // --- ESP (FAT32) ---
    let part_range =
        (PARTITION_START_LBA * SECTOR) as usize..((part_end_lba + 1) * SECTOR) as usize;
    write_fat_partition(&mut disk[part_range], files)?;

    // --- Ochranny MBR (LBA 0) ---
    {
        let mbr = &mut disk[..SECTOR as usize];
        let entry = &mut mbr[446..462];
        entry[0] = 0x00; // neaktivni
        entry[1..4].copy_from_slice(&[0x00, 0x02, 0x00]); // CHS zacatku
        entry[4] = 0xEE; // GPT protective
        entry[5..8].copy_from_slice(&[0xFF, 0xFF, 0xFF]); // CHS konce
        entry[8..12].copy_from_slice(&1u32.to_le_bytes());
        let size = (total_lba - 1).min(u32::MAX as u64) as u32;
        entry[12..16].copy_from_slice(&size.to_le_bytes());
        mbr[510] = 0x55;
        mbr[511] = 0xAA;
    }

    // --- Tabulka oddilu ---
    let mut entries = vec![0u8; (GPT_ENTRY_COUNT * GPT_ENTRY_SIZE) as usize];
    {
        let e = &mut entries[..GPT_ENTRY_SIZE as usize];
        e[0..16].copy_from_slice(&ESP_TYPE_GUID);
        e[16..32].copy_from_slice(&random_guid(1));
        e[32..40].copy_from_slice(&PARTITION_START_LBA.to_le_bytes());
        e[40..48].copy_from_slice(&part_end_lba.to_le_bytes());
        // atributy (48..56) = 0
        // Jmeno oddilu: format GPT predepisuje UTF-16LE.
        for (i, unit) in "EFI System".encode_utf16().enumerate() {
            e[56 + i * 2..58 + i * 2].copy_from_slice(&unit.to_le_bytes());
        }
    }
    let entries_crc = crc32fast::hash(&entries);
    let disk_guid = random_guid(2);

    let header = |my_lba: u64, alternate_lba: u64, entries_lba: u64| -> Vec<u8> {
        let mut h = vec![0u8; SECTOR as usize];
        h[0..8].copy_from_slice(b"EFI PART");
        h[8..12].copy_from_slice(&0x0001_0000u32.to_le_bytes()); // revize 1.0
        h[12..16].copy_from_slice(&92u32.to_le_bytes()); // velikost hlavicky
        // 16..20 CRC hlavicky, doplni se nize
        h[24..32].copy_from_slice(&my_lba.to_le_bytes());
        h[32..40].copy_from_slice(&alternate_lba.to_le_bytes());
        h[40..48].copy_from_slice(&first_usable.to_le_bytes());
        h[48..56].copy_from_slice(&last_usable.to_le_bytes());
        h[56..72].copy_from_slice(&disk_guid);
        h[72..80].copy_from_slice(&entries_lba.to_le_bytes());
        h[80..84].copy_from_slice(&(GPT_ENTRY_COUNT as u32).to_le_bytes());
        h[84..88].copy_from_slice(&(GPT_ENTRY_SIZE as u32).to_le_bytes());
        h[88..92].copy_from_slice(&entries_crc.to_le_bytes());
        let crc = crc32fast::hash(&h[..92]);
        h[16..20].copy_from_slice(&crc.to_le_bytes());
        h
    };

    let backup_entries_lba = last_lba - GPT_ENTRIES_SECTORS;
    let at = |lba: u64| (lba * SECTOR) as usize;

    disk[at(1)..at(2)].copy_from_slice(&header(1, last_lba, 2));
    disk[at(2)..at(2 + GPT_ENTRIES_SECTORS)].copy_from_slice(&entries);
    disk[at(backup_entries_lba)..at(last_lba)].copy_from_slice(&entries);
    disk[at(last_lba)..].copy_from_slice(&header(last_lba, 1, backup_entries_lba));

    fs::create_dir_all(path.parent().unwrap())?;
    fs::write(path, &disk)?;
    Ok(())
}

fn write_fat_partition(partition: &mut [u8], files: &[(&str, &Path)]) -> Result<()> {
    let options = fatfs::FormatVolumeOptions::new()
        .fat_type(fatfs::FatType::Fat32)
        .volume_label(*b"OS ESP     ");
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
