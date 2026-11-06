//! Obsah souboru.
//!
//! - Soubor do `INLINE_MAX` bajtu lezi primo ve strome (polozka INLINE).
//! - Vetsi soubor ma extenty: polozka EXTENT s klicem = prvni logicky blok,
//!   hodnota = fyzicky zacatek, generace vzniku a soucet kazdeho bloku.
//! - Zapis jde nejdriv do bufferu v pameti (odlozena alokace). Az pri
//!   zapisu na disk (commit, fsync, preplneni bufferu) se po sobe jdouci
//!   bloky alokuji jako jeden souvisly usek s cilem hned za predchozim
//!   extentem souboru - soubor se neroztrha a zapis je sekvencni.
//! - Prepsani casti extentu ho rozdeli (levy zbytek, nova data, pravy
//!   zbytek); prepsane fyzicke bloky se uvolni podle pravidel `Store::release`.

use alloc::collections::BTreeSet;
use alloc::vec;
use alloc::vec::Vec;

use abi::{Error, Result};

use super::{DIRTY_LIMIT, Fs, zil};
use crate::btree;
use crate::device::BlockDevice;
use crate::layout::{
    BLOCK_SIZE, Extent, INLINE_MAX, Inode, KIND_DIR, KIND_FILE, Key, MAX_EXTENT, MAX_FILE_SIZE, item,
};
use crate::node::Child;

const BS: u64 = BLOCK_SIZE as u64;

/// Chyba pro operaci s obsahem nad objektem, ktery neni soubor.
fn not_file(kind: u8) -> Error {
    if kind == KIND_DIR { Error::IsADirectory } else { Error::WrongType }
}

/// Zkopiruje cast bloku `lb` (obsah `block`) do vystupu cteni `out`, ktere
/// zacina na bajtu `offset`.
fn copy_block(out: &mut [u8], offset: u64, lb: u64, block: &[u8]) {
    let end = offset + out.len() as u64;
    let bs = lb * BS;
    let lo = bs.max(offset);
    let hi = (bs + BS).min(end);
    if lo < hi {
        out[(lo - offset) as usize..(hi - offset) as usize]
            .copy_from_slice(&block[(lo - bs) as usize..(hi - bs) as usize]);
    }
}

impl<D: BlockDevice> Fs<D> {
    /// Extenty souboru v rozsahu logickych bloku `a..b` (vcetne tech, ktere
    /// do rozsahu jen zasahuji).
    pub(super) fn extents_in(
        &mut self,
        root: &Child,
        oid: u64,
        a: u64,
        b: u64,
    ) -> Result<Vec<(u64, Extent)>> {
        let start = Key::new(oid, item::EXTENT, a.saturating_sub(MAX_EXTENT - 1));
        let end = if b == u64::MAX {
            Key::new(oid, item::EXTENT + 1, 0)
        } else {
            Key::new(oid, item::EXTENT, b)
        };
        let mut out = Vec::new();
        for (k, raw) in btree::collect(&mut self.s, root, &start, &end)? {
            let e = Extent::decode(&raw)?;
            if k.offset + e.len() > a {
                out.push((k.offset, e));
            }
        }
        Ok(out)
    }

    pub(super) fn read_oid(
        &mut self,
        root: &Child,
        oid: u64,
        offset: u64,
        len: usize,
        live: bool,
    ) -> Result<Vec<u8>> {
        let i = self.inode(root, oid)?;
        if i.kind != KIND_FILE {
            return Err(not_file(i.kind));
        }
        if offset >= i.size || len == 0 {
            return Ok(Vec::new());
        }
        let len = (len as u64).min(i.size - offset) as usize;
        let mut out = vec![0u8; len];
        if i.size as usize <= INLINE_MAX {
            let data =
                btree::get(&mut self.s, root, &Key::new(oid, item::INLINE, 0))?.unwrap_or_default();
            let avail = data.len().saturating_sub(offset as usize).min(len);
            out[..avail].copy_from_slice(&data[offset as usize..offset as usize + avail]);
            return Ok(out);
        }
        let first = offset / BS;
        let last = (offset + len as u64 - 1) / BS;
        for (ks, e) in self.extents_in(root, oid, first, last + 1)? {
            let lo = ks.max(first);
            let hi = (ks + e.len()).min(last + 1);
            if lo >= hi {
                continue;
            }
            let rel = (lo - ks) as usize;
            let n = (hi - lo) as usize;
            let mut buf = vec![0u8; n * BLOCK_SIZE];
            self.s
                .read_data(e.phys + rel as u64, &e.csums[rel..rel + n], &mut buf)?;
            for (j, block) in buf.chunks(BLOCK_SIZE).enumerate() {
                copy_block(&mut out, offset, lo + j as u64, block);
            }
        }
        if live && let Some(fd) = self.data.get(&oid) {
            for (&lb, block) in fd.blocks.range(first..=last) {
                copy_block(&mut out, offset, lb, block);
            }
        }
        Ok(out)
    }

