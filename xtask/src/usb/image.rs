//! Obraz ziveho USB: GPT, ESP (FAT32) se zavadecem, jadrem a initfs
//! a datovy oddil s nasim typem GPT a souborovym systemem.
//!
//! Rozlozeni (sektor 512 B):
//!
//! | oblast        | od      | velikost             | obsah                              |
//! |---------------|---------|----------------------|------------------------------------|
//! | MBR + GPT     | 0       | 34 sektoru           | ochranny MBR, hlavicka, tabulka    |
//! | ESP           | 1 MiB   | [`ESP_MIB`] MiB      | FAT32: EFI/BOOT/BOOT*.EFI, os/...   |
//! | MO-LOGY       | za ESP  | [`LOGY_MIB`] MiB     | FAT32 pro logy (cte ho Windows)    |
//! | data          | za logy | do konce obrazu      | nas FS (oddil "MagnumOpus")        |
//! | zalozni GPT   | konec   | 33 sektoru           | tabulka a hlavicka                 |
//!
//! Obraz se pise po castech primo do souboru (zadny buffer velikosti
//! celeho obrazu: hostitel ma omezenou pamet). Datovy FS formatuje stejna
//! knihovna `fs`, jakou pouziva sluzba fsd.

use std::fs::{File, OpenOptions};
use std::io::{Seek, SeekFrom, Write};
use std::path::Path;

use diskguard::gpt::{self, NewPartition};
use fs::{Algo, BlockDevice, FormatOptions, Fs, Kind};

use crate::Result;

const SECTOR: u64 = 512;
const MIB: u64 = 1024 * 1024;
/// Zacatek ESP (zarovnani 1 MiB, obvykle pro flash).
const ESP_START: u64 = MIB;
/// Velikost ESP. Ladici initfs ma ~35 MiB, rezerva na budouci soubory
/// (zalozni zavadec, aktualizace).
pub const ESP_MIB: u64 = 128;
/// Oddil pro logy (FAT32 `MO-LOGY`, typ basic data): system do nej kopiruje
/// logy, zaznamy padu a balicek pro vyvojare (`user/logd/src/fatlog.rs`).
pub const LOGY_MIB: u64 = 64;
/// Nejmensi obraz: ESP + logy + 64 MiB dat + GPT.
pub const MIN_MIB: u64 = ESP_MIB + LOGY_MIB + 64 + 2;
/// Jmenovka datoveho FS.
pub const DATA_LABEL: &str = "MagnumOpus";

/// Oddil MO-LOGY v obrazu (bajty: od, delka).
pub fn logy_range() -> (u64, u64) {
    (ESP_START + ESP_MIB * MIB, LOGY_MIB * MIB)
}

/// Udaje o vytvorenem obrazu.
pub struct Layout {
    pub size: u64,
    /// Datovy oddil v bajtech (od, delka).
    pub data: (u64, u64),
}

/// Vytvori obraz `path` o `size_mib` MiB. `esp_files` = (cesta v ESP,
/// zdrojovy soubor).
pub fn create(path: &Path, size_mib: u64, esp_files: &[(&str, &Path)]) -> Result<Layout> {
    if size_mib < MIN_MIB {
        return Err(format!("obraz USB musi mit aspon {MIN_MIB} MiB").into());
    }
    let size = size_mib * MIB;
    let sectors = size / SECTOR;
    let esp_first = ESP_START / SECTOR;
    let esp_last = esp_first + ESP_MIB * MIB / SECTOR - 1;
    let logy_first = esp_last + 1;
    let logy_last = logy_first + LOGY_MIB * MIB / SECTOR - 1;
    let data_first = logy_last + 1;
    // Datovy oddil konci na hranici 1 MiB pred zalozni tabulkou (cele bloky
    // FS po 4 KiB).
    let data_last = (gpt::last_usable(sectors, SECTOR as usize) + 1) / (MIB / SECTOR) * (MIB / SECTOR) - 1;
    let data = (data_first * SECTOR, (data_last + 1 - data_first) * SECTOR);

    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut file = OpenOptions::new().read(true).write(true).create(true).truncate(true).open(path)?;
    file.set_len(size)?;

    // ESP: FAT32 v pameti (128 MiB), pak do souboru.
    let mut esp = vec![0u8; (ESP_MIB * MIB) as usize];
    crate::image::write_fat_partition(&mut esp, esp_files, *b"MAGNUMOPUS ")?;
    file.seek(SeekFrom::Start(ESP_START))?;
    file.write_all(&esp)?;
    drop(esp);

    // Oddil pro logy: prazdny FAT32 s navodem.
    let mut logy = vec![0u8; (LOGY_MIB * MIB) as usize];
    let readme = crate::target_dir().join("usb-logy-readme.txt");
    std::fs::write(&readme, LOGY_README)?;
    crate::image::write_fat_partition(&mut logy, &[("CTI-MNE.TXT", readme.as_path())], *b"MO-LOGY    ")?;
    file.seek(SeekFrom::Start(logy_first * SECTOR))?;
    file.write_all(&logy)?;
    drop(logy);

    // Datovy FS primo v souboru na offsetu oddilu.
    format_data(&file, data, random_uuid())?;

    let disk_guid = random_uuid();
    let ends = gpt::build(
        sectors,
        SECTOR as usize,
        disk_guid,
        &[
            NewPartition {
                type_guid: diskguard::ESP_TYPE,
                guid: random_uuid(),
                first: esp_first,
                last: esp_last,
                attrs: 0,
                name: "EFI System",
            },
            NewPartition {
                type_guid: diskguard::BASIC_DATA_TYPE,
                guid: random_uuid(),
                first: logy_first,
                last: logy_last,
                attrs: 0,
                name: diskguard::LOG_LABEL,
            },
            NewPartition {
                type_guid: diskguard::OUR_TYPE,
                guid: random_uuid(),
                first: data_first,
                last: data_last,
                attrs: 0,
                name: diskguard::OUR_LABEL,
            },
        ],
    )?;
    file.seek(SeekFrom::Start(0))?;
    file.write_all(&ends.head)?;
    file.seek(SeekFrom::Start(size - ends.tail.len() as u64))?;
    file.write_all(&ends.tail)?;
    file.sync_all()?;
    Ok(Layout { size, data })
}

