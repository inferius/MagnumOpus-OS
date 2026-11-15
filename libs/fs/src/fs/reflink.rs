//! Okamzite kopie (reflink, faze 3): kopie ukazuje na stejne bloky.
//!
//! Extent kopie dostane priznak SHARED a generaci vzniku teto transakce;
//! tabulka sdileni (`sharing.rs`) pricte odkaz. Zapis do kopie jde jako vzdy
//! do novych bloku, takze original se nezmeni. Kopie je hotova v case
//! umernem poctu extentu, ne velikosti dat.
//!
//! - nezapsany (predalokovany) extent = dira v kopii,
//! - komprimovany kus se sdili jen cely; cast kusu se zkopiruje (rozbali),
//! - zdroj musi byt ulozeny (generace extentu < tato transakce): zivy
//!   zdroj se pred kopii commitne - jinak by se jeho bloky z teto transakce
//!   smely uvolnit hned, i kdyz na ne uz ukazuje kopie.
//!
//! Kopie funguji v celem svazku (i mezi datasety); sifrovane trezory
//! (faze 4) kopiruji normalne. Operace se na konci commitne (do intent
//! logu se nezapisuje).

use alloc::string::String;

use abi::{Error, Result};

use super::{Fs, Kind, split_parent};
use crate::device::BlockDevice;
use crate::layout::{BLOCK_SIZE, EXTENT_SHARED, INLINE_MAX, KIND_FILE, Key, item};
use crate::node::Child;

const BS: u64 = BLOCK_SIZE as u64;

/// Odkud se sdili: zivy strom datasetu (koren se muze behem kopie zmenit)
/// nebo pevny ulozeny strom (snapshot).
#[derive(Clone, Copy)]
pub(super) enum Src {
    Live(u64),
    Fixed(Child),
}

/// Je `dst` uvnitr `src` (nebo totez)? Kopie slozky do sebe by se nikdy
/// nedokoncila.
fn inside(src: &str, dst: &str) -> bool {
    let s = src.trim_end_matches('/');
    let d = dst.trim_end_matches('/');
    s.is_empty() || d == s || d.strip_prefix(s).is_some_and(|r| r.starts_with('/'))
}

impl<D: BlockDevice> Fs<D> {
    fn src_root(&mut self, src: Src) -> Result<Child> {
        match src {
            Src::Live(ds) => self.tree_of(ds),
            Src::Fixed(c) => Ok(c),
        }
    }

    /// Namapuje logicke bloky `a..b` souboru `src` do souboru `dst`
    /// aktivniho datasetu od bloku `to` (stare mapovani `dst` v rozsahu se
    /// uvolni). Velikost `dst` neresi.
    pub(super) fn share_blocks(&mut self, src: Src, soid: u64, a: u64, b: u64, dst: u64, to: u64) -> Result<()> {
        let back = self.cur_ds;
        self.punch_range(dst, to, to + (b - a))?;
        // Rozpracovane bloky cile v rozsahu prepisuje kopie.
        if let Some(fd) = self.data.get_mut(&dst) {
            let drop: alloc::vec::Vec<u64> = fd.blocks.range(to..to + (b - a)).map(|(&k, _)| k).collect();
            for k in drop {
                fd.blocks.remove(&k);
                self.dirty_blocks -= 1;
            }
        }
        let txg = self.s.txg;
        let mut from = a;
        while from < b {
            let root = self.src_root(src)?;
            let batch = self.extents_batch(&root, soid, from, b, 256)?;
            let full = batch.len() == 256;
            let next = batch.last().map(|(ks, e)| ks + e.len());
            for (ks, e) in batch {
                let lo = ks.max(a);
                let hi = (ks + e.len()).min(b);
                if lo >= hi || e.is_unwritten() {
                    continue;
                }
                let at = to + (lo - a);
                let whole = lo == ks && hi == ks + e.len();
                if e.birth >= txg || (e.is_compressed() && !whole) {
                    // Nesdilitelne: zkopirovat do bufferu cile.
                    let data = self.extent_data(&e, lo - ks, hi - lo)?;
                    self.activate(back)?;
                    let fd = self.data.entry(dst).or_default();
                    for (j, blk) in data.chunks(BLOCK_SIZE).enumerate() {
                        if fd.blocks.insert(at + j as u64, blk.to_vec()).is_none() {
                            self.dirty_blocks += 1;
                        }
                        self.zil.touch_block(dst, at + j as u64);
                    }
                    continue;
                }
                let mut n = if e.is_compressed() { e.clone() } else { e.slice(lo - ks, hi - ks).ok_or(Error::Corrupted)? };
                n.birth = txg;
                n.flags |= EXTENT_SHARED;
                self.share_ref(n.phys, n.plen(), n.copy)?;
                self.activate(back)?;
                self.put(Key::new(dst, item::EXTENT, at), n.encode())?;
            }
            self.activate(back)?;
            match next {
                Some(n) if full => from = n.max(from + 1),
                _ => break,
            }
        }
        self.activate(back)?;
        self.s.mark_changed();
        Ok(())
    }