    pub(super) fn file_oid(&mut self, path: &str) -> Result<(u64, Inode)> {
        let root = self.s.root;
        let oid = self.resolve(&root, path)?;
        let i = self.inode(&root, oid)?;
        if i.kind != KIND_FILE {
            return Err(not_file(i.kind));
        }
        Ok((oid, i))
    }

    /// Zapise `data` od `offset` (soubor se pripadne zvetsi, diru vyplni nuly).
    pub fn write(&mut self, path: &str, offset: u64, data: &[u8]) -> Result<()> {
        let (oid, inode) = self.file_oid(path)?;
        if data.is_empty() {
            return Ok(());
        }
        let end = offset
            .checked_add(data.len() as u64)
            .filter(|&e| e <= MAX_FILE_SIZE)
            .ok_or(Error::OutOfRange)?;
        let blocks = end.div_ceil(BS) - offset / BS;
        self.ensure_room(blocks + 8)?;
        let new_size = inode.size.max(end);
        self.set_content(oid, inode, new_size, Some((offset, data)))?;
        if self.dirty_blocks > DIRTY_LIMIT {
            self.flush_all_data()?;
        }
        Ok(())
    }

    /// Zmeni velikost souboru (zvetseni doplni nuly).
    pub fn truncate(&mut self, path: &str, size: u64) -> Result<()> {
        let (oid, inode) = self.file_oid(path)?;
        if size > MAX_FILE_SIZE {
            return Err(Error::OutOfRange);
        }
        if size == inode.size {
            return Ok(());
        }
        self.ensure_room(8)?;
        self.truncate_oid(oid, size)
    }

    pub(super) fn truncate_oid(&mut self, oid: u64, size: u64) -> Result<()> {
        let inode = self.live_inode(oid)?;
        self.set_content(oid, inode, size, None)?;
        self.zil.op(zil::Op::Truncate {
            oid,
            size,
            time: self.now,
        });
        Ok(())
    }

    fn store_inline(&mut self, oid: u64, data: Vec<u8>) -> Result<()> {
        let key = Key::new(oid, item::INLINE, 0);
        if data.is_empty() {
            self.del(&key)?;
            Ok(())
        } else {
            self.put(key, data)
        }
    }

    /// Rozpracovany blok `lb` souboru (pri prvnim pouziti nacte stary obsah,
    /// pokud se blok prepisuje jen zcasti).
    fn dirty_block(&mut self, oid: u64, lb: u64, partial: bool) -> Result<&mut Vec<u8>> {
        let exists = self
            .data
            .get(&oid)
            .is_some_and(|fd| fd.blocks.contains_key(&lb));
        if !exists {
            let mut block = vec![0u8; BLOCK_SIZE];
            if partial {
                let root = self.s.root;
                if let Some((ks, e)) = self.extents_in(&root, oid, lb, lb + 1)?.into_iter().next() {
                    let rel = (lb - ks) as usize;
                    self.s
                        .read_data(e.phys + rel as u64, &e.csums[rel..rel + 1], &mut block)?;
                }
            }
            self.data.entry(oid).or_default().blocks.insert(lb, block);
            self.dirty_blocks += 1;
        }
        Ok(self
            .data
            .get_mut(&oid)
            .unwrap()
            .blocks
            .get_mut(&lb)
            .unwrap())
    }

    fn write_dirty(&mut self, oid: u64, offset: u64, bytes: &[u8]) -> Result<()> {
        let mut pos = offset;
        let end = offset + bytes.len() as u64;
        while pos < end {
            let lb = pos / BS;
            let in_block = (pos % BS) as usize;
            let n = (BLOCK_SIZE - in_block).min((end - pos) as usize);
            let at = (pos - offset) as usize;
            let block = self.dirty_block(oid, lb, n < BLOCK_SIZE)?;
            block[in_block..in_block + n].copy_from_slice(&bytes[at..at + n]);
            self.zil.touch_block(oid, lb);
            pos += n as u64;
        }
        Ok(())
    }

