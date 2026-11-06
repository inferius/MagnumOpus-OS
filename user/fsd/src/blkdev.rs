//! Blokove zarizeni souboroveho systemu nad klientem ovladace disku.
//!
//! Blok FS (4 KiB) = 8 sektoru po 512 B. Souvisle useky jdou jednim
//! volanim klienta (ten je rozdeli podle limitu protokolu).

use blockproto::{BlockClient, SECTOR};
use fs::{BLOCK_SIZE, BlockDevice, Error, Result};

const SECTORS_PER_BLOCK: u64 = (BLOCK_SIZE / SECTOR) as u64;

pub struct BlkDev {
    client: BlockClient,
    blocks: u64,
    /// Zarizeni umi DISCARD (po prvnim NOT_SUPPORTED se uz nezkousi).
    discard: bool,
    /// Pocet bloku, ktere dostaly TRIM (statistika).
    pub discarded: u64,
}

impl BlkDev {
    pub fn new(client: BlockClient) -> Self {
        let blocks = client.sectors() / SECTORS_PER_BLOCK;
        Self {
            client,
            blocks,
            discard: true,
            discarded: 0,
        }
    }
}

impl BlockDevice for BlkDev {
    fn block_count(&self) -> u64 {
        self.blocks
    }

    fn read_block(&mut self, block: u64, buf: &mut [u8]) -> Result<()> {
        self.read_blocks(block, buf)
    }

    fn write_block(&mut self, block: u64, data: &[u8]) -> Result<()> {
        self.write_blocks(block, data)
    }

    fn read_blocks(&mut self, start: u64, buf: &mut [u8]) -> Result<()> {
        if start + (buf.len() / BLOCK_SIZE) as u64 > self.blocks {
            return Err(Error::OutOfRange);
        }
        self.client.read(start * SECTORS_PER_BLOCK, buf)
    }

    fn write_blocks(&mut self, start: u64, data: &[u8]) -> Result<()> {
        if start + (data.len() / BLOCK_SIZE) as u64 > self.blocks {
            return Err(Error::OutOfRange);
        }
        self.client.write(start * SECTORS_PER_BLOCK, data)
    }

    fn flush(&mut self) -> Result<()> {
        self.client.flush()
    }

    fn discard(&mut self, start: u64, len: u64) -> Result<()> {
        if !self.discard {
            return Err(Error::NotSupported);
        }
        match self
            .client
            .discard(start * SECTORS_PER_BLOCK, len * SECTORS_PER_BLOCK)
        {
            Ok(()) => {
                self.discarded += len;
                Ok(())
            }
            Err(Error::NotSupported) => {
                self.discard = false;
                Err(Error::NotSupported)
            }
            Err(e) => Err(e),
        }
    }
}
