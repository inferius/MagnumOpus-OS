//! Perzistence volneho mista: space logy alokacnich skupin a jejich tabulka.
//!
//! ```text
//! superblok.space -> index (az 127 ukazatelu na bloky tabulky)
//! blok tabulky    -> az 85 zaznamu AG: hlava logu (BlockPtr), pocet bloku, pocet zaznamu
//! blok logu       -> hlavicka (magic "SL", pocet, AG, ukazatel na predchozi blok)
//!                    + az 168 zaznamu (zacatek u64, delka u32, alokace u8, txg u64)
//! ```
//!
//! Log je retez od nejnovejsiho bloku ke starsim. Commit nove zaznamy pripise:
//! neplny posledni blok nahradi novym (copy-on-write), plny necha a navaze za
//! nej. Pri pripojeni se log prehraje od nejstarsiho zaznamu.
//!
//! Alokace bloku pro samotne logy meni volne misto, ktere se zase musi
//! zapsat do logu. Reseni: priprava opakuje alokace, dokud se vysledek
//! neustali (alokace prida zaznam, ale bloku pribyva jen vyjimecne), a az pak
//! se vse zapise.

use alloc::collections::{BTreeMap, BTreeSet};
use alloc::vec;
use alloc::vec::Vec;

use abi::{Error, Result};

use super::{Class, Rec, Space};
use crate::device::BlockDevice;
use crate::hash::Algo;
use crate::layout::{BLOCK_SIZE, BlockPtr, PTR_SIZE, REUSE_DELAY, u32_at, u64_at};

const LOG_MAGIC: u16 = u16::from_le_bytes(*b"SL");
const TABLE_MAGIC: u16 = u16::from_le_bytes(*b"AT");
const INDEX_MAGIC: u16 = u16::from_le_bytes(*b"AI");
const LOG_HEADER: usize = 64;
const REC_SIZE: usize = 24;
/// Zaznamu v jednom bloku logu.
pub const LOG_PER_BLOCK: usize = (BLOCK_SIZE - LOG_HEADER) / REC_SIZE;
const TABLE_HEADER: usize = 16;
const TABLE_ENTRY: usize = PTR_SIZE + 16;
pub const TABLE_PER_BLOCK: usize = (BLOCK_SIZE - TABLE_HEADER) / TABLE_ENTRY;
pub const INDEX_PER_BLOCK: usize = (BLOCK_SIZE - TABLE_HEADER) / PTR_SIZE;

/// Stav logu jedne AG.
#[derive(Default, Clone)]
struct AgLog {
    head: BlockPtr,
    /// Ukazatel ulozeny v hlave (na predchozi blok).
    head_prev: BlockPtr,
    /// Zaznamy v hlave.
    tail: Vec<Rec>,
    /// Bloky retezu od nejstarsiho.
    chain: Vec<u64>,
    // --- priprava commitu ---
    condense: bool,
    old_freed: bool,
    new_blocks: Vec<u64>,
}

impl AgLog {
    /// Prepise se pri commitu hlava (neni plna)?
    fn rewrite_head(&self) -> bool {
        !self.condense && self.head.block != 0 && self.tail.len() < LOG_PER_BLOCK
    }
}

pub struct SpaceMeta {
    logs: Vec<AgLog>,
    tables: Vec<BlockPtr>,
    index: BlockPtr,
    new_tables: BTreeMap<usize, u64>,
    new_index: Option<u64>,
    old_index_freed: bool,
}

impl SpaceMeta {
    pub fn new(ag_count: usize) -> Self {
        Self {
            logs: vec![AgLog::default(); ag_count],
            tables: vec![BlockPtr::default(); ag_count.div_ceil(TABLE_PER_BLOCK)],
            index: BlockPtr::default(),
            new_tables: BTreeMap::new(),
            new_index: None,
            old_index_freed: false,
        }
    }

    /// Bloky metadat mista (kontrola konzistence).
    pub fn blocks(&self) -> Vec<u64> {
        let mut out: Vec<u64> = self
            .logs
            .iter()
            .flat_map(|l| l.chain.iter().copied())
            .collect();
        out.extend(self.tables.iter().filter(|p| p.block != 0).map(|p| p.block));
        if self.index.block != 0 {
            out.push(self.index.block);
        }
        out
    }

    /// Pocet bloku logu AG (testy zhusteni).
    #[cfg(test)]
    pub fn log_blocks(&self, ag: usize) -> usize {
        self.logs[ag].chain.len()
    }
}