    /// Zahodi obsah souboru od logickeho bloku `from` (extenty i buffer).
    fn drop_blocks_from(&mut self, oid: u64, from: u64) -> Result<()> {
        self.punch_range(oid, from, u64::MAX)?;
        if let Some(fd) = self.data.get_mut(&oid) {
            let dropped = fd.blocks.split_off(&from);
            self.dirty_blocks -= dropped.len() as u64;
            if fd.blocks.is_empty() {
                self.data.remove(&oid);
            }
        }
        self.zil.drop_blocks_from(oid, from);
        Ok(())
    }

    /// Odstrani veskery obsah souboru (pri mazani).
    pub(super) fn drop_content(&mut self, oid: u64) -> Result<()> {
        self.store_inline(oid, Vec::new())?;
        self.drop_blocks_from(oid, 0)
    }

    /// Nastavi velikost a obsah souboru podle pravidla: velikost <=
    /// INLINE_MAX = data ve strome, jinak v blocich.
    pub(super) fn set_content(
        &mut self,
        oid: u64,
        mut inode: Inode,
        new_size: u64,
        write: Option<(u64, &[u8])>,
    ) -> Result<()> {
        let old_size = inode.size;
        let was_inline = old_size as usize <= INLINE_MAX;
        let now_inline = new_size as usize <= INLINE_MAX;
        let root = self.s.root;
        match (was_inline, now_inline) {
            (true, true) => {
                let mut data = btree::get(&mut self.s, &root, &Key::new(oid, item::INLINE, 0))?
                    .unwrap_or_default();
                data.resize(new_size as usize, 0);
                if let Some((off, bytes)) = write {
                    data[off as usize..off as usize + bytes.len()].copy_from_slice(bytes);
                }
                self.store_inline(oid, data)?;
            }
            (true, false) => {
                let data = btree::get(&mut self.s, &root, &Key::new(oid, item::INLINE, 0))?
                    .unwrap_or_default();
                self.store_inline(oid, Vec::new())?;
                if !data.is_empty() {
                    self.write_dirty(oid, 0, &data)?;
                }
                if let Some((off, bytes)) = write {
                    self.write_dirty(oid, off, bytes)?;
                }
            }
            (false, true) => {
                let mut data = self.read_oid(&root, oid, 0, new_size as usize, true)?;
                data.resize(new_size as usize, 0);
                if let Some((off, bytes)) = write {
                    data[off as usize..off as usize + bytes.len()].copy_from_slice(bytes);
                }
                self.drop_blocks_from(oid, 0)?;
                self.store_inline(oid, data)?;
            }
            (false, false) => {
                if new_size < old_size {
                    self.drop_blocks_from(oid, new_size.div_ceil(BS))?;
                    // Konec posledniho castecneho bloku vynulovat, aby pozdejsi
                    // zvetseni cetlo nuly.
                    let tail = (new_size % BS) as usize;
                    if tail != 0 {
                        let lb = new_size / BS;
                        let has_extent = !self.extents_in(&root, oid, lb, lb + 1)?.is_empty();
                        let has_dirty = self
                            .data
                            .get(&oid)
                            .is_some_and(|fd| fd.blocks.contains_key(&lb));
                        if has_extent || has_dirty {
                            self.dirty_block(oid, lb, true)?[tail..].fill(0);
                            self.zil.touch_block(oid, lb);
                        }
                    }
                }
                if let Some((off, bytes)) = write {
                    self.write_dirty(oid, off, bytes)?;
                }
            }
        }
        inode.size = new_size;
        inode.mtime = self.now;
        self.put_inode(oid, &inode)?;
        self.zil.touch(oid);
        let parent = inode.parent;
        // Obe velikosti <= MAX_FILE_SIZE = i64::MAX, rozdil se vejde do i64.
        self.adjust_tree(parent, new_size as i64 - old_size as i64, 0, 0)
    }