    /// Obsah souboru `soid` (zdroj `src`) do noveho prazdneho souboru `dst`
    /// aktivniho datasetu sdilenim bloku; maly soubor (ve strome) se
    /// zkopiruje. `keep_mtime` = cas zmeny obsahu jako u zdroje.
    pub(super) fn share_file(&mut self, src: Src, soid: u64, dst: u64, keep_mtime: bool) -> Result<u64> {
        let back = self.cur_ds;
        let root = self.src_root(src)?;
        let si = self.inode(&root, soid)?;
        if si.kind != KIND_FILE {
            return Err(Error::WrongType);
        }
        self.activate(back)?;
        if si.size as usize <= INLINE_MAX {
            let data = self.read_oid(&root, soid, 0, si.size as usize, false)?;
            let inode = self.live_inode(dst)?;
            self.set_content(dst, inode, si.size, (!data.is_empty()).then_some((0, data.as_slice())))?;
        } else {
            self.ensure_room(16)?;
            let inode = self.live_inode(dst)?;
            self.set_content(dst, inode, si.size, None)?;
            self.share_blocks(src, soid, 0, si.size.div_ceil(BS), dst, 0)?;
        }
        if keep_mtime {
            self.set_times_oid(dst, super::SetTimes { mtime: Some(si.mtime), btime: None, atime: None })?;
        }
        Ok(si.size)
    }

    /// Okamzita kopie souboru, odkazu nebo cele slozky `src` na novou cestu
    /// `dst` (datasety uvnitr slozky se preskoci - maji vlastni historii).
    /// Prava: cist zdroj (u slozky vypsat, kazdy soubor cist), vytvorit
    /// v cili. Vraci pocet sdilenych bajtu.
    pub fn clone_path(&mut self, src: &str, dst: &str) -> Result<u64> {
        if inside(src, dst) {
            return Err(Error::InvalidArgs);
        }
        split_parent(dst)?;
        // Zdroj musi byt ulozeny (viz hlavicka modulu).
        self.commit()?;
        let r = self.guarded(|fs| fs.clone_rec(src, dst));
        if r.is_ok() {
            self.commit()?;
        }
        r
    }

