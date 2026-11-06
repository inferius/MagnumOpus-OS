//! Souborovy system: cesty, soubory, adresare, snapshoty, intent log.
//!
//! Zmeny se hromadi v pameti: metadata v rozpracovanych uzlech stromu, data
//! souboru v bufferu (odlozena alokace - bloky dostanou az pri zapisu, takze
//! lezi souvisle). `commit` vse ulozi atomicky; `fsync` udela zmeny trvalymi
//! rychle pres intent log (jeden zapis + flush), commit pak muze probihat
//! zridka. Cteni ze snapshotu pouziva stejny kod nad jinym korenem.
//!
//! Podmoduly: `data` (obsah souboru, extenty), `snap` (snapshoty, deadlisty),
//! `zil` (intent log), `defrag` (defragmentace), `check` (kontrola
//! konzistence).

mod check;
mod data;
mod defrag;
mod snap;
mod zil;

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;

use abi::{Error, Result};

use crate::btree;
use crate::device::BlockDevice;
use crate::hash::Algo;
use crate::layout::{
    BLOCK_SIZE, Dirent, Inode, KIND_DIR, KIND_FILE, Key, ROOT_OID, decode_dirents, encode_dirents,
    item,
};
use crate::names::{fold_name, name_hash, stored_name, validate_name};
use crate::node::Child;
use crate::store::{FormatOptions, Store};

pub use check::CheckReport;
pub use defrag::DefragReport;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    File,
    Dir,
}

impl Kind {
    fn from_raw(k: u8) -> Self {
        if k == KIND_DIR { Kind::Dir } else { Kind::File }
    }

    fn raw(self) -> u8 {
        match self {
            Kind::File => KIND_FILE,
            Kind::Dir => KIND_DIR,
        }
    }
}

