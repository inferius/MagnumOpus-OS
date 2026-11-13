//! Disky pro instalator: raw kanaly od initu (cely disk, zapis kontroluje
//! ovladac), cteni GPT, popis oddilu, oddil jako blokove zarizeni pro FAT
//! a FS.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use blockproto::{BlockClient, SECTOR};
use diskguard::gpt::{self, Read, Table};
use diskguard::{BASIC_DATA_TYPE, LOG_LABEL};
use installplan::DiskInfo;
use libos::{Error, Result};

/// Disk s raw kanalem.
pub struct Disk {
    pub client: BlockClient,
    pub model: String,
    /// Sektor zarizeni (instalator zatim umi jen 512 B).
    pub sector_size: usize,
}

impl Disk {
    /// Pocet sektoru po 512 B.
    pub fn sectors(&self) -> u64 {
        self.client.sectors()
    }

    pub fn read(&self, lba: u64, buf: &mut [u8]) -> Result<()> {
        self.client.read(lba, buf)
    }

    pub fn write(&self, lba: u64, data: &[u8]) -> Result<()> {
        self.client.write(lba, data)
    }

    /// Aktualni GPT (`None` = bez tabulky nebo poskozena).
    pub fn table(&self) -> Option<Table> {
        let sectors = self.sectors();
        match gpt::read_table(sectors, SECTOR, &mut |lba, buf| self.read(lba, buf).is_ok()) {
            Read::Table(t) => Some(t),
            _ => None,
        }
    }

    /// Je to disk zive flashky (oddil MO-LOGY a nas oddil)?
    pub fn is_live(t: Option<&Table>) -> bool {
        t.is_some_and(|t| {
            t.entries().any(|e| !e.is_empty() && e.type_guid == BASIC_DATA_TYPE && e.name_is(LOG_LABEL))
                && t.entries().any(|e| !e.is_empty() && diskguard::is_our_entry(&e))
        })
    }

    pub fn info(&self, index: u32, table: Option<&Table>) -> DiskInfo {
        DiskInfo {
            index,
            model: self.model.clone(),
            sectors: self.sectors(),
            sector_size: SECTOR,
            live: Self::is_live(table),
        }
    }

    /// Popis oddilu (vypis disku).
    pub fn describe(t: Option<&Table>) -> Vec<String> {
        let Some(t) = t else { return alloc::vec![String::from("bez tabulky oddílů (prázdný nebo neznámý disk)")] };
        t.entries()
            .filter(|e| !e.is_empty())
            .map(|e| {
                let name = installplan::entry_name(&e);
                format!("{} ({})", if name.is_empty() { String::from("oddíl") } else { name }, installplan::human((e.last - e.first + 1) * SECTOR as u64))
            })
            .collect()
    }
}

/// Usek disku (oddil) jako zarizeni: sektor 0 = prvni sektor oddilu.
pub struct Part<'a> {
    pub disk: &'a Disk,
    pub first: u64,
    pub sectors: u64,
}

impl Part<'_> {
    fn check(&self, lba: u64, len: usize) -> Result<u64> {
        let n = (len / SECTOR) as u64;
        if lba.checked_add(n).is_none_or(|e| e > self.sectors) {
            return Err(Error::OutOfRange);
        }
        Ok(self.first + lba)
    }
}

impl fatw::Device for Part<'_> {
    fn read(&mut self, lba: u64, buf: &mut [u8]) -> fatw::Result<()> {
        let at = self.check(lba, buf.len()).map_err(|_| fatw::Error::Io)?;
        self.disk.read(at, buf).map_err(|_| fatw::Error::Io)
    }
    fn write(&mut self, lba: u64, data: &[u8]) -> fatw::Result<()> {
        let at = self.check(lba, data.len()).map_err(|_| fatw::Error::Io)?;
        self.disk.write(at, data).map_err(|_| fatw::Error::Io)
    }
    fn flush(&mut self) -> fatw::Result<()> {
        self.disk.client.flush().map_err(|_| fatw::Error::Io)
    }
}

/// Oddil jako blokove zarizeni souboroveho systemu (bloky 4 KiB).
pub struct FsPart<'a>(pub Part<'a>);

const SPB: u64 = (fs::BLOCK_SIZE / SECTOR) as u64;

impl fs::BlockDevice for FsPart<'_> {
    fn block_count(&self) -> u64 {
        self.0.sectors / SPB
    }
    fn read_block(&mut self, block: u64, buf: &mut [u8]) -> fs::Result<()> {
        self.read_blocks(block, buf)
    }
    fn write_block(&mut self, block: u64, data: &[u8]) -> fs::Result<()> {
        self.write_blocks(block, data)
    }
    fn read_blocks(&mut self, start: u64, buf: &mut [u8]) -> fs::Result<()> {
        let at = self.0.check(start * SPB, buf.len())?;
        self.0.disk.read(at, buf)
    }
    fn write_blocks(&mut self, start: u64, data: &[u8]) -> fs::Result<()> {
        let at = self.0.check(start * SPB, data.len())?;
        self.0.disk.write(at, data)
    }
    fn flush(&mut self) -> fs::Result<()> {
        self.0.disk.client.flush()
    }
}