    fn clone_rec(&mut self, src: &str, dst: &str) -> Result<u64> {
        let kind = {
            let oid = self.enter(src)?;
            let root = self.s.root;
            self.inode(&root, oid)?.kind
        };
        match Kind::from_raw(kind) {
            Kind::File => {
                let soid = self.require(src, crate::perm::rights::READ)?;
                let sds = self.cur_ds;
                self.create(dst, Kind::File)?;
                let d = self.enter(dst)?;
                self.share_file(Src::Live(sds), soid, d, false)
            }
            Kind::Symlink => {
                let target = self.readlink(src)?;
                self.symlink(dst, &target)?;
                Ok(0)
            }
            // Kopie uzlu socketu = novy uzel (bez posluchace, jako cp -a).
            Kind::Socket => {
                self.create(dst, Kind::Socket)?;
                Ok(0)
            }
            Kind::Dir => {
                let entries = self.list(src)?;
                self.create(dst, Kind::Dir)?;
                let mut total = 0;
                for e in entries {
                    if e.kind == Kind::Dataset {
                        continue;
                    }
                    total += self.clone_rec(&join(src, &e.name), &join(dst, &e.name))?;
                }
                Ok(total)
            }
            Kind::Dataset => Err(Error::InvalidArgs),
        }
    }

    /// Okamzita kopie rozsahu: `len` bajtu souboru `src` od `src_off` do
    /// souboru `dst` od `dst_off` (jako FICLONERANGE). Posuny a delka po
    /// celych blocich (4 KiB); delka smi koncit na konci zdroje. Cil se
    /// pripadne zvetsi. Prava: cist zdroj, zapsat do cile.
    pub fn clone_range(&mut self, src: &str, src_off: u64, dst: &str, dst_off: u64, len: u64) -> Result<()> {
        if !src_off.is_multiple_of(BS) || !dst_off.is_multiple_of(BS) {
            return Err(Error::InvalidArgs);
        }
        self.commit()?;
        let r = self.guarded(|fs| {
            let soid = fs.require(src, crate::perm::rights::READ)?;
            let sds = fs.cur_ds;
            let root = fs.s.root;
            let si = fs.inode(&root, soid)?;
            if si.kind != KIND_FILE {
                return Err(Error::WrongType);
            }
            let end = src_off.checked_add(len).ok_or(Error::OutOfRange)?.min(si.size);
            if !len.is_multiple_of(BS) && src_off + len < si.size {
                return Err(Error::InvalidArgs);
            }
            if end <= src_off {
                return Ok(());
            }
            let bytes = end - src_off;
            let doid = fs.require(dst, crate::perm::rights::WRITE)?;
            let (_, di) = fs.file_oid(dst)?;
            let dend = dst_off.checked_add(bytes).filter(|&e| e <= crate::layout::MAX_FILE_SIZE).ok_or(Error::OutOfRange)?;
            fs.check_path_access(doid, super::handles::access::WRITE, dst_off, dend)?;
            if sds == fs.cur_ds && soid == doid {
                let (a, b) = (src_off, end);
                let (c, d) = (dst_off, dend);
                if a < d && c < b {
                    return Err(Error::InvalidArgs);
                }
            }
            // Maly zdroj: kopie dat.
            if si.size as usize <= INLINE_MAX {
                let data = fs.read_oid(&root, soid, src_off, bytes as usize, false)?;
                let di = fs.live_inode(doid)?;
                return fs.write_oid(doid, di, dst_off, &data);
            }
            // Kopie konci uprostred bloku (konec zdroje) a cil za ni
            // pokracuje: ocas bloku by se prepsal nulami ze zdroje.
            if !bytes.is_multiple_of(BS) && dend < di.size {
                return Err(Error::InvalidArgs);
            }
            fs.ensure_room(16)?;
            // Cil do podoby s bloky a jeho rozpracovana data na disk, aby je
            // pozdejsi zapis bufferu nepreplacl pres kopii.
            let new_size = di.size.max(dend);
            fs.set_content(doid, di, new_size, None)?;
            fs.writeback(doid, None)?;
            let blocks = bytes.div_ceil(BS);
            fs.share_blocks(Src::Live(sds), soid, src_off / BS, src_off / BS + blocks, doid, dst_off / BS)
        });
        if r.is_ok() {
            self.commit()?;
        }
        r
    }
}

fn join(dir: &str, name: &str) -> String {
    let mut p = String::from(dir.trim_end_matches('/'));
    p.push('/');
    p.push_str(name);
    p
}