fn encode_recs(b: &mut [u8], recs: &[Rec]) {
    for (i, r) in recs.iter().enumerate() {
        let o = LOG_HEADER + i * REC_SIZE;
        b[o..o + 8].copy_from_slice(&r.start.to_le_bytes());
        b[o + 8..o + 12].copy_from_slice(&(r.len as u32).to_le_bytes());
        b[o + 12] = r.alloc as u8;
        b[o + 16..o + 24].copy_from_slice(&r.txg.to_le_bytes());
    }
}

fn read_verified<D: BlockDevice>(
    dev: &mut D,
    algo: Algo,
    ptr: &BlockPtr,
    magic: u16,
) -> Result<Vec<u8>> {
    let mut b = vec![0u8; BLOCK_SIZE];
    dev.read_block(ptr.block, &mut b)?;
    if algo.sum(&b) != ptr.csum || u16::from_le_bytes([b[0], b[1]]) != magic {
        return Err(Error::Corrupted);
    }
    Ok(b)
}

fn write_block<D: BlockDevice>(
    dev: &mut D,
    algo: Algo,
    block: u64,
    txg: u64,
    b: &[u8],
) -> Result<BlockPtr> {
    dev.write_block(block, b)?;
    Ok(BlockPtr::single(block, txg, algo.sum(b)))
}

impl Space {
    /// Nacte volne misto ulozeneho stavu. `txg` = nasledujici transakce.
    pub fn load<D: BlockDevice>(
        dev: &mut D,
        algo: Algo,
        block_count: u64,
        ag_blocks: u64,
        index: &BlockPtr,
        txg: u64,
    ) -> Result<Self> {
        let mut s = Space::new(block_count, ag_blocks);
        let n = s.ags.len();
        let ib = read_verified(dev, algo, index, INDEX_MAGIC)?;
        let tables = u16::from_le_bytes([ib[2], ib[3]]) as usize;
        if tables != s.meta.tables.len() {
            return Err(Error::Corrupted);
        }
        s.meta.index = *index;
        let mut recent: Vec<Rec> = Vec::new();
        for t in 0..tables {
            let tp = BlockPtr::get(&ib[TABLE_HEADER + t * PTR_SIZE..]);
            s.meta.tables[t] = tp;
            let tb = read_verified(dev, algo, &tp, TABLE_MAGIC)?;
            let count = u16::from_le_bytes([tb[2], tb[3]]) as usize;
            let first = u32_at(&tb, 4) as usize;
            if first != t * TABLE_PER_BLOCK || first + count > n || count > TABLE_PER_BLOCK {
                return Err(Error::Corrupted);
            }
            for j in 0..count {
                let ag = first + j;
                let e = TABLE_HEADER + j * TABLE_ENTRY;
                let head = BlockPtr::get(&tb[e..]);
                let blocks = u32_at(&tb, e + PTR_SIZE) as usize;
                s.load_ag(dev, algo, ag, head, blocks, txg, &mut recent)?;
            }
        }
        // Nedavno uvolnene bloky jeste nesmi byt pouzity: zpet do odlozenych
        // (nejnovejsi zaznam vyhrava).
        for r in recent.iter().rev() {
            let ag = s.ag_of(r.start);
            let pieces: Vec<(u64, u64)> = s.ags[ag]
                .free
                .range(..r.start + r.len)
                .rev()
                .take_while(|&(&fs, &fl)| fs + fl > r.start)
                .map(|(&fs, &fl)| (fs.max(r.start), (fs + fl).min(r.start + r.len)))
                .map(|(a, b)| (a, b - a))
                .collect();
            for (ps, pl) in pieces {
                s.ags[ag].take(ps, pl)?;
                s.deferred
                    .entry(r.txg + REUSE_DELAY)
                    .or_default()
                    .push((ps, pl));
                s.deferred_blocks += pl;
            }
        }
        Ok(s)
    }

