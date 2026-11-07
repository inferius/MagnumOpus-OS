//! Rozdil dvou verzi datasetu a obnova ze snapshotu jako kopie.
//!
//! **Diff** porovna dva stromy datasetu (snapshot proti snapshotu, nebo
//! snapshot proti zive verzi). Objekt (oid) je v datasetu stabilni, takze
//! presun nebo prejmenovani se pozna presne - ne heuristikou podle obsahu
//! jako v gitu. Pro kazdy objekt se porovna:
//! - kde lezi: (rodic, jmeno) z polozek adresaru,
//! - obsah: velikost a otisk polozek s daty (extenty, maly soubor, cil odkazu).
//!
//! Prvni verze projde oba stromy cele (cena podle velikosti datasetu).
//! Planovana optimalizace: preskakovat podstromy se stejnym ukazatelem
//! (cena podle velikosti zmeny).
//!
//! **Obnova jako kopie** precte soubor (nebo celou slozku) ze snapshotu
//! a zapise ho do zive verze pod novym jmenem vedle originalu - obycejna
//! kopie dat. Okamzita kopie pres sdilene bloky prijde se sdilenim bloku
//! souboru (faze 3).

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;

use abi::{Error, Result};

use super::{Fs, Kind};
use crate::btree;
use crate::device::BlockDevice;
use crate::hash::{Csum, xxh3};
use crate::layout::{
    BLOCK_SIZE, Inode, KIND_DATASET, KIND_DIR, Key, META_OID, ROOT_OID, decode_dirents, item,
};
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

/// Souhrn jedne verze stromu potrebny pro porovnani.
#[derive(Default)]
struct Version {
    /// oid -> (druh, velikost, otisk obsahu)
    objects: BTreeMap<u64, (u8, u64, Csum)>,
    /// oid -> (rodic, jmeno)
    names: BTreeMap<u64, (u64, String)>,
}

impl Version {
    /// Cesta objektu ve verzi (s mezi pro poskozeny cyklus rodicu).
    fn path(&self, mut oid: u64) -> String {
        let mut parts: Vec<&str> = Vec::new();
        for _ in 0..super::MAX_DEPTH {
            if oid == ROOT_OID {
                break;
            }
            let Some((parent, name)) = self.names.get(&oid) else {
                break;
            };
            parts.push(name);
            oid = *parent;
        }
        let mut p = String::new();
        for part in parts.iter().rev() {
            p.push('/');
            p.push_str(part);
        }
        if p.is_empty() {
            p.push('/');
        }
        p
    }
}

impl<D: BlockDevice> Fs<D> {
    /// Nacte souhrn stromu `root` (vsechny objekty, jmena, otisky obsahu).
    fn version_of(&mut self, root: &Child) -> Result<Version> {
        let mut v = Version::default();
        let mut content: BTreeMap<u64, Csum> = BTreeMap::new();
        let mut err = None;
        let tf = self.s.time_format();
        btree::range(
            &mut self.s,
            root,
            &Key::new(ROOT_OID, 0, 0),
            &Key::new(u64::MAX, u8::MAX, u64::MAX),
            &mut |k, val| {
                if k.oid == META_OID {
                    return Ok(true);
                }
                match k.kind {
                    item::INODE => match Inode::decode(val, tf) {
                        Ok(i) => {
                            v.objects.insert(k.oid, (i.kind, i.size, Csum::default()));
                        }
                        Err(e) => {
                            err = Some(e);
                            return Ok(false);
                        }
                    },
                    item::DIRENT => match decode_dirents(val) {
                        Ok(list) => {
                            for d in list {
                                // Vstup do jineho datasetu nema objekt v tomto strome.
                                if d.kind != KIND_DATASET {
                                    v.names.insert(d.oid, (k.oid, d.name));
                                }
                            }
                        }
                        Err(e) => {
                            err = Some(e);
                            return Ok(false);
                        }
                    },
                    item::INLINE | item::EXTENT | item::SYMLINK => {
                        // Otisk = soucet (predchozi otisk || klic || hodnota).
                        let prev = content.get(&k.oid).copied().unwrap_or_default();
                        let mut buf = Vec::with_capacity(16 + 9 + val.len());
                        buf.extend_from_slice(&prev.0);
                        buf.push(k.kind);
                        buf.extend_from_slice(&k.offset.to_le_bytes());
                        buf.extend_from_slice(val);
                        content.insert(k.oid, xxh3(&buf));
                    }
                    _ => {}
                }
                Ok(true)
            },
        )?;
        if let Some(e) = err {
            return Err(e);
        }
        for (oid, sum) in content {
            if let Some(o) = v.objects.get_mut(&oid) {
                o.2 = sum;
            }
        }
        Ok(v)
    }

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

    fn diff_roots(&mut self, old_root: &Child, new_root: &Child) -> Result<Vec<Change>> {
        let old = self.version_of(old_root)?;
        let new = self.version_of(new_root)?;
        let mut out = Vec::new();
        for (&oid, &(kind, size, sum)) in &new.objects {
            if oid == ROOT_OID {
                continue;
            }
            let is_dir = kind == KIND_DIR;
            let path = new.path(oid);
            match old.objects.get(&oid) {
                None => out.push(Change { kind: ChangeKind::Added, path, old_path: None, oid, is_dir }),
                Some(&(_, osize, osum)) => {
                    let moved = old.names.get(&oid) != new.names.get(&oid);
                    let modified = !is_dir && (osize != size || osum != sum);
                    let kind = match (moved, modified) {
                        (true, true) => ChangeKind::MovedModified,
                        (true, false) => ChangeKind::Moved,
                        (false, true) => ChangeKind::Modified,
                        (false, false) => continue,
                    };
                    let old_path = moved.then(|| old.path(oid));
                    out.push(Change { kind, path, old_path, oid, is_dir });
                }
            }
        }
        for (&oid, &(kind, _, _)) in &old.objects {
            if oid != ROOT_OID && !new.objects.contains_key(&oid) {
                out.push(Change {
                    kind: ChangeKind::Removed,
                    path: old.path(oid),
                    old_path: None,
                    oid,
                    is_dir: kind == KIND_DIR,
                });
            }
        }
        out.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(out)
    }

    /// Obnovi soubor, odkaz nebo celou slozku `path` ze snapshotu `snapshot`
    /// datasetu `dataset` jako kopii `dest` v zive verzi (vedle originalu).
    /// `dest` nesmi existovat. Vraci pocet zkopirovanych bajtu.
    pub fn restore_copy(&mut self, dataset: &str, snapshot: &str, path: &str, dest: &str) -> Result<u64> {
        let st = self.snapshot_stat_of(dataset, snapshot, path)?;
        match st.kind {
            Kind::File => {
                self.create(dest, Kind::File)?;
                let mut off = 0u64;
                while off < st.size {
                    let n = ((st.size - off) as usize).min(256 * BLOCK_SIZE);
                    let data = self.snapshot_read_of(dataset, snapshot, path, off, n)?;
                    if data.is_empty() {
                        break;
                    }
                    self.write(dest, off, &data)?;
                    off += data.len() as u64;
                }
                // Ridky konec (dira): velikost jako v originalu.
                if off < st.size {
                    self.truncate(dest, st.size)?;
                }
                Ok(st.size)
            }
            Kind::Symlink => {
                let target = self.snapshot_readlink_of(dataset, snapshot, path)?;
                self.symlink(dest, &target)?;
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
