//! Rozdil dvou verzi datasetu a obnova ze snapshotu jako kopie.
//!
//! **Diff** porovna dva stromy datasetu (snapshot proti snapshotu, nebo
//! snapshot proti zive verzi). Objekt (oid) je v datasetu stabilni, takze
//! presun nebo prejmenovani se pozna presne - ne heuristikou podle obsahu
//! jako v gitu. Pro kazdy objekt se porovna:
//! - kde lezi: (rodic, jmeno) z polozek adresaru,
//! - obsah: velikost a otisk polozek s daty (extenty, maly soubor, cil odkazu).
//!
//! Porovnani preskakuje podstromy se stejnym ukazatelem a zmenene objekty
//! dohledava bodovymi dotazy (cena podle velikosti zmeny, viz `delta.rs`).
//!
//! **Obnova jako kopie** precte soubor (nebo celou slozku) ze snapshotu
//! a zapise ho do zive verze pod novym jmenem vedle originalu - obycejna
//! kopie dat. Okamzita kopie pres sdilene bloky prijde se sdilenim bloku
//! souboru (faze 3).

use alloc::string::String;
use alloc::vec::Vec;

use abi::{Error, Result};

use super::{Fs, Kind};
use crate::btree;
use crate::device::BlockDevice;
use crate::layout::{BLOCK_SIZE, Key, META_OID, item};
use crate::node::Child;

/// Druh zmeny objektu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeKind {
    Added,
    Removed,
    /// Zmenil se obsah (u souboru data, u odkazu cil). U adresare se
    /// nehlasi - zmeny adresare jsou zmeny jeho polozek.
    Modified,
    /// Presunuty nebo prejmenovany (jina cesta, tentyz objekt).
    Moved,
    /// Presunuty a zaroven zmeneny obsah.
    MovedModified,
}

/// Jedna zmena v rozdilu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    pub kind: ChangeKind,
    /// Cesta v novejsi verzi (u `Removed` cesta ve starsi).
    pub path: String,
    /// U presunu cesta ve starsi verzi.
    pub old_path: Option<String>,
    pub oid: u64,
    pub is_dir: bool,
}

impl<D: BlockDevice> Fs<D> {
    /// Rozdil dvou verzi datasetu `dataset`: ze snapshotu `from` do
    /// snapshotu `to`, nebo do zive verze (`to = None`). Serazeno podle cesty.
    /// S kontrolou opravneni vyzaduje `MANAGE` ke koreni datasetu (rozdil
    /// ukazuje jmena vsech objektu).
    pub fn diff(&mut self, dataset: &str, from: &str, to: Option<&str>) -> Result<Vec<Change>> {
        self.require(dataset, crate::perm::rights::MANAGE)?;
        self.enter_dataset(dataset)?;
        let old_root = Child::Disk(self.find_snapshot(from)?.1.root);
        let new_root = match to {
            Some(name) => Child::Disk(self.find_snapshot(name)?.1.root),
            None => {
                // Data v bufferu jeste nejsou v extentech: zapsat je (bez
                // commitu), aby diff videl i neulozene zmeny.
                if !self.s.read_only {
                    self.flush_all_data()?;
                }
                self.s.root
            }
        };
        self.diff_roots(&old_root, &new_root)
    }

    /// Rozdil zapisovatelneho klonu `clone` proti snapshotu, ze ktereho
    /// vznikl (zakladni krok funkce "zkusit zmeny bokem": co klon zmenil).
    pub fn diff_clone(&mut self, clone: &str) -> Result<Vec<Change>> {
        self.require(clone, crate::perm::rights::MANAGE)?;
        self.enter_dataset(clone)?;
        let (ods, osnap, _) = self.cur_origin;
        if ods == 0 {
            return Err(Error::InvalidArgs);
        }
        if !self.s.read_only {
            self.flush_all_data()?;
        }
        let new_root = self.s.root;
        self.activate(ods)?;
        let key = Key::new(META_OID, item::SNAPSHOT, osnap);
        let root = self.s.root;
        let raw = btree::get(&mut self.s, &root, &key)?.ok_or(Error::Corrupted)?;
        let old_root = Child::Disk(crate::layout::SnapshotRecord::decode(&raw)?.root);
        self.diff_roots(&old_root, &new_root)
    }

