//! Blokove zarizeni souboroveho systemu nad klientem ovladace disku.
//!
//! Blok FS (4 KiB) = 8 sektoru po 512 B. Souvisle useky jdou jednim
//! volanim klienta (ten je rozdeli podle limitu protokolu).

use blockproto::{BlockClient, SECTOR, feature};
use fs::{BLOCK_SIZE, BlockDevice, Durability, Error, Result};

const SECTORS_PER_BLOCK: u64 = (BLOCK_SIZE / SECTOR) as u64;

pub struct BlkDev {
    client: BlockClient,
    blocks: u64,
    /// Zarizeni umi DISCARD (po prvnim NOT_SUPPORTED se uz nezkousi).
    discard: bool,
    /// Pocet bloku, ktere dostaly TRIM (statistika).
    pub discarded: u64,
    /// Zaruka trvanlivosti podle vlastnosti ovladace.
    durability: Durability,
}

impl BlkDev {
    pub fn new(client: BlockClient) -> Self {
        let blocks = client.sectors() / SECTORS_PER_BLOCK;
        // Prechodna chyba dotazu (napr. TIMED_OUT) se zkusi znovu; kdyz
        // trva, plati nejhorsi pripad (bez zaruky = jen pro cteni) a hlasi
        // se zvlast, aby se nepletla s diskem, ktery opravdu neumi flush.
        let mut f = 0;
        for attempt in 1..=3 {
            match client.features() {
                Ok(v) => {
                    f = v;
                    break;
                }
                // Vyprseny cas uz trval REPLY_TIMEOUT: dalsi pokus by start
                // jen zdrzel.
                Err(e) if attempt == 3 || e == Error::TimedOut => {
                    libos::println!(
                        "fsd: VAROVANI: vlastnosti disku nejde zjistit ({e}) - beru disk jako bez zaruky trvanlivosti"
                    );
                    break;
                }
                Err(_) => {}
            }
        }
        let durability = if f & feature::READ_ONLY != 0 {
            Durability::None
        } else if f & feature::FLUSH != 0 {
            Durability::Flush
        } else if f & feature::WRITE_THROUGH != 0 {
            Durability::WriteThrough
        } else {
            Durability::None
        };
        Self {
            client,
            blocks,
            discard: true,
            discarded: 0,
            durability,
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
        // Bez cekani na odpoved: vybere se pred dalsim ctenim nebo flush
        // (poradi odpovedi = poradi pozadavku, viz `blockproto`).
        self.client.write_queued(start * SECTORS_PER_BLOCK, data)
    }

    fn durability(&self) -> Durability {
        self.durability
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
