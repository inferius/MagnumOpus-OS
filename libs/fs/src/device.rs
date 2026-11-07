//! Blokove zarizeni pod souborovym systemem.

use alloc::vec;
use alloc::vec::Vec;

use abi::{Error, Result};

use crate::layout::BLOCK_SIZE;

/// Zaruka trvanlivosti zapisu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Durability {
    /// `flush` opravdu ulozi vse drive zapsane na medium.
    Flush,
    /// Zarizeni nema mezipamet zapisu: dokonceny zapis je trvaly.
    WriteThrough,
    /// Zadna zaruka (nebo neznama): jen pro cteni.
    None,
}

/// Zarizeni s bloky po `BLOCK_SIZE` bajtech.
pub trait BlockDevice {
    /// Pocet bloku.
    fn block_count(&self) -> u64;
    fn read_block(&mut self, block: u64, buf: &mut [u8]) -> Result<()>;
    fn write_block(&mut self, block: u64, data: &[u8]) -> Result<()>;
    /// Vse zapsane drive je trvale ulozene (pred zapisem superbloku).
    fn flush(&mut self) -> Result<()>;

    /// Jakou zaruku trvanlivosti zarizeni dava. Zarizeni bez zaruky (ma
    /// mezipamet zapisu a neumi flush) se pripoji jen pro cteni: commit by
    /// jinak mohl po vypadku proudu nechat superblok ukazovat na nezapsane
    /// bloky a `fsync` by lhal.
    fn durability(&self) -> Durability {
        Durability::Flush
    }

    /// Precte souvisly usek bloku (`buf.len()` = nasobek `BLOCK_SIZE`).
    /// Zarizeni s podporou vetsich pozadavku si metodu prepise.
    fn read_blocks(&mut self, start: u64, buf: &mut [u8]) -> Result<()> {
        for (i, b) in buf.chunks_mut(BLOCK_SIZE).enumerate() {
            self.read_block(start + i as u64, b)?;
        }
        Ok(())
    }

    /// Zapise souvisly usek bloku.
    fn write_blocks(&mut self, start: u64, data: &[u8]) -> Result<()> {
        for (i, b) in data.chunks(BLOCK_SIZE).enumerate() {
            self.write_block(start + i as u64, b)?;
        }
        Ok(())
    }

    /// Napoveda, ze bloky uz nejsou pouzite (TRIM/UNMAP). Vychozi: nic.
    fn discard(&mut self, _start: u64, _len: u64) -> Result<()> {
        Ok(())
    }
}

/// Zarizeni v pameti (testy, nastroje).
pub struct MemDevice {
    pub data: Vec<u8>,
    /// Pocet bloku, ktere dostaly TRIM (statistika pro testy).
    pub discarded: u64,
    /// Statistika pro testy a mereni: zapsane bloky, volani zapisu, flush.
    pub written: u64,
    pub write_calls: u64,
    pub flushes: u64,
}

impl MemDevice {
    pub fn new(blocks: u64) -> Self {
        Self {
            data: vec![0; (blocks * BLOCK_SIZE as u64) as usize],
            discarded: 0,
            written: 0,
            write_calls: 0,
            flushes: 0,
        }
    }
}

impl BlockDevice for MemDevice {
    fn block_count(&self) -> u64 {
        (self.data.len() / BLOCK_SIZE) as u64
    }

    fn read_block(&mut self, block: u64, buf: &mut [u8]) -> Result<()> {
        let start = block as usize * BLOCK_SIZE;
        let src = self
            .data
            .get(start..start + buf.len())
            .ok_or(Error::OutOfRange)?;
        buf.copy_from_slice(src);
        Ok(())
    }

    fn write_block(&mut self, block: u64, data: &[u8]) -> Result<()> {
        let start = block as usize * BLOCK_SIZE;
        let dst = self
            .data
            .get_mut(start..start + data.len())
            .ok_or(Error::OutOfRange)?;
        dst.copy_from_slice(data);
        self.written += (data.len() / BLOCK_SIZE) as u64;
        self.write_calls += 1;
        Ok(())
    }

    fn read_blocks(&mut self, start: u64, buf: &mut [u8]) -> Result<()> {
        self.read_block(start, buf)
    }

    fn write_blocks(&mut self, start: u64, data: &[u8]) -> Result<()> {
        self.write_block(start, data)
    }

    fn flush(&mut self) -> Result<()> {
        self.flushes += 1;
        Ok(())
    }

    fn discard(&mut self, start: u64, len: u64) -> Result<()> {
        // Jako SSD po TRIM: obsah je nedefinovany (tady nuly).
        let s = start as usize * BLOCK_SIZE;
        let e = (start + len) as usize * BLOCK_SIZE;
        self.data.get_mut(s..e).ok_or(Error::OutOfRange)?.fill(0);
        self.discarded += len;
        Ok(())
    }
}

/// Soubor s obrazem disku (nastroje na hostiteli, napr. xtask).
#[cfg(feature = "std")]
pub struct FileDevice {
    file: std::fs::File,
    blocks: u64,
}

#[cfg(feature = "std")]
impl FileDevice {
    pub fn open(path: &std::path::Path) -> std::io::Result<Self> {
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(path)?;
        let blocks = file.metadata()?.len() / BLOCK_SIZE as u64;
        Ok(Self { file, blocks })
    }
}

#[cfg(feature = "std")]
impl BlockDevice for FileDevice {
    fn block_count(&self) -> u64 {
        self.blocks
    }

    fn read_block(&mut self, block: u64, buf: &mut [u8]) -> Result<()> {
        use std::io::{Read, Seek, SeekFrom};
        self.file
            .seek(SeekFrom::Start(block * BLOCK_SIZE as u64))
            .and_then(|_| self.file.read_exact(buf))
            .map_err(|_| Error::IoError)
    }

    fn write_block(&mut self, block: u64, data: &[u8]) -> Result<()> {
        use std::io::{Seek, SeekFrom, Write};
        self.file
            .seek(SeekFrom::Start(block * BLOCK_SIZE as u64))
            .and_then(|_| self.file.write_all(data))
            .map_err(|_| Error::IoError)
    }

    fn flush(&mut self) -> Result<()> {
        self.file.sync_data().map_err(|_| Error::IoError)
    }

    fn read_blocks(&mut self, start: u64, buf: &mut [u8]) -> Result<()> {
        self.read_block(start, buf)
    }

    fn write_blocks(&mut self, start: u64, data: &[u8]) -> Result<()> {
        self.write_block(start, data)
    }
}
