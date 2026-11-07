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

use alloc::vec;
use alloc::vec::Vec;

use abi::{Error, Result};

use super::{DIRTY_HARD_LIMIT, DIRTY_LIMIT, Fs, zil};
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
        self.extents_batch(root, oid, a, b, usize::MAX)
    }

    /// Nejvys `limit` extentu z rozsahu `a..b` (od zacatku). Pro zpracovani
    /// po davkach: obri soubor (1 TiB = ~540 000 extentu se soucty, ~4 GiB)
    /// se nikdy nenacita do pameti cely.
    pub(super) fn extents_batch(
        &mut self,
        root: &Child,
        oid: u64,
        a: u64,
        b: u64,
        limit: usize,
    ) -> Result<Vec<(u64, Extent)>> {
        let start = Key::new(oid, item::EXTENT, a.saturating_sub(MAX_EXTENT - 1));
        let end = if b == u64::MAX {
            Key::new(oid, item::EXTENT + 1, 0)
        } else {
            Key::new(oid, item::EXTENT, b)
        };
        let mut out = Vec::new();
        let blocks = self.s.sb.block_count;
        let mut bad = false;
        btree::range(&mut self.s, root, &start, &end, &mut |k, raw| {
            let e = Extent::decode(raw)?;
            // Extent (i kopie) musi lezet ve svazku (jinak by cteni
            // i uvolneni slo mimo; uvolneni by navic poskodilo alokator).
            let within = |p: u64| p >= crate::layout::DATA_START && p.checked_add(e.plen()).is_some_and(|x| x <= blocks);
            let inside = within(e.phys) && (e.copy == 0 || within(e.copy)) && k.offset.checked_add(e.len()).is_some();
            if !inside {
                bad = true;
                return Ok(false);
            }
            if k.offset + e.len() > a {
                out.push((k.offset, e));
            }
            Ok(out.len() < limit)
        })?;
        if bad {
            return Err(Error::Corrupted);
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
            let data = self.inline_data(root, oid, i.size)?;
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
            let buf = self.extent_data(&e, lo - ks, hi - lo)?;
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
        let oid = self.enter(path)?;
        let root = self.s.root;
        let i = self.inode(&root, oid)?;
        if i.kind != KIND_FILE {
            return Err(not_file(i.kind));
        }
        Ok((oid, i))
    }

    /// Zapise `data` od `offset` (soubor se pripadne zvetsi, diru vyplni nuly).
    ///
    /// Vysledek rika, zda byla data **prijata** (do bufferu; hned je vidi
    /// cteni a ulozi je pristi commit nebo fsync), ne zda jsou **trvala** -
    /// to potvrzuje az `fsync`/`commit`.
    ///
    /// - `Ok` = prijata cela.
    /// - Chyba = neprijato nic z tohoto volani (stav souboru jako pred nim).
    ///
    /// Kdyz buffer preroste `DIRTY_LIMIT`, zkusi se zapsat na disk; jeho
    /// chyba zapis neodmitne (data zustanou v bufferu a zkusi se znovu).
    /// Az kdyz zapis na disk selhava opakovane a buffer dosahne
    /// `DIRTY_HARD_LIMIT`, dalsi data se neprijmou.
    pub fn write(&mut self, path: &str, offset: u64, data: &[u8]) -> Result<()> {
        if self.checking() {
            self.require(path, crate::perm::rights::WRITE)?;
        }
        let (oid, inode) = self.file_oid(path)?;
        let end = offset.saturating_add(data.len() as u64);
        self.check_path_access(oid, super::handles::access::WRITE, offset, end)?;
        self.write_oid(oid, inode, offset, data)
    }

    /// Zapis do souboru `oid` aktivniho datasetu (cesta i handle).
    pub(super) fn write_oid(&mut self, oid: u64, inode: Inode, offset: u64, data: &[u8]) -> Result<()> {
        if data.is_empty() {
            return Ok(());
        }
        let end = offset
            .checked_add(data.len() as u64)
            .filter(|&e| e <= MAX_FILE_SIZE)
            .ok_or(Error::OutOfRange)?;
        let blocks = end.div_ceil(BS) - offset / BS;
        self.ensure_room(blocks + 8)?;
        if self.dirty_blocks + blocks > DIRTY_HARD_LIMIT {
            // Pred prijetim: kdyz se buffer neda vyprazdnit, nic neprijmout.
            self.for_each_dataset(|fs| fs.flush_all_data())?;
        }
        let new_size = inode.size.max(end);
        self.guarded(|fs| fs.set_content(oid, inode, new_size, Some((offset, data))))?;
        // Data jsou prijata. Selhany odpis bufferu je cisty (data se vrati do
        // bufferu) a nahlasi se az pri fsync/commit; jen kdyz po nem svazek
        // presel do stavu chyby (nic uz se neulozi), vrati se chyba.
        if self.dirty_blocks > DIRTY_LIMIT
            && let Err(e) = self.for_each_dataset(|fs| fs.flush_all_data())
            && self.s.poisoned
        {
            return Err(e);
        }
        Ok(())
    }

    /// Zmeni velikost souboru (zvetseni doplni nuly).
    pub fn truncate(&mut self, path: &str, size: u64) -> Result<()> {
        if self.checking() {
            self.require(path, crate::perm::rights::WRITE)?;
        }
        self.guarded(|fs| {
            let (oid, inode) = fs.file_oid(path)?;
            fs.check_path_access(oid, super::handles::access::WRITE, size.min(inode.size), u64::MAX)?;
            fs.truncate_checked(oid, inode, size)
        })
    }

    /// Zmena velikosti souboru `oid` aktivniho datasetu (cesta i handle).
    pub(super) fn truncate_checked(&mut self, oid: u64, inode: Inode, size: u64) -> Result<()> {
        self.guarded(|fs| {
            if size > MAX_FILE_SIZE {
                return Err(Error::OutOfRange);
            }
            if size == inode.size {
                return Ok(());
            }
            // Zkraceni misto uvolnuje: smi do poloviny rezervy.
            if size < inode.size {
                fs.ensure_room_freeing(8)?;
            } else {
                fs.ensure_room(8)?;
            }
            fs.truncate_oid(oid, size)
        })
    }

    pub(super) fn truncate_oid(&mut self, oid: u64, size: u64) -> Result<()> {
        let inode = self.live_inode(oid)?;
        self.set_content(oid, inode, size, None)?;
        self.zil_op(zil::Op::Truncate {
            oid,
            size,
            time: self.now,
        });
        Ok(())
    }

    /// Obsah maleho souboru; delka musi presne odpovidat velikosti
    /// v zaznamu (jinak `CORRUPTED`, ne cteni mimo nebo tiche zkraceni).
    pub(super) fn inline_data(&mut self, root: &Child, oid: u64, size: u64) -> Result<Vec<u8>> {
        let data = btree::get(&mut self.s, root, &Key::new(oid, item::INLINE, 0))?.unwrap_or_default();
        if data.len() as u64 != size {
            return Err(Error::Corrupted);
        }
        Ok(data)
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
                    block = self.extent_data(&e, lb - ks, 1)?;
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
            self.mutations += 1;
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
                let mut data = self.inline_data(&root, oid, old_size)?;
                data.resize(new_size as usize, 0);
                if let Some((off, bytes)) = write {
                    data[off as usize..off as usize + bytes.len()].copy_from_slice(bytes);
                }
                self.store_inline(oid, data)?;
            }
            (true, false) => {
                let data = self.inline_data(&root, oid, old_size)?;
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
        inode.version = inode.version.wrapping_add(1);
        self.put_inode(oid, &inode)?;
        self.zil.touch(oid);
        self.jnote_content(oid);
        let parent = inode.parent;
        self.adjust_tree(parent, new_size as i128 - old_size as i128, 0, 0)
    }

    /// Pocet souvislych fyzickych useku souboru (1 = nefragmentovany).
    pub fn fragments(&mut self, path: &str) -> Result<u64> {
        self.require(path, crate::perm::rights::READ)?;
        let (oid, _) = self.file_oid(path)?;
        let root = self.s.root;
        let mut runs = 0;
        let mut next = None;
        let mut from = 0;
        loop {
            let batch = self.extents_batch(&root, oid, from, u64::MAX, 256)?;
            for (_, e) in &batch {
                if next != Some(e.phys) {
                    runs += 1;
                }
                next = Some(e.phys + e.len());
            }
            match batch.last() {
                Some((ks, e)) if batch.len() == 256 => from = ks + e.len(),
                _ => return Ok(runs),
            }
        }
    }
}