    #[allow(clippy::too_many_arguments)]
    fn load_ag<D: BlockDevice>(
        &mut self,
        dev: &mut D,
        algo: Algo,
        ag: usize,
        head: BlockPtr,
        blocks: usize,
        txg: u64,
        recent: &mut Vec<Rec>,
    ) -> Result<()> {
        let mut per_block: Vec<Vec<Rec>> = Vec::new();
        let mut chain = Vec::new();
        let mut ptr = head;
        let mut head_prev = BlockPtr::default();
        while ptr.block != 0 {
            if chain.len() >= blocks {
                return Err(Error::Corrupted);
            }
            let b = read_verified(dev, algo, &ptr, LOG_MAGIC)?;
            let count = u16::from_le_bytes([b[2], b[3]]) as usize;
            if count > LOG_PER_BLOCK || u32_at(&b, 4) as usize != ag {
                return Err(Error::Corrupted);
            }
            let recs = (0..count)
                .map(|i| {
                    let o = LOG_HEADER + i * REC_SIZE;
                    Rec {
                        start: u64_at(&b, o),
                        len: u32_at(&b, o + 8) as u64,
                        alloc: b[o + 12] != 0,
                        txg: u64_at(&b, o + 16),
                    }
                })
                .collect();
            per_block.push(recs);
            chain.push(ptr.block);
            let prev = BlockPtr::get(&b[16..16 + PTR_SIZE]);
            if chain.len() == 1 {
                head_prev = prev;
            }
            ptr = prev;
        }
        if chain.len() != blocks {
            return Err(Error::Corrupted);
        }
        chain.reverse();
        let (lo, hi) = self.ag_range(ag);
        for recs in per_block.iter().rev() {
            for r in recs {
                if r.len == 0 || r.start < lo || r.start.checked_add(r.len).is_none_or(|e| e > hi) {
                    return Err(Error::Corrupted);
                }
                if r.alloc {
                    self.ags[ag].take(r.start, r.len)?;
                } else {
                    let a = &self.ags[ag];
                    let overlaps = a.containing(r.start).is_some()
                        || a.free.range(r.start..r.start + r.len).next().is_some();
                    if overlaps {
                        return Err(Error::Corrupted);
                    }
                    self.ags[ag].give(r.start, r.len);
                    if r.txg + REUSE_DELAY > txg {
                        recent.push(*r);
                    }
                }
            }
        }
        let log = &mut self.meta.logs[ag];
        log.head = head;
        log.head_prev = head_prev;
        log.tail = per_block.into_iter().next().unwrap_or_default();
        log.chain = chain;
        Ok(())
    }

    /// Zhusteny obsah logu AG: alokovane useky + odlozena uvolneni.
    fn condensed(&self, ag: usize, txg: u64) -> Vec<Rec> {
        let (lo, hi) = self.ag_range(ag);
        let mut out = Vec::new();
        let mut pos = lo;
        for (&s, &l) in &self.ags[ag].free {
            if s > pos {
                out.push(Rec {
                    alloc: true,
                    start: pos,
                    len: s - pos,
                    txg,
                });
            }
            pos = s + l;
        }
        if pos < hi {
            out.push(Rec {
                alloc: true,
                start: pos,
                len: hi - pos,
                txg,
            });
        }
        for (&ready, list) in &self.deferred {
            for &(s, l) in list {
                let (a, b) = (s.max(lo), (s + l).min(hi));
                if a < b {
                    out.push(Rec {
                        alloc: false,
                        start: a,
                        len: b - a,
                        txg: ready - REUSE_DELAY,
                    });
                }
            }
        }
        out
    }

    fn new_records(&self, ag: usize, txg: u64) -> Vec<Rec> {
        let log = &self.meta.logs[ag];
        if log.condense {
            return self.condensed(ag, txg);
        }
        let mut r = if log.rewrite_head() {
            log.tail.clone()
        } else {
            Vec::new()
        };
        r.extend_from_slice(&self.ags[ag].delta);
        r
    }

    fn dirty_ags(&self) -> Vec<usize> {
        (0..self.ags.len())
            .filter(|&i| !self.ags[i].delta.is_empty() || self.meta.logs[i].condense)
            .collect()
    }

