//! Naformatovani oddilu na FAT32 (instalator: nase ESP).
//!
//! Parametry podle specifikace Microsoft FAT (FATGEN 1.03): sektor 512 B,
//! 32 rezervovanych sektoru, 2 kopie FAT, korenovy adresar v clusteru 2,
//! FSInfo v sektoru 1, zalozni boot sektor v sektoru 6. Cluster 4 KiB, kdyz
//! je clusteru dost (FAT32 potrebuje aspon 65 525), jinak mensi.

use alloc::vec;

use crate::{Device, Error, Result, SECTOR};

const RESERVED: u64 = 32;
const FATS: u64 = 2;
/// Nejmene clusteru FAT32 (s rezervou).
const MIN_CLUSTERS: u64 = 65_600;

/// Naformatuje `sectors` sektoru zarizeni (od sektoru 0) na FAT32 se
/// jmenovkou `label` (11 znaku ASCII). `hidden` = prvni sektor oddilu na
/// disku (pole BPB pro firmware), `volume_id` = cislo svazku.
pub fn format<D: Device>(dev: &mut D, sectors: u64, label: &[u8; 11], hidden: u64, volume_id: u32) -> Result<()> {
    if sectors > u32::MAX as u64 {
        return Err(Error::Full);
    }
    let spc = [8u64, 4, 2, 1]
        .into_iter()
        .find(|&s| (sectors - RESERVED) / s >= MIN_CLUSTERS)
        .ok_or(Error::NotFat32)?;
    // Velikost FAT (FATGEN 1.03): (disk - rezerva) / ((256 * spc + FATS) / 2).
    let tmp1 = sectors - RESERVED;
    let tmp2 = (256 * spc + FATS) / 2;
    let fat_size = tmp1.div_ceil(tmp2);
    let data = RESERVED + FATS * fat_size;
    let clusters = (sectors - data) / spc;
    if clusters < 65_525 {
        return Err(Error::NotFat32);
    }

    let mut bs = [0u8; SECTOR];
    bs[0..3].copy_from_slice(&[0xEB, 0x58, 0x90]);
    bs[3..11].copy_from_slice(b"MAGNUMOS");
    bs[11..13].copy_from_slice(&(SECTOR as u16).to_le_bytes());
    bs[13] = spc as u8;
    bs[14..16].copy_from_slice(&(RESERVED as u16).to_le_bytes());
    bs[16] = FATS as u8;
    bs[21] = 0xF8;
    bs[24..26].copy_from_slice(&63u16.to_le_bytes());
    bs[26..28].copy_from_slice(&255u16.to_le_bytes());
    bs[28..32].copy_from_slice(&(hidden.min(u32::MAX as u64) as u32).to_le_bytes());
    bs[32..36].copy_from_slice(&(sectors as u32).to_le_bytes());
    bs[36..40].copy_from_slice(&(fat_size as u32).to_le_bytes());
    bs[44..48].copy_from_slice(&2u32.to_le_bytes());
    bs[48..50].copy_from_slice(&1u16.to_le_bytes());
    bs[50..52].copy_from_slice(&6u16.to_le_bytes());
    bs[64] = 0x80;
    bs[66] = 0x29;
    bs[67..71].copy_from_slice(&volume_id.to_le_bytes());
    bs[71..82].copy_from_slice(label);
    bs[82..90].copy_from_slice(b"FAT32   ");
    bs[510] = 0x55;
    bs[511] = 0xAA;

    let mut fsinfo = [0u8; SECTOR];
    fsinfo[0..4].copy_from_slice(&0x4161_5252u32.to_le_bytes());
    fsinfo[484..488].copy_from_slice(&0x6141_7272u32.to_le_bytes());
    fsinfo[488..492].copy_from_slice(&((clusters - 1) as u32).to_le_bytes());
    fsinfo[492..496].copy_from_slice(&3u32.to_le_bytes());
    fsinfo[508..512].copy_from_slice(&0xAA55_0000u32.to_le_bytes());

    // Rezervovana oblast nulove, pak boot sektor, FSInfo a jejich zalohy.
    let zero = vec![0u8; SECTOR * 32];
    dev.write(0, &zero)?;
    dev.write(0, &bs)?;
    dev.write(1, &fsinfo)?;
    dev.write(6, &bs)?;
    dev.write(7, &fsinfo)?;
    // FAT: vynulovat, prvni tri polozky (medium, konec, koren = konec).
    for f in 0..FATS {
        let start = RESERVED + f * fat_size;
        let mut at = 0;
        while at < fat_size {
            let n = (fat_size - at).min(32);
            dev.write(start + at, &zero[..n as usize * SECTOR])?;
            at += n;
        }
        let mut first = [0u8; SECTOR];
        first[0..4].copy_from_slice(&0x0FFF_FFF8u32.to_le_bytes());
        first[4..8].copy_from_slice(&0x0FFF_FFFFu32.to_le_bytes());
        first[8..12].copy_from_slice(&0x0FFF_FFFFu32.to_le_bytes());
        dev.write(start, &first)?;
    }
    // Korenovy adresar (cluster 2): jmenovka svazku, jinak prazdny.
    let mut root = vec![0u8; spc as usize * SECTOR];
    root[..11].copy_from_slice(label);
    root[11] = 0x08;
    dev.write(data, &root)?;
    dev.flush()
}
