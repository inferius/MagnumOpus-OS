//! Nastaveni dat datasetu a predalokace (faze 3).
//!
//! **Nastaveni** (`DataSettings`): komprese (vychozi vypnuto - na rychlem
//! NVMe by LZ4 v jednom vlakne zpomalila dlouhy zapis az nekolikrat),
//! pocet kopii dat (1 nebo 2) a deduplikace (vychozi na pozadi). Zmena
//! plati pro nove zapsana data; stara zustanou, jak jsou. Novy dataset
//! zdedi nastaveni rodicovskeho.
//!
//! **Predalokace** (`fallocate`): diry v rozsahu dostanou souvisle bloky
//! jako nezapsane extenty (ctou se jako nuly). Prvni zapis do nich jde na
//! misto (viz `blocks.rs`), takze databaze nebo stahovani maji zarucene
//! misto i souvisly soubor.

use alloc::vec::Vec;

use abi::{Error, Result};

use super::Fs;
use crate::btree;
use crate::datarec::DataSettings;
use crate::device::BlockDevice;
use crate::layout::{BLOCK_SIZE, Extent, INLINE_MAX, Key, MAX_EXTENT, MAX_FILE_SIZE, META_OID, item};
use crate::space::Class;

const BS: u64 = BLOCK_SIZE as u64;

impl<D: BlockDevice> Fs<D> {
    /// Nastaveni dat datasetu `dataset` (cesta jeho korene).
    pub fn data_settings_of(&mut self, dataset: &str) -> Result<DataSettings> {
        self.require(dataset, crate::perm::rights::READ)?;
        self.enter_dataset(dataset)?;
        self.data_settings()
    }

    /// Zmeni nastaveni dat datasetu (vyzaduje `MANAGE`). Hned commituje.
    pub fn set_data_settings(&mut self, dataset: &str, s: &DataSettings) -> Result<()> {
        s.validate()?;
        self.require(dataset, crate::perm::rights::MANAGE)?;
        self.guarded(|fs| {
            fs.enter_dataset(dataset)?;
            fs.ensure_room(4)?;
            // Rozpracovana data jeste podle stareho nastaveni.
            fs.flush_all_data()?;
            fs.put(Key::new(META_OID, item::DATAOPT, 0), s.encode())?;
            let id = fs.cur_ds;
            fs.dcfg.insert(id, *s);
            fs.commit()
        })
    }

    /// Novy dataset `child` zdedi nastaveni dat aktivniho datasetu (rodice).
    pub(super) fn inherit_data_settings(&mut self, child: u64) -> Result<()> {
        let root = self.s.root;
        let Some(v) = btree::get(&mut self.s, &root, &Key::new(META_OID, item::DATAOPT, 0))? else {
            return Ok(());
        };
        let s = DataSettings::decode(&v)?;
        let back = self.cur_ds;
        self.activate(child)?;
        let r = self.put(Key::new(META_OID, item::DATAOPT, 0), s.encode());
        self.dcfg.insert(child, s);
        self.activate(back)?;
        r
    }

    /// Predalokuje `len` bajtu souboru `path` od `offset` (jako fallocate).
    /// Bloky, kde uz data jsou, zustanou. `keep_size` = velikost souboru
    /// se nemeni (misto za koncem pro pozdejsi zapis), jinak se soubor
    /// zvetsi na `offset + len`. Vyzaduje pravo zapisu. Hned commituje.
    pub fn fallocate(&mut self, path: &str, offset: u64, len: u64, keep_size: bool) -> Result<()> {
        if len == 0 {
            return Err(Error::InvalidArgs);
        }
        let end = offset.checked_add(len).filter(|&e| e <= MAX_FILE_SIZE).ok_or(Error::OutOfRange)?;
        self.require(path, crate::perm::rights::WRITE)?;
        let r = self.guarded(|fs| {
            let (oid, inode) = fs.file_oid(path)?;
            fs.check_path_access(oid, super::handles::access::WRITE, offset, end)?;
            let new_size = if keep_size { inode.size } else { inode.size.max(end) };
            // Maly soubor ve strome: predalokovat jde az soubor s bloky.
            if new_size as usize <= INLINE_MAX {
                if new_size != inode.size {
                    fs.set_content(oid, inode, new_size, None)?;
                }
                return Ok(());
            }
            let (a, b) = (offset / BS, end.div_ceil(BS));
            fs.ensure_room(b - a + 16)?;
            if new_size != inode.size || (inode.size as usize) <= INLINE_MAX {
                fs.set_content(oid, inode, new_size, None)?;
            }
            fs.writeback(oid, None)?;
            fs.prealloc_holes(oid, a, b)
        });
        if r.is_ok() {
            self.commit()?;
        }
        r
    }

    /// Diry souboru `oid` v logickych blocich `a..b` vyplni nezapsanymi
    /// extenty.
    fn prealloc_holes(&mut self, oid: u64, a: u64, b: u64) -> Result<()> {
        let root = self.s.root;
        let mut holes: Vec<(u64, u64)> = Vec::new();
        let mut pos = a;
        for (ks, e) in self.extents_in(&root, oid, a, b)? {
            if ks > pos {
                holes.push((pos, ks.min(b)));
            }
            pos = pos.max(ks + e.len());
        }
        if pos < b {
            holes.push((pos, b));
        }
        let copies = self.data_settings()?.copies;
        let txg = self.s.txg;
        for (lo, hi) in holes {
            let mut goal = if lo > 0 {
                let root = self.s.root;
                self.extents_in(&root, oid, lo - 1, lo)?.into_iter().next().map(|(_, e)| e.phys + e.plen())
            } else {
                None
            };
            let mut lb = lo;
            while lb < hi {
                let want = (hi - lb).min(MAX_EXTENT);
                let (start, got) = self.s.alloc_data(want, goal)?;
                let copy = if copies > 1 {
                    match self.s.space.alloc_min(got, got, Class::MetaHigh, txg) {
                        Ok((c, _)) => c,
                        Err(e) => {
                            self.s.space.free(start, got, txg, true);
                            return Err(e);
                        }
                    }
                } else {
                    0
                };
                let mut e = Extent::unwritten(start, txg, got as u32);
                e.copy = copy;
                self.put(Key::new(oid, item::EXTENT, lb), e.encode())?;
                lb += got;
                goal = Some(start + got);
            }
        }
        self.s.mark_changed();
        Ok(())
    }
}