/// Metadata objektu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stat {
    pub oid: u64,
    pub kind: Kind,
    pub size: u64,
    pub mtime: u64,
    pub ctime: u64,
    /// U adresare: soucet velikosti vsech souboru v podstromu.
    pub tree_bytes: u64,
    /// U adresare: pocet souboru v podstromu.
    pub tree_files: u64,
    /// U adresare: pocet adresaru v podstromu (bez sebe).
    pub tree_dirs: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirEntry {
    pub name: String,
    pub kind: Kind,
    pub oid: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FsInfo {
    pub label: String,
    pub block_size: u32,
    pub block_count: u64,
    /// Volne bloky (vcetne tech, ktere cekaji na odlozene uvolneni).
    pub free_blocks: u64,
    /// Z toho cekajici na odlozene uvolneni.
    pub deferred_blocks: u64,
    pub generation: u64,
    pub algo: Algo,
    pub ag_blocks: u64,
    /// O kolik generaci se pripojeni vratilo kvuli poskozenemu stavu (0 = ne).
    pub fallback: u64,
    /// Kolik operaci se pri pripojeni prehralo z intent logu.
    pub replayed: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotInfo {
    pub id: u64,
    pub name: String,
    pub generation: u64,
}

/// Rozpracovana data jednoho souboru (odlozena alokace).
#[derive(Default)]
struct FileDirty {
    /// Logicky blok -> obsah (BLOCK_SIZE bajtu).
    blocks: BTreeMap<u64, Vec<u8>>,
}

pub struct Fs<D: BlockDevice> {
    s: Store<D>,
    now: u64,
    /// Rozpracovana data souboru podle oid.
    data: BTreeMap<u64, FileDirty>,
    dirty_blocks: u64,
    zil: zil::Pending,
    replayed: u64,
    /// Kde pokracuje defragmentace na pozadi (oid).
    defrag_cursor: u64,
}

/// Kolik rozpracovanych bloku dat drzet v pameti, nez se zapisi (16 MiB).
const DIRTY_LIMIT: u64 = 4096;

fn split_path(path: &str) -> impl Iterator<Item = &str> {
    path.split('/').filter(|c| !c.is_empty())
}

/// (cesta rodice, jmeno posledni slozky)
fn split_parent(path: &str) -> Result<(&str, &str)> {
    if !path.starts_with('/') {
        return Err(Error::InvalidArgs);
    }
    let trimmed = path.trim_end_matches('/');
    let i = trimmed.rfind('/').ok_or(Error::InvalidArgs)?;
    let name = &trimmed[i + 1..];
    if name.is_empty() {
        return Err(Error::InvalidArgs);
    }
    let parent = &trimmed[..i];
    Ok((if parent.is_empty() { "/" } else { parent }, name))
}

impl<D: BlockDevice> Fs<D> {
    /// Vytvori prazdny souborovy system na zarizeni.
    pub fn format(dev: D, opts: &FormatOptions) -> Result<D> {
        Store::format(dev, opts)
    }

    /// Pripoji souborovy system: stav po poslednim commitu a prehrani
    /// intent logu (zmeny potvrzene `fsync`).
    pub fn mount(dev: D) -> Result<Self> {
        let s = Store::open(dev)?;
        let mut fs = Self {
            s,
            now: 0,
            data: BTreeMap::new(),
            dirty_blocks: 0,
            zil: zil::Pending::default(),
            replayed: 0,
            defrag_cursor: 0,
        };
        fs.replay_log()?;
        Ok(fs)
    }

    /// Odpoji system bez commitu a vrati zarizeni (testy, nastroje).
    pub fn into_device(self) -> D {
        self.s.dev
    }

    /// Cas pro nasledujici zmeny (ns; zdroj voli volajici).
    pub fn set_time(&mut self, now: u64) {
        self.now = now;
    }

    /// Posilat zarizeni TRIM pro definitivne uvolnene bloky.
    pub fn set_discard(&mut self, on: bool) {
        self.s.discard = on;
    }

    /// Ulozi vsechny zmeny atomicky.
    pub fn commit(&mut self) -> Result<()> {
        self.s.check_writable()?;
        self.flush_all_data()?;
        self.flush_dead()?;
        self.s.commit()?;
        self.zil.clear();
        Ok(())
    }

    pub fn info(&self) -> FsInfo {
        let label = &self.s.sb.label;
        let end = label.iter().position(|&b| b == 0).unwrap_or(label.len());
        FsInfo {
            label: String::from_utf8_lossy(&label[..end]).into(),
            block_size: BLOCK_SIZE as u32,
            block_count: self.s.sb.block_count,
            free_blocks: self.s.free_blocks(),
            deferred_blocks: self.s.space.deferred_blocks(),
            generation: self.s.sb.generation,
            algo: self.s.algo,
            ag_blocks: self.s.sb.ag_blocks,
            fallback: self.s.fallback,
            replayed: self.replayed,
        }
    }

    /// Pocet bloku space logu skupiny `ag` (testy zhusteni).
    #[cfg(test)]
    pub(crate) fn space_log_blocks(&self, ag: usize) -> usize {
        self.s.space.meta.log_blocks(ag)
    }

    /// Zajisti misto pro `blocks` novych bloku; kdyz chybi a jsou bloky
    /// cekajici na odlozene uvolneni, uvolni je commity.
    fn ensure_room(&mut self, blocks: u64) -> Result<()> {
        self.s.check_writable()?;
        let mut tries = 0;
        while !self.s.has_room(self.dirty_blocks + blocks) {
            if self.s.space.deferred_blocks() == 0 || tries > crate::layout::REUSE_DELAY + 1 {
                return Err(Error::NoSpace);
            }
            self.commit()?;
            // Commit bez zmen neposune transakci; prazdna zmena ji posune.
            self.s.mark_changed();
            tries += 1;
        }
        Ok(())
    }

    // --- cteni (nad libovolnym korenem: zivy stav nebo snapshot) ---

    fn inode(&mut self, root: &Child, oid: u64) -> Result<Inode> {
        let raw = btree::get(&mut self.s, root, &Key::new(oid, item::INODE, 0))?
            .ok_or(Error::Corrupted)?;
        Inode::decode(&raw)
    }

    fn live_inode(&mut self, oid: u64) -> Result<Inode> {
        let root = self.s.root;
        self.inode(&root, oid)
    }

    fn lookup_child(&mut self, root: &Child, dir: u64, name: &str) -> Result<Option<Dirent>> {
        let folded = fold_name(name);
        let key = Key::new(dir, item::DIRENT, name_hash(&folded));
        let Some(raw) = btree::get(&mut self.s, root, &key)? else {
            return Ok(None);
        };
        Ok(decode_dirents(&raw)?
            .into_iter()
            .find(|d| fold_name(&d.name) == folded))
    }

    fn resolve(&mut self, root: &Child, path: &str) -> Result<u64> {
        if !path.starts_with('/') {
            return Err(Error::InvalidArgs);
        }
        let mut oid = ROOT_OID;
        for name in split_path(path) {
            if self.inode(root, oid)?.kind != KIND_DIR {
                return Err(Error::NotADirectory);
            }
            oid = self
                .lookup_child(root, oid, name)?
                .ok_or(Error::NotFound)?
                .oid;
        }
        Ok(oid)
    }

    fn stat_oid(&mut self, root: &Child, oid: u64) -> Result<Stat> {
        let i = self.inode(root, oid)?;
        Ok(Stat {
            oid,
            kind: Kind::from_raw(i.kind),
            size: i.size,
            mtime: i.mtime,
            ctime: i.ctime,
            tree_bytes: i.tree_bytes,
            tree_files: i.tree_files,
            tree_dirs: i.tree_dirs,
        })
    }

    fn list_oid(&mut self, root: &Child, oid: u64) -> Result<Vec<DirEntry>> {
        if self.inode(root, oid)?.kind != KIND_DIR {
            return Err(Error::NotADirectory);
        }
        let items = btree::collect(
            &mut self.s,
            root,
            &Key::new(oid, item::DIRENT, 0),
            &Key::new(oid, item::DIRENT + 1, 0),
        )?;
        let mut out = Vec::new();
        for (_, v) in items {
            for d in decode_dirents(&v)? {
                out.push(DirEntry {
                    name: d.name,
                    kind: Kind::from_raw(d.kind),
                    oid: d.oid,
                });
            }
        }
        out.sort_by_cached_key(|e| fold_name(&e.name));
        Ok(out)
    }

    // --- verejne cteni zive verze ---

    pub fn stat(&mut self, path: &str) -> Result<Stat> {
        let root = self.s.root;
        let oid = self.resolve(&root, path)?;
        self.stat_oid(&root, oid)
    }

    pub fn list(&mut self, path: &str) -> Result<Vec<DirEntry>> {
        let root = self.s.root;
        let oid = self.resolve(&root, path)?;
        self.list_oid(&root, oid)
    }

    pub fn read(&mut self, path: &str, offset: u64, len: usize) -> Result<Vec<u8>> {
        let root = self.s.root;
        let oid = self.resolve(&root, path)?;
        self.read_oid(&root, oid, offset, len, true)
    }

    // --- zmeny adresaru (na urovni oid, aby je slo prehrat z logu) ---

    fn put_inode(&mut self, oid: u64, i: &Inode) -> Result<()> {
        btree::insert(&mut self.s, Key::new(oid, item::INODE, 0), i.encode())
    }

    /// Promitne zmenu do souhrnu adresare `dir` a vsech jeho predku.
    fn adjust_tree(&mut self, mut dir: u64, bytes: i64, files: i64, dirs: i64) -> Result<()> {
        loop {
            let mut i = self.live_inode(dir)?;
            i.tree_bytes = i.tree_bytes.wrapping_add_signed(bytes);
            i.tree_files = i.tree_files.wrapping_add_signed(files);
            i.tree_dirs = i.tree_dirs.wrapping_add_signed(dirs);
            self.put_inode(dir, &i)?;
            if dir == ROOT_OID {
                return Ok(());
            }
            dir = i.parent;
        }
    }

    fn add_dirent(&mut self, dir: u64, d: Dirent) -> Result<()> {
        let key = Key::new(dir, item::DIRENT, name_hash(&fold_name(&d.name)));
        let root = self.s.root;
        let mut list = match btree::get(&mut self.s, &root, &key)? {
            Some(raw) => decode_dirents(&raw)?,
            None => Vec::new(),
        };
        list.push(d);
        btree::insert(&mut self.s, key, encode_dirents(&list))
    }

    fn remove_dirent(&mut self, dir: u64, name: &str) -> Result<()> {
        let folded = fold_name(name);
        let key = Key::new(dir, item::DIRENT, name_hash(&folded));
        let root = self.s.root;
        let mut list =
            decode_dirents(&btree::get(&mut self.s, &root, &key)?.ok_or(Error::NotFound)?)?;
        list.retain(|d| fold_name(&d.name) != folded);
        if list.is_empty() {
            btree::remove(&mut self.s, &key)?;
            Ok(())
        } else {
            btree::insert(&mut self.s, key, encode_dirents(&list))
        }
    }

    fn touch_dir(&mut self, dir: u64) -> Result<()> {
        let mut i = self.live_inode(dir)?;
        i.mtime = self.now;
        self.put_inode(dir, &i)
    }

    fn dir_of(&mut self, path: &str) -> Result<u64> {
        let root = self.s.root;
        let oid = self.resolve(&root, path)?;
        if self.inode(&root, oid)?.kind != KIND_DIR {
            return Err(Error::NotADirectory);
        }
        Ok(oid)
    }

    /// Vytvori soubor nebo adresar.
    pub fn create(&mut self, path: &str, kind: Kind) -> Result<()> {
        let (parent_path, name) = split_parent(path)?;
        validate_name(name)?;
        let parent = self.dir_of(parent_path)?;
        self.ensure_room(4)?;
        let oid = self.s.next_oid;
        self.create_at(parent, name, kind, oid)
    }

    fn create_at(&mut self, parent: u64, name: &str, kind: Kind, oid: u64) -> Result<()> {
        let root = self.s.root;
        if self.lookup_child(&root, parent, name)?.is_some() {
            return Err(Error::AlreadyExists);
        }
        self.s.next_oid = self.s.next_oid.max(oid + 1);
        let inode = Inode {
            kind: kind.raw(),
            size: 0,
            mtime: self.now,
            ctime: self.now,
            parent,
            tree_bytes: 0,
            tree_files: 0,
            tree_dirs: 0,
        };
        self.put_inode(oid, &inode)?;
        let name = stored_name(name);
        self.add_dirent(
            parent,
            Dirent {
                oid,
                kind: kind.raw(),
                name: name.clone(),
            },
        )?;
        self.touch_dir(parent)?;
        match kind {
            Kind::File => self.adjust_tree(parent, 0, 1, 0)?,
            Kind::Dir => self.adjust_tree(parent, 0, 0, 1)?,
        }
        self.zil.op(zil::Op::Create {
            parent,
            oid,
            kind: kind.raw(),
            name,
            time: self.now,
        });
        Ok(())
    }

    /// Odstrani soubor nebo prazdny adresar.
    pub fn remove(&mut self, path: &str) -> Result<()> {
        let (parent_path, name) = split_parent(path)?;
        let parent = self.dir_of(parent_path)?;
        self.ensure_room(4)?;
        self.remove_at(parent, name)
    }

    fn remove_at(&mut self, parent: u64, name: &str) -> Result<()> {
        let root = self.s.root;
        let d = self
            .lookup_child(&root, parent, name)?
            .ok_or(Error::NotFound)?;
        let inode = self.inode(&root, d.oid)?;
        if inode.kind == KIND_DIR {
            let first = btree::collect(
                &mut self.s,
                &root,
                &Key::new(d.oid, item::DIRENT, 0),
                &Key::new(d.oid, item::DIRENT + 1, 0),
            )?;
            if !first.is_empty() {
                return Err(Error::NotEmpty);
            }
        } else {
            self.drop_content(d.oid)?;
        }
        btree::remove(&mut self.s, &Key::new(d.oid, item::INODE, 0))?;
        self.remove_dirent(parent, &d.name)?;
        self.touch_dir(parent)?;
        if inode.kind == KIND_DIR {
            self.adjust_tree(parent, 0, 0, -1)?;
        } else {
            self.adjust_tree(parent, -(inode.size as i64), -1, 0)?;
        }
        self.zil.op(zil::Op::Remove {
            parent,
            name: d.name,
            time: self.now,
        });
        self.zil.forget(d.oid);
        Ok(())
    }

    /// Presune nebo prejmenuje polozku. Cil nesmi existovat (krome zmeny
    /// velikosti pismen tehoz jmena).
    pub fn rename(&mut self, from: &str, to: &str) -> Result<()> {
        let (fp, fname) = split_parent(from)?;
        let (tp, tname) = split_parent(to)?;
        validate_name(tname)?;
        let src = self.dir_of(fp)?;
        let dst = self.dir_of(tp)?;
        self.ensure_room(8)?;
        self.rename_at(src, fname, dst, tname)
    }

    fn rename_at(&mut self, src: u64, fname: &str, dst: u64, tname: &str) -> Result<()> {
        let root = self.s.root;
        let d = self
            .lookup_child(&root, src, fname)?
            .ok_or(Error::NotFound)?;
        if let Some(existing) = self.lookup_child(&root, dst, tname)?
            && existing.oid != d.oid
        {
            return Err(Error::AlreadyExists);
        }
        // Adresar nejde presunout do sveho podstromu.
        let mut a = dst;
        loop {
            if a == d.oid {
                return Err(Error::InvalidArgs);
            }
            if a == ROOT_OID {
                break;
            }
            a = self.inode(&root, a)?.parent;
        }
        let mut inode = self.inode(&root, d.oid)?;
        let (bytes, files, dirs) = if inode.kind == KIND_DIR {
            (
                inode.tree_bytes as i64,
                inode.tree_files as i64,
                inode.tree_dirs as i64 + 1,
            )
        } else {
            (inode.size as i64, 1, 0)
        };
        let new_name = stored_name(tname);
        self.remove_dirent(src, &d.name)?;
        self.add_dirent(
            dst,
            Dirent {
                oid: d.oid,
                kind: d.kind,
                name: new_name.clone(),
            },
        )?;
        inode.parent = dst;
        inode.ctime = self.now;
        self.put_inode(d.oid, &inode)?;
        self.touch_dir(src)?;
        self.touch_dir(dst)?;
        if src != dst {
            self.adjust_tree(src, -bytes, -files, -dirs)?;
            self.adjust_tree(dst, bytes, files, dirs)?;
        }
        self.zil.op(zil::Op::Rename {
            src,
            name: d.name,
            dst,
            new_name,
            time: self.now,
        });
        Ok(())
    }
}