    /// Zapise zmeny volneho mista; vraci ukazatel na novy index.
    pub fn commit<D: BlockDevice>(
        &mut self,
        dev: &mut D,
        algo: Algo,
        txg: u64,
    ) -> Result<BlockPtr> {
        // Zhustit logy, ktere narostly vic nez dvojnasobne proti zhustene podobe.
        for ag in self.dirty_ags() {
            let estimate = (self.ags[ag].free.len() + 1).div_ceil(LOG_PER_BLOCK);
            if self.meta.logs[ag].chain.len() > 2 * estimate + 8 {
                self.meta.logs[ag].condense = true;
            }
        }
        // Priprava: alokovat a uvolnit vse potrebne, dokud se stav neustali.
        loop {
            let mut changed = false;
            for ag in self.dirty_ags() {
                let need = self.new_records(ag, txg).len().div_ceil(LOG_PER_BLOCK);
                while self.meta.logs[ag].new_blocks.len() < need {
                    let (b, _) = self.alloc(1, None, Class::Meta, txg)?;
                    self.meta.logs[ag].new_blocks.push(b);
                    changed = true;
                }
                let log = &self.meta.logs[ag];
                if !log.old_freed {
                    let old: Vec<u64> = if log.condense {
                        log.chain.clone()
                    } else if log.rewrite_head() {
                        vec![log.head.block]
                    } else {
                        Vec::new()
                    };
                    self.meta.logs[ag].old_freed = true;
                    for b in old {
                        self.free(b, 1, txg, false);
                        changed = true;
                    }
                }
            }
            let dirty_tables: BTreeSet<usize> = self
                .dirty_ags()
                .iter()
                .map(|a| a / TABLE_PER_BLOCK)
                .collect();
            for t in dirty_tables {
                if !self.meta.new_tables.contains_key(&t) {
                    let (b, _) = self.alloc(1, None, Class::Meta, txg)?;
                    self.meta.new_tables.insert(t, b);
                    let old = self.meta.tables[t].block;
                    if old != 0 {
                        self.free(old, 1, txg, false);
                    }
                    changed = true;
                }
            }
            if !self.meta.new_tables.is_empty() && self.meta.new_index.is_none() {
                let (b, _) = self.alloc(1, None, Class::Meta, txg)?;
                self.meta.new_index = Some(b);
                changed = true;
            }
            if self.meta.new_index.is_some() && !self.meta.old_index_freed {
                self.meta.old_index_freed = true;
                let old = self.meta.index.block;
                if old != 0 {
                    self.free(old, 1, txg, false);
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }
        // Zapis logu.
        for ag in self.dirty_ags() {
            let recs = self.new_records(ag, txg);
            let log = &self.meta.logs[ag];
            let mut prev = if log.condense {
                BlockPtr::default()
            } else if log.rewrite_head() {
                log.head_prev
            } else {
                log.head
            };
            let new_blocks = log.new_blocks.clone();
            let mut head_prev = prev;
            let mut last: Vec<Rec> = Vec::new();
            for (i, &blk) in new_blocks.iter().enumerate() {
                let chunk = recs
                    .get(i * LOG_PER_BLOCK..((i + 1) * LOG_PER_BLOCK).min(recs.len()))
                    .unwrap_or(&[]);
                let mut b = vec![0u8; BLOCK_SIZE];
                b[0..2].copy_from_slice(&LOG_MAGIC.to_le_bytes());
                b[2..4].copy_from_slice(&(chunk.len() as u16).to_le_bytes());
                b[4..8].copy_from_slice(&(ag as u32).to_le_bytes());
                prev.put(&mut b[16..16 + PTR_SIZE]);
                encode_recs(&mut b, chunk);
                head_prev = prev;
                prev = write_block(dev, algo, blk, txg, &b)?;
                last = chunk.to_vec();
            }
            let log = &mut self.meta.logs[ag];
            if log.condense {
                log.chain.clear();
            } else if log.rewrite_head() {
                log.chain.pop();
            }
            if new_blocks.is_empty() {
                // Jen zhusteni prazdne AG: log je prazdny.
                log.head = BlockPtr::default();
                log.head_prev = BlockPtr::default();
                log.tail.clear();
            } else {
                log.chain.extend_from_slice(&new_blocks);
                log.head = prev;
                log.head_prev = head_prev;
                log.tail = last;
            }
            log.condense = false;
            log.old_freed = false;
            log.new_blocks.clear();
        }
        // Tabulka AG a index.
        let n = self.ags.len();
        for (&t, &blk) in &self.meta.new_tables {
            let first = t * TABLE_PER_BLOCK;
            let count = TABLE_PER_BLOCK.min(n - first);
            let mut b = vec![0u8; BLOCK_SIZE];
            b[0..2].copy_from_slice(&TABLE_MAGIC.to_le_bytes());
            b[2..4].copy_from_slice(&(count as u16).to_le_bytes());
            b[4..8].copy_from_slice(&(first as u32).to_le_bytes());
            for j in 0..count {
                let log = &self.meta.logs[first + j];
                let e = TABLE_HEADER + j * TABLE_ENTRY;
                log.head.put(&mut b[e..e + PTR_SIZE]);
                b[e + PTR_SIZE..e + PTR_SIZE + 4]
                    .copy_from_slice(&(log.chain.len() as u32).to_le_bytes());
            }
            self.meta.tables[t] = write_block(dev, algo, blk, txg, &b)?;
        }
        if let Some(blk) = self.meta.new_index.take() {
            let mut b = vec![0u8; BLOCK_SIZE];
            b[0..2].copy_from_slice(&INDEX_MAGIC.to_le_bytes());
            b[2..4].copy_from_slice(&(self.meta.tables.len() as u16).to_le_bytes());
            for (t, p) in self.meta.tables.iter().enumerate() {
                p.put(&mut b[TABLE_HEADER + t * PTR_SIZE..]);
            }
            self.meta.index = write_block(dev, algo, blk, txg, &b)?;
        }
        self.meta.new_tables.clear();
        self.meta.old_index_freed = false;
        for a in &mut self.ags {
            a.delta.clear();
        }
        Ok(self.meta.index)
    }
}