    /// Odstrani mapovani logickych bloku `a..b` (rozdeli castecne prekryte
    /// extenty) a uvolni jejich fyzicke bloky.
    pub(super) fn punch_range(&mut self, oid: u64, a: u64, b: u64) -> Result<()> {
        let root = self.s.root;
        for (ks, e) in self.extents_in(&root, oid, a, b)? {
            let ke = ks + e.len();
            let lo = ks.max(a);
            let hi = ke.min(b);
            if lo >= hi {
                continue;
            }
            self.s.release(e.phys + (lo - ks), hi - lo, e.birth);
            if ks < lo {
                let left = Extent {
                    phys: e.phys,
                    birth: e.birth,
                    csums: e.csums[..(lo - ks) as usize].to_vec(),
                };
                self.put(Key::new(oid, item::EXTENT, ks), left.encode())?;
            } else {
                self.del(&Key::new(oid, item::EXTENT, ks))?;
            }
            if hi < ke {
                let r = (hi - ks) as usize;
                let right = Extent {
                    phys: e.phys + r as u64,
                    birth: e.birth,
                    csums: e.csums[r..].to_vec(),
                };
                self.put(Key::new(oid, item::EXTENT, hi), right.encode())?;
            }
        }
        Ok(())
    }

    /// Fyzicky blok hned za extentem, ktery mapuje logicky blok `lb`.
    fn goal_after(&mut self, oid: u64, lb: u64) -> Result<Option<u64>> {
        let root = self.s.root;
        Ok(self
            .extents_in(&root, oid, lb, lb + 1)?
            .into_iter()
            .next()
            .map(|(ks, e)| e.phys + (lb - ks) + 1))
    }

    /// Zapise rozpracovane bloky souboru na disk (vsechny, nebo jen `only`).
    /// Po sobe jdouci bloky dostanou jeden souvisly usek.
    pub(super) fn writeback(&mut self, oid: u64, only: Option<&BTreeSet<u64>>) -> Result<()> {
        let Some(fd) = self.data.get_mut(&oid) else {
            return Ok(());
        };
        let picked: Vec<(u64, Vec<u8>)> = match only {
            None => core::mem::take(&mut fd.blocks).into_iter().collect(),
            Some(set) => {
                let keys: Vec<u64> = set
                    .iter()
                    .copied()
                    .filter(|lb| fd.blocks.contains_key(lb))
                    .collect();
                keys.into_iter()
                    .map(|lb| (lb, fd.blocks.remove(&lb).unwrap()))
                    .collect()
            }
        };
        if fd.blocks.is_empty() {
            self.data.remove(&oid);
        }
        self.dirty_blocks -= picked.len() as u64;
        let mut i = 0;
        while i < picked.len() {
            let mut j = i + 1;
            while j < picked.len() && picked[j].0 == picked[j - 1].0 + 1 {
                j += 1;
            }
            self.write_run(oid, &picked[i..j])?;
            i = j;
        }
        Ok(())
    }

    fn write_run(&mut self, oid: u64, run: &[(u64, Vec<u8>)]) -> Result<()> {
        let a = run[0].0;
        let n = run.len() as u64;
        self.punch_range(oid, a, a + n)?;
        let mut goal = if a > 0 {
            self.goal_after(oid, a - 1)?
        } else {
            None
        };
        let mut done = 0u64;
        while done < n {
            let (start, got) = self.s.alloc_data(n - done, goal)?;
            let piece = &run[done as usize..(done + got) as usize];
            let buf: Vec<u8> = piece.iter().flat_map(|(_, b)| b.iter().copied()).collect();
            self.s.dev.write_blocks(start, &buf)?;
            let algo = self.s.algo;
            let csums: Vec<_> = piece.iter().map(|(_, b)| algo.sum(b)).collect();
            let txg = self.s.txg;
            for (k, chunk) in csums.chunks(MAX_EXTENT as usize).enumerate() {
                let off = k as u64 * MAX_EXTENT;
                let e = Extent {
                    phys: start + off,
                    birth: txg,
                    csums: chunk.to_vec(),
                };
                self.put(Key::new(oid, item::EXTENT, a + done + off), e.encode())?;
            }
            done += got;
            goal = Some(start + got);
        }
        Ok(())
    }

    /// Zapise vsechna rozpracovana data.
    pub(super) fn flush_all_data(&mut self) -> Result<()> {
        let oids: Vec<u64> = self.data.keys().copied().collect();
        for oid in oids {
            self.writeback(oid, None)?;
        }
        Ok(())
    }

    /// Pocet souvislych fyzickych useku souboru (1 = nefragmentovany).
    pub fn fragments(&mut self, path: &str) -> Result<u64> {
        let (oid, _) = self.file_oid(path)?;
        let root = self.s.root;
        let ext = self.extents_in(&root, oid, 0, u64::MAX)?;
        let mut runs = 0;
        let mut next = None;
        for (_, e) in &ext {
            if next != Some(e.phys) {
                runs += 1;
            }
            next = Some(e.phys + e.len());
        }
        Ok(runs)
    }
}
