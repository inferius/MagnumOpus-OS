//! Testovaci disky pro ovladace a ochranu cizich disku (GPT pres
//! `diskguard::gpt`, stejny kod, jakym je cte ovladac).
//!
//! | disk            | radic        | sektor | obsah                                      | klient vidi (po 512 B) |
//! |-----------------|--------------|--------|--------------------------------------------|------------------------|
//! | scratch         | virtio-blk   | 512 B  | GPT, nas oddil 1 MiB                       | oddil, 2048            |
//! | nvme-test       | NVMe         | 512 B  | GPT, nas oddil 2 MiB se znackou            | oddil, 4096            |
//! | nvme4k-test     | NVMe         | 4 KiB  | GPT (v sektorech 4 KiB), nas oddil 4 MiB   | oddil, 8192            |
//! | ahci-test       | AHCI port 0  | 512 B  | GPT, nas oddil 3 MiB se znackou            | oddil, 6144            |
//! | ahci-foreign    | AHCI port 1  | 512 B  | GPT s oddilem Linux (cizi data se znackou) | cely disk, jen cteni   |
//!
//! Velikosti a znacky musi sedet s `user/init/src/tests/disks.rs`
//! a `drivers.rs`. Datovy disk na NVMe nebo AHCI (`--data-disk`) dostane
//! GPT s nasim oddilem kolem souboroveho systemu ([`wrap_data`]).
//! QEMU umi sektor 4 KiB jen u NVMe a virtio (IDE/AHCI ho odmitne).

use std::path::{Path, PathBuf};

use diskguard::gpt::{self, NewPartition};
use diskguard::{Guid, OUR_TYPE, guid};

use crate::Result;

/// Oddily zacinaji na 1 MiB (obvykle zarovnani).
const PART_START_BYTES: u64 = 1024 * 1024;

/// Typ "Linux filesystem" (cizi oddil).
const LINUX_TYPE: Guid = guid(
    0x0FC6_3DAF,
    0x8483,
    0x4772,
    [0x8E, 0x79, 0x3D, 0x69, 0xD8, 0x47, 0x7D, 0xE4],
);

pub const NVME_MARK: &[u8] = b"MagnumOpus test NVMe";
pub const NVME4K_MARK: &[u8] = b"MagnumOpus test NVMe 4K";
pub const AHCI_MARK: &[u8] = b"MagnumOpus test AHCI";
pub const FOREIGN_MARK: &[u8] = b"cizi data - nezapisovat";

/// Disk o `bytes` bajtech se sektorem `ss` a jednim oddilem od 1 MiB
/// o `part_bytes` bajtech (0 = do konce disku). Do prvniho sektoru oddilu
/// zapise `mark`.
fn gpt_disk(
    ss: usize,
    bytes: u64,
    type_guid: Guid,
    part_bytes: u64,
    name: &str,
    seed: u8,
    mark: &[u8],
) -> Result<Vec<u8>> {
    let mut d = vec![0u8; bytes as usize];
    let sectors = bytes / ss as u64;
    let first = PART_START_BYTES / ss as u64;
    let last = if part_bytes == 0 {
        gpt::last_usable(sectors, ss)
    } else {
        first + part_bytes / ss as u64 - 1
    };
    gpt::write(
        &mut d,
        ss,
        [seed; 16],
        &[NewPartition {
            type_guid,
            guid: [seed.wrapping_add(1); 16],
            first,
            last,
            attrs: 0,
            name,
        }],
    )?;
    let at = PART_START_BYTES as usize;
    d[at..at + mark.len()].copy_from_slice(mark);
    Ok(d)
}

const MIB: u64 = 1024 * 1024;

fn put(path: &Path, data: &[u8]) -> Result<PathBuf> {
    std::fs::write(path, data)?;
    Ok(path.to_path_buf())
}

/// Disk virtio-blk pro testy ovladace: nas oddil presne 1 MiB.
pub fn scratch(path: &Path) -> Result<PathBuf> {
    put(path, &gpt_disk(512, 3 * MIB, OUR_TYPE, MIB, "MagnumOpus test", 1, &[])?)
}

/// Testovaci disk NVMe: nas oddil 2 MiB, v prvnim sektoru znacka.
pub fn nvme(path: &Path) -> Result<PathBuf> {
    put(path, &gpt_disk(512, 4 * MIB, OUR_TYPE, 2 * MIB, "MagnumOpus test", 2, NVME_MARK)?)
}

/// Testovaci disk NVMe se sektorem 4 KiB (`logical_block_size=4096`):
/// GPT v sektorech 4 KiB, nas oddil 4 MiB.
pub fn nvme4k(path: &Path) -> Result<PathBuf> {
    put(path, &gpt_disk(4096, 8 * MIB, OUR_TYPE, 4 * MIB, "MagnumOpus test", 6, NVME4K_MARK)?)
}

/// Testovaci disk AHCI: nas oddil 3 MiB, v prvnim sektoru znacka.
pub fn ahci(path: &Path) -> Result<PathBuf> {
    put(path, &gpt_disk(512, 5 * MIB, OUR_TYPE, 3 * MIB, "MagnumOpus test", 3, AHCI_MARK)?)
}

/// Cizi disk (GPT s oddilem Linux): ovladac ho musi nabidnout jen pro cteni.
pub fn foreign(path: &Path) -> Result<PathBuf> {
    put(path, &gpt_disk(512, 8 * MIB, LINUX_TYPE, 0, "linux-root", 4, FOREIGN_MARK)?)
}

/// Obali obraz souboroveho systemu `raw` do GPT s nasim oddilem pro disk
/// se sektorem `ss` (datovy disk na NVMe/AHCI, jako na skutecnem
/// pocitaci). Vysledek je v `out`.
pub fn wrap_data(raw: &Path, out: &Path, ss: usize) -> Result<PathBuf> {
    let fs = std::fs::read(raw)?;
    if !(fs.len() as u64).is_multiple_of(MIB) {
        return Err("obraz FS nema delku v celych MiB".into());
    }
    let part = fs.len() as u64;
    // Pred oddilem 1 MiB, za nim 1 MiB na zalozni tabulku.
    let bytes = PART_START_BYTES + part + MIB;
    let mut d = gpt_disk(ss, bytes, OUR_TYPE, part, diskguard::OUR_LABEL, 5, &[])?;
    let at = PART_START_BYTES as usize;
    d[at..at + fs.len()].copy_from_slice(&fs);
    std::fs::remove_file(raw)?;
    put(out, &d)
}