const LOGY_README: &str = "MagnumOpus - logy a zaznamy padu\r\n\r\n\
System sem kopiruje logy (slozka MAGNUMOS). Soubor REPORT.TXT posilejte vyvojari.\r\n";

/// Blokove zarizeni FS nad casti souboru (oddil v obrazu).
struct PartDevice<'a> {
    file: &'a File,
    offset: u64,
    blocks: u64,
}

impl BlockDevice for PartDevice<'_> {
    fn block_count(&self) -> u64 {
        self.blocks
    }

    fn read_block(&mut self, block: u64, buf: &mut [u8]) -> abi::Result<()> {
        self.read_blocks(block, buf)
    }

    fn write_block(&mut self, block: u64, data: &[u8]) -> abi::Result<()> {
        self.write_blocks(block, data)
    }

    fn read_blocks(&mut self, start: u64, buf: &mut [u8]) -> abi::Result<()> {
        use std::io::Read;
        let mut f = self.file;
        f.seek(SeekFrom::Start(self.offset + start * fs::BLOCK_SIZE as u64))
            .and_then(|_| f.read_exact(buf))
            .map_err(|_| abi::Error::IoError)
    }

    fn write_blocks(&mut self, start: u64, data: &[u8]) -> abi::Result<()> {
        let mut f = self.file;
        f.seek(SeekFrom::Start(self.offset + start * fs::BLOCK_SIZE as u64))
            .and_then(|_| f.write_all(data))
            .map_err(|_| abi::Error::IoError)
    }

    fn flush(&mut self) -> abi::Result<()> {
        // Trvanlivost resi `sync_all` na konci tvorby obrazu.
        Ok(())
    }
}

fn fs_err(what: &str, e: fs::Error) -> Box<dyn std::error::Error> {
    format!("datovy oddil USB: {what}: {e:?}").into()
}

fn format_data(file: &File, (offset, len): (u64, u64), uuid: [u8; 16]) -> Result<()> {
    let dev = PartDevice {
        file,
        offset,
        blocks: len / fs::BLOCK_SIZE as u64,
    };
    let opts = FormatOptions {
        label: DATA_LABEL.into(),
        algo: Algo::Xxh3,
        ag_blocks: None,
        uuid: Some(uuid),
        time: now(),
    };
    let dev = Fs::format(dev, &opts).map_err(|e| fs_err("formatovani", e))?;
    let mut fs = Fs::mount(dev).map_err(|e| fs_err("pripojeni", e))?;
    fs.set_time(now());
    let readme: &[u8] = b"Datovy oddil ziveho USB MagnumOpus.\n\
        Logy jsou v /system/log, zaznamy padu v /system/pady.\n";
    fs.create("/vitejte.txt", Kind::File).map_err(|e| fs_err("soubor", e))?;
    fs.write("/vitejte.txt", 0, readme).map_err(|e| fs_err("soubor", e))?;
    fs.commit().map_err(|e| fs_err("commit", e))?;
    let report = fs.check(true).map_err(|e| fs_err("kontrola", e))?;
    if !report.problems.is_empty() {
        return Err(format!("datovy oddil USB: kontrola konzistence: {:?}", report.problems).into());
    }
    Ok(())
}

fn now() -> fs::Timestamp {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| fs::Timestamp::new(d.as_secs() as i64, d.subsec_nanos() as i64))
        .unwrap_or(fs::Timestamp::UNIX_EPOCH)
}

/// Nahodne UUID verze 4 (nahoda z `RandomState`, viz `datadisk.rs`).
pub fn random_uuid() -> [u8; 16] {
    use std::hash::{BuildHasher, Hasher};
    let mut u = [0u8; 16];
    for half in u.chunks_mut(8) {
        let mut h = std::collections::hash_map::RandomState::new().build_hasher();
        h.write_u64(0);
        half.copy_from_slice(&h.finish().to_le_bytes());
    }
    u[6] = (u[6] & 0x0F) | 0x40;
    u[8] = (u[8] & 0x3F) | 0x80;
    u
}