    /// Obnovi soubor, odkaz nebo celou slozku `path` ze snapshotu `snapshot`
    /// datasetu `dataset` jako kopii `dest` v zive verzi (vedle originalu).
    /// `dest` nesmi existovat. Vraci pocet zkopirovanych bajtu.
    pub fn restore_copy(&mut self, dataset: &str, snapshot: &str, path: &str, dest: &str) -> Result<u64> {
        let st = self.snapshot_stat_of(dataset, snapshot, path)?;
        match st.kind {
            Kind::File => {
                // Pravo cist zdroj (a ze snapshotu je videt) overi tento krok.
                self.enter_dataset(dataset)?;
                let root = Child::Disk(self.find_snapshot(snapshot)?.1.root);
                let src = self.snapshot_object(dataset, &root, path, crate::perm::rights::READ)?;
                self.create(dest, Kind::File)?;
                let dst = self.enter(dest)?;
                self.copy_content(&root, src, dst)?;
                Ok(st.size)
            }
            Kind::Symlink => {
                let target = self.snapshot_readlink_of(dataset, snapshot, path)?;
                self.symlink(dest, &target)?;
                Ok(0)
            }
            Kind::Socket => {
                self.create(dest, Kind::Socket)?;
                Ok(0)
            }
            Kind::Dir => {
                self.create(dest, Kind::Dir)?;
                let mut total = 0;
                for e in self.snapshot_list_of(dataset, snapshot, path)? {
                    if e.kind == Kind::Dataset {
                        // Vnoreny dataset ma vlastni historii: neobnovuje se.
                        continue;
                    }
                    let from = join(path, &e.name);
                    let to = join(dest, &e.name);
                    total += self.restore_copy(dataset, snapshot, &from, &to)?;
                }
                Ok(total)
            }
            Kind::Dataset => Err(Error::InvalidArgs),
        }
    }

    /// Obsah souboru `src` ze stromu `root` do souboru `dst` aktivniho
    /// datasetu (prazdny, prave vytvoreny): data po 1 MiB, ridky konec jako
    /// v originalu, cas zmeny obsahu zachovany. Spolecne pro obnovu ze
    /// snapshotu a merge.
    pub(super) fn copy_content(&mut self, root: &Child, src: u64, dst: u64) -> Result<()> {
        // Ulozeny strom (snapshot, commit): okamzita kopie sdilenim bloku.
        if matches!(root, Child::Disk(_)) {
            self.share_file(super::reflink::Src::Fixed(*root), src, dst, true)?;
            return Ok(());
        }
        let i = self.inode(root, src)?;
        let mut off = 0u64;
        while off < i.size {
            let n = ((i.size - off) as usize).min(256 * BLOCK_SIZE);
            let d = self.read_oid(root, src, off, n, false)?;
            if d.is_empty() {
                return Err(Error::Corrupted);
            }
            let inode = self.live_inode(dst)?;
            self.write_oid(dst, inode, off, &d)?;
            off += d.len() as u64;
        }
        if off < i.size {
            self.truncate_oid(dst, i.size)?;
        }
        self.set_times_oid(dst, super::SetTimes { mtime: Some(i.mtime), btime: None, atime: None })
    }

    /// Cil odkazu ve snapshotu.
    pub fn snapshot_readlink_of(&mut self, dataset: &str, snapshot: &str, path: &str) -> Result<String> {
        self.enter_dataset(dataset)?;
        let root = Child::Disk(self.find_snapshot(snapshot)?.1.root);
        let oid = self.snapshot_object(dataset, &root, path, crate::perm::rights::READ)?;
        self.readlink_oid(&root, oid)
    }
}

fn join(dir: &str, name: &str) -> String {
    let mut p = String::from(dir.trim_end_matches('/'));
    p.push('/');
    p.push_str(name);
    p
}
