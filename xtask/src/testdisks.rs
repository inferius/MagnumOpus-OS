//! Testovaci disky pro ovladace a ochranu cizich disku (GPT pres
//! `diskguard::gpt`, stejny kod, jakym je cte ovladac).
//!
//! | disk            | radic        | obsah                                       | klient vidi          |
//! |-----------------|--------------|---------------------------------------------|----------------------|
//! | scratch         | virtio-blk   | GPT, nas oddil 2048 sektoru                 | oddil (1 MiB)        |
//! | nvme-test       | NVMe         | GPT, nas oddil 4096 sektoru se znackou      | oddil (2 MiB)        |
//! | ahci-test       | AHCI port 0  | GPT, nas oddil 6144 sektoru se znackou      | oddil (3 MiB)        |
//! | ahci-foreign    | AHCI port 1  | GPT s oddilem Linux (cizi data se znackou)  | cely disk, jen cteni |
//!
//! Velikosti a znacky musi sedet s `user/init/src/tests/disks.rs`
//! a `drivers.rs`. Datovy disk na NVMe nebo AHCI (`--data-disk`) dostane
//! GPT s nasim oddilem kolem souboroveho systemu ([`wrap_data`]).

use std::path::{Path, PathBuf};

use diskguard::gpt::{self, NewPartition};
use diskguard::{Guid, OUR_TYPE, SECTOR, guid};

use crate::Result;

/// Oddily zacinaji na 1 MiB (obvykle zarovnani).
const PART_START: u64 = 2048;

/// Typ "Linux filesystem" (cizi oddil).
const LINUX_TYPE: Guid = guid(
    0x0FC6_3DAF,
    0x8483,
    0x4772,
    [0x8E, 0x79, 0x3D, 0x69, 0xD8, 0x47, 0x7D, 0xE4],
);

pub const NVME_MARK: &[u8] = b"MagnumOpus test NVMe";
pub const AHCI_MARK: &[u8] = b"MagnumOpus test AHCI";
pub const FOREIGN_MARK: &[u8] = b"cizi data - nezapisovat";

/// Disk o `sectors` sektorech s jednim oddilem od `PART_START`
/// o `part_sectors` sektorech (0 = do konce disku).
fn gpt_disk(sectors: u64, type_guid: Guid, part_sectors: u64, name: &str, seed: u8) -> Result<Vec<u8>> {
    let mut d = vec![0u8; (sectors as usize) * SECTOR];
    let last = if part_sectors == 0 {
        gpt::last_usable(sectors)
    } else {
        PART_START + part_sectors - 1
    };
    gpt::write(
        &mut d,
        [seed; 16],
        &[NewPartition {
            type_guid,
            guid: [seed.wrapping_add(1); 16],
            first: PART_START,
            last,
            attrs: 0,
            name,
        }],
    )?;
    Ok(d)
}

fn put(path: &Path, data: &[u8]) -> Result<PathBuf> {
    std::fs::write(path, data)?;
    Ok(path.to_path_buf())
}

/// Disk virtio-blk pro testy ovladace: nas oddil presne 1 MiB.
pub fn scratch(path: &Path) -> Result<PathBuf> {
    put(path, &gpt_disk(6144, OUR_TYPE, 2048, "MagnumOpus test", 1)?)
}

/// Testovaci disk NVMe: nas oddil 2 MiB, v prvnim sektoru znacka.
pub fn nvme(path: &Path) -> Result<PathBuf> {
    let mut d = gpt_disk(8192, OUR_TYPE, 4096, "MagnumOpus test", 2)?;
    let at = PART_START as usize * SECTOR;
    d[at..at + NVME_MARK.len()].copy_from_slice(NVME_MARK);
    put(path, &d)
}

/// Testovaci disk AHCI: nas oddil 3 MiB, v prvnim sektoru znacka.
pub fn ahci(path: &Path) -> Result<PathBuf> {
    let mut d = gpt_disk(10240, OUR_TYPE, 6144, "MagnumOpus test", 3)?;
    let at = PART_START as usize * SECTOR;
    d[at..at + AHCI_MARK.len()].copy_from_slice(AHCI_MARK);
    put(path, &d)
}

/// Cizi disk (GPT s oddilem Linux): ovladac ho musi nabidnout jen pro cteni.
pub fn foreign(path: &Path) -> Result<PathBuf> {
    let mut d = gpt_disk(16384, LINUX_TYPE, 0, "linux-root", 4)?;
    let at = PART_START as usize * SECTOR;
    d[at..at + FOREIGN_MARK.len()].copy_from_slice(FOREIGN_MARK);
    put(path, &d)
}

/// Obali obraz souboroveho systemu `raw` do GPT s nasim oddilem (datovy
/// disk na NVMe/AHCI, jako na skutecnem pocitaci). Vysledek je v `out`.
pub fn wrap_data(raw: &Path, out: &Path) -> Result<PathBuf> {
    let fs = std::fs::read(raw)?;
    if !fs.len().is_multiple_of(SECTOR) {
        return Err("obraz FS nema delku v celych sektorech".into());
    }
    let part = (fs.len() / SECTOR) as u64;
    // Za oddilem misto na zalozni tabulku, zaokrouhleno na 1 MiB.
    let sectors = (PART_START + part + gpt::WRITE_TABLE_SECTORS + 1).next_multiple_of(2048);
    let mut d = gpt_disk(sectors, OUR_TYPE, part, diskguard::OUR_LABEL, 5)?;
    let at = PART_START as usize * SECTOR;
    d[at..at + fs.len()].copy_from_slice(&fs);
    std::fs::remove_file(raw)?;
    put(out, &d)
}
