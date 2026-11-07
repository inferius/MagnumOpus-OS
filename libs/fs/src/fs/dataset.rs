//! Datasety (subvolumy): samostatne stromy se snapshoty a historii.
//!
//! Kazdy dataset ma vlastni B+strom (koren), vlastni cisla objektu, vlastni
//! snapshoty a deadlisty. Korenovy dataset (`ROOT_DATASET`) ma stav
//! v superbloku; ostatni maji zaznam `DatasetRecord` ve strome korenoveho
//! datasetu (`META_OID`, `item::DATASET`, cislo). V adresari je dataset
//! polozka druhu `KIND_DATASET`: cesta do nej prejde na koren jeho stromu.
//!
//! **Aktivni dataset.** Kod operaci pracuje vzdy s "aktualnim" stromem
//! (`Store::root`, `next_oid`, ...). Stav ostatnich nactenych datasetu lezi
//! v `Fs::others` a `activate` ho prohodi - operace nad souborem tak zustaly
//! beze zmeny, jen se pred nimi aktivuje spravny dataset (`enter`).
//!
//! **Klon** vznikne ze snapshotu S: koren = koren S, `snapshot_generation`
//! = generace S. Bloky dosazitelne z S (narozene do generace S) klon jen
//! sdili - jejich prepsani je zapise do deadlistu klonu, uvolnit je smi az
//! S. Snapshot s klony nejde smazat doopravdy: skryje se (`SNAP_HIDDEN`)
//! a smaze se se zanikem posledniho klonu.
//!
//! Operace s datasety (vytvoreni, klon, smazani) hned commituji, proto se
//! nezapisuji do intent logu.

use alloc::collections::BTreeMap;
use alloc::vec::Vec;

use abi::{Error, Result};

use super::{FileDirty, Fs, split_parent, zil};
use crate::btree;
use crate::device::BlockDevice;
use crate::layout::{
    DatasetRecord, DeadExtent, Dirent, FIRST_FREE_OID, Inode, KIND_DATASET, KIND_DIR, Key, META_OID,
    NODE_BLOCKS, ROOT_DATASET, ROOT_OID, SnapshotRecord, item,
};
use crate::names::{stored_name, validate_name};
use crate::node::{Body, Child};

/// Stav neaktivniho datasetu (aktivni ma stejna pole primo v `Store`/`Fs`).
#[derive(Default)]
pub(super) struct DsCtx {
    root: Child,
    next_oid: u64,
    next_snapshot: u64,
    snapshot_generation: u64,
    dead_seq: u64,
    dead: Vec<DeadExtent>,
    origin: (u64, u64, u64),
    data: BTreeMap<u64, FileDirty>,
    zil: zil::Pending,
    defrag_cursor: u64,
    /// Zaznam datasetu je potreba zapsat i bez zmeny stromu (novy klon).
    rec_dirty: bool,
}

impl Default for Child {
    fn default() -> Self {
        Child::Mem(0)
    }
}

impl DsCtx {
    fn from_record(r: &DatasetRecord) -> Self {
        Self {
            root: Child::Disk(r.root),
            next_oid: r.next_oid,
            next_snapshot: r.next_snapshot,
            snapshot_generation: r.snapshot_generation,
            dead_seq: r.dead_seq,
            origin: (r.origin_ds, r.origin_snap, r.origin_gen),
            ..Self::default()
        }
    }
}

/// Polozka vypisu datasetu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DatasetInfo {
    pub id: u64,
    /// Cesta v zive verzi (prazdna, kdyz polozka datasetu nikde neni -
    /// nemelo by nastat).
    pub path: alloc::string::String,
    /// Klon: (dataset, snapshot, generace) puvodu.
    pub origin: Option<(u64, u64, u64)>,
}

impl<D: BlockDevice> Fs<D> {
    /// Udela z datasetu `id` aktivni (nacte ho, kdyz jeste neni v pameti).
    pub(super) fn activate(&mut self, id: u64) -> Result<()> {
        if id == self.cur_ds {
            return Ok(());
        }
        let target = match self.others.remove(&id) {
            Some(c) => c,
            None => {
                let r = self.dataset_record(id)?;
                DsCtx::from_record(&r)
            }
        };
        let old = DsCtx {
            root: self.s.root,
            next_oid: self.s.next_oid,
            next_snapshot: self.s.next_snapshot,
            snapshot_generation: self.s.snapshot_generation,
            dead_seq: self.s.dead_seq,
            dead: core::mem::take(&mut self.s.dead),
            origin: self.cur_origin,
            data: core::mem::take(&mut self.data),
            zil: core::mem::take(&mut self.zil),
            defrag_cursor: self.defrag_cursor,
            rec_dirty: self.cur_rec_dirty,
        };
        self.others.insert(self.cur_ds, old);
        self.s.root = target.root;
        self.s.next_oid = target.next_oid;
        self.s.next_snapshot = target.next_snapshot;
        self.s.snapshot_generation = target.snapshot_generation;
        self.s.dead_seq = target.dead_seq;
        self.s.dead = target.dead;
        self.cur_origin = target.origin;
        self.data = target.data;
        self.zil = target.zil;
        self.defrag_cursor = target.defrag_cursor;
        self.cur_rec_dirty = target.rec_dirty;
        self.cur_ds = id;
        Ok(())
    }

    /// Koren stromu korenoveho datasetu (aktivniho nebo odlozeneho).
    fn root_tree(&self) -> Child {
        if self.cur_ds == ROOT_DATASET {
            self.s.root
        } else {
            self.others[&ROOT_DATASET].root
        }
    }

    fn dataset_record(&mut self, id: u64) -> Result<DatasetRecord> {
        let root = self.root_tree();
        let raw = btree::get(&mut self.s, &root, &Key::new(META_OID, item::DATASET, id))?
            .ok_or(Error::Corrupted)?;
        DatasetRecord::decode(&raw)
    }

    /// Projde cestu v zive verzi pres hranice datasetu: na konci je aktivni
    /// dataset, ve kterem lezi cil, a vrati se jeho oid.
    pub(super) fn enter(&mut self, path: &str) -> Result<u64> {
        if !path.starts_with('/') {
            return Err(Error::InvalidArgs);
        }
        self.activate(ROOT_DATASET)?;
        let mut oid = ROOT_OID;
        for name in super::split_path(path) {
            let root = self.s.root;
            if self.inode(&root, oid)?.kind != KIND_DIR {
                return Err(Error::NotADirectory);
            }
            let d = self.lookup_child(&root, oid, name)?.ok_or(Error::NotFound)?;
            if d.kind == KIND_DATASET {
                self.activate(d.oid)?;
                oid = ROOT_OID;
            } else {
                oid = d.oid;
            }
        }
        Ok(oid)
    }

    /// Aktivuje dataset, jehoz korenem je `path` (napr. "/" nebo "/projekt").
    pub(super) fn enter_dataset(&mut self, path: &str) -> Result<u64> {
        if self.enter(path)? != ROOT_OID {
            return Err(Error::InvalidArgs);
        }
        Ok(self.cur_ds)
    }

    /// Vsechny datasety: korenovy, ulozene i nove v pameti.
    pub(super) fn all_dataset_ids(&mut self) -> Result<Vec<u64>> {
        let mut ids: Vec<u64> = self.datasets_all()?.into_iter().map(|d| d.id).collect();
        ids.push(ROOT_DATASET);
        ids.sort_unstable();
        Ok(ids)
    }

    pub(super) fn loaded_datasets(&self) -> Vec<u64> {
        self.loaded()
    }

    /// Nema dataset `id` zadne zmeny pro intent log?
    pub(super) fn pending_empty(&self, id: u64) -> bool {
        if id == self.cur_ds {
            self.zil.is_empty()
        } else {
            self.others.get(&id).is_none_or(|c| c.zil.is_empty())
        }
    }

    /// Smaze cekajici zmeny pro log ve vsech datasetech (po zapisu davky).
    pub(super) fn clear_all_pending(&mut self) {
        self.zil.clear();
        for c in self.others.values_mut() {
            c.zil.clear();
        }
    }

    /// Cisla vsech datasetu v pameti (aktivni i odlozene).
    fn loaded(&self) -> Vec<u64> {
        let mut v: Vec<u64> = self.others.keys().copied().collect();
        v.push(self.cur_ds);
        v.sort_unstable();
        v
    }

    fn cur_dirty(&self) -> bool {
        matches!(self.s.root, Child::Mem(_))
            || !self.data.is_empty()
            || !self.s.dead.is_empty()
            || self.cur_rec_dirty
    }

    /// Commit vsech datasetu: kazdy zmeneny nekorenovy dataset zapise svuj
    /// strom a zaznam do korenoveho, nakonec commit korenoveho (superblok).
    /// Vse v teze transakci = atomicky.
    pub(super) fn commit_all(&mut self) -> Result<()> {
        // Commit muze prijit uprostred operace (ensure_room): aktivni
        // dataset se na konci vrati.
        let back = self.cur_ds;
        for id in self.loaded() {
            if id == ROOT_DATASET {
                continue;
            }
            self.activate(id)?;
            if !self.cur_dirty() {
                continue;
            }
            self.flush_all_data()?;
            self.flush_dead()?;
            let ptr = self.s.write_tree(self.s.root)?;
            self.s.root = Child::Disk(ptr);
            self.cur_rec_dirty = false;
            let rec = DatasetRecord {
                root: ptr,
                next_oid: self.s.next_oid,
                next_snapshot: self.s.next_snapshot,
                snapshot_generation: self.s.snapshot_generation,
                dead_seq: self.s.dead_seq,
                origin_ds: self.cur_origin.0,
                origin_snap: self.cur_origin.1,
                origin_gen: self.cur_origin.2,
                flags: 0,
            };
            self.activate(ROOT_DATASET)?;
            self.put(Key::new(META_OID, item::DATASET, id), rec.encode())?;
        }
        self.activate(ROOT_DATASET)?;
        self.flush_all_data()?;
        self.flush_dead()?;
        self.s.commit()?;
        self.zil.clear();
        for c in self.others.values_mut() {
            c.zil.clear();
        }
        self.activate(back)
    }

    /// Provede `f` nad kazdym nactenym datasetem a vrati aktivni zpet.
    pub(super) fn for_each_dataset(&mut self, mut f: impl FnMut(&mut Self) -> Result<()>) -> Result<()> {
        let back = self.cur_ds;
        for id in self.loaded() {
            self.activate(id)?;
            f(self)?;
        }
        self.activate(back)
    }

    fn next_dataset_id(&mut self) -> Result<u64> {
        let root = self.root_tree();
        let items = btree::collect(
            &mut self.s,
            &root,
            &Key::new(META_OID, item::DATASET, 0),
            &Key::new(META_OID, item::DATASET + 1, 0),
        )?;
        let on_disk = items.last().map_or(ROOT_DATASET, |(k, _)| k.offset);
        let in_mem = self.loaded().into_iter().max().unwrap_or(ROOT_DATASET);
        Ok(on_disk.max(in_mem) + 1)
    }

    /// Vlozi polozku datasetu `id` do adresare `path` (rodic musi byt
    /// aktivni a jmeno volne).
    fn link_dataset(&mut self, parent: u64, name: &str, id: u64) -> Result<()> {
        let root = self.s.root;
        if self.lookup_child(&root, parent, name)?.is_some() {
            return Err(Error::AlreadyExists);
        }
        self.add_dirent(
            parent,
            Dirent {
                oid: id,
                kind: KIND_DATASET,
                name: stored_name(name),
            },
        )?;
        self.touch_dir(parent)
    }

    /// Vytvori novy prazdny dataset `path` (pro uzivatele obycejna slozka
    /// s vlastni historii). Hned commituje.
    pub fn create_dataset(&mut self, path: &str) -> Result<u64> {
        self.guarded(|fs| {
            fs.require_parent(path, crate::perm::rights::CREATE)?;
            let (parent_path, name) = split_parent(path)?;
            validate_name(name)?;
            let parent = fs.dir_of(parent_path)?;
            fs.ensure_room(8)?;
            let id = fs.next_dataset_id()?;
            let mut root_dir = Inode::new(KIND_DIR, 0, fs.now, fs.s.txg);
            root_dir.owner = fs.new_owner();
            let node = fs.s.new_node(
                0,
                Body::Leaf(alloc::vec![(Key::new(ROOT_OID, item::INODE, 0), root_dir.encode(fs.s.time_format()))]),
            );
            fs.mutations += 1;
            fs.others.insert(
                id,
                DsCtx {
                    root: Child::Mem(node),
                    next_oid: FIRST_FREE_OID,
                    next_snapshot: 1,
                    rec_dirty: true,
                    ..DsCtx::default()
                },
            );
            fs.link_dataset(parent, name, id)?;
            fs.commit()?;
            Ok(id)
        })
    }

    /// Zapisovatelny klon snapshotu `snapshot` datasetu `dataset` jako novy
    /// dataset `path`. Okamzity (sdili bloky). Hned commituje.
    pub fn clone_snapshot(&mut self, dataset: &str, snapshot: &str, path: &str) -> Result<u64> {
        self.require(dataset, crate::perm::rights::MANAGE)?;
        self.require_parent(path, crate::perm::rights::CREATE)?;
        self.guarded(|fs| {
            let (parent_path, name) = split_parent(path)?;
            validate_name(name)?;
            fs.ensure_room(8)?;
            let origin = fs.enter_dataset(dataset)?;
            let (snap_id, mut rec) = fs.find_snapshot(snapshot)?;
            let next_oid = fs.s.next_oid;
            rec.clones = rec.clones.checked_add(1).ok_or(Error::NoResources)?;
            fs.put(Key::new(META_OID, item::SNAPSHOT, snap_id), rec.encode())?;
            let parent = fs.dir_of(parent_path)?;
            let id = fs.next_dataset_id()?;
            fs.others.insert(
                id,
                DsCtx {
                    root: Child::Disk(rec.root),
                    next_oid,
                    next_snapshot: 1,
                    snapshot_generation: rec.generation,
                    origin: (origin, snap_id, rec.generation),
                    rec_dirty: true,
                    ..DsCtx::default()
                },
            );
            fs.link_dataset(parent, name, id)?;
            fs.commit()?;
            Ok(id)
        })
    }

    /// Smaze dataset `path` i s jeho snapshoty a uvolni bloky, ktere drzel
    /// jen on (bloky sdilene s puvodnim snapshotem klonu zustanou).
    ///
    /// Odmitne korenovy dataset, dataset s vnorenymi datasety (`NOT_EMPTY`)
    /// a dataset, z jehoz snapshotu vznikl klon (`BAD_STATE`).
    pub fn delete_dataset(&mut self, path: &str) -> Result<()> {
        self.require(path, crate::perm::rights::MANAGE | crate::perm::rights::DELETE)?;
        self.guarded(|fs| {
            let (parent_path, name) = split_parent(path)?;
            let parent = fs.dir_of(parent_path)?;
            let parent_ds = fs.cur_ds;
            let root = fs.s.root;
            let d = fs.lookup_child(&root, parent, name)?.ok_or(Error::NotFound)?;
            if d.kind != KIND_DATASET {
                return Err(Error::WrongType);
            }
            let id = d.oid;
            fs.ensure_room_freeing(16)?;
            fs.activate(id)?;
            if fs.contains_datasets()? {
                return Err(Error::NotEmpty);
            }
            // Snapshot s klonem (i skryty) drzi bloky pro jiny dataset:
            // nejde - overit drive, nez se cokoli zmeni.
            if fs.records_active()?.iter().any(|(_, r)| r.clones > 0) {
                return Err(Error::BadState);
            }
            // Vlastni snapshoty pryc.
            for s in fs.snapshots_active()? {
                fs.delete_snapshot_active(&s.name)?;
            }
            // Ulozit (strom na disk), pak projit a uvolnit vlastni bloky.
            fs.commit()?;
            fs.activate(id)?;
            let floor = fs.cur_origin.2;
            fs.free_own_blocks(floor)?;
            let origin = fs.cur_origin;
            // Dataset zmizi z pameti i ze stromu korenoveho datasetu.
            fs.activate(parent_ds)?;
            fs.others.remove(&id);
            fs.remove_dirent(parent, &d.name)?;
            fs.touch_dir(parent)?;
            fs.activate(ROOT_DATASET)?;
            fs.del(&Key::new(META_OID, item::DATASET, id))?;
            if origin.0 != 0 {
                fs.release_origin(origin.0, origin.1)?;
            }
            fs.commit()
        })
    }

    /// Klon puvodu zanikl: snizit pocet klonu snapshotu; skryty snapshot
    /// bez klonu se smaze doopravdy.
    fn release_origin(&mut self, ds: u64, snap: u64) -> Result<()> {
        self.activate(ds)?;
        let key = Key::new(META_OID, item::SNAPSHOT, snap);
        let root = self.s.root;
        let mut rec = SnapshotRecord::decode(&btree::get(&mut self.s, &root, &key)?.ok_or(Error::Corrupted)?)?;
        rec.clones = rec.clones.saturating_sub(1);
        if rec.clones == 0 && rec.hidden() {
            self.put(key, rec.encode())?;
            return self.destroy_snapshot(snap);
        }
        self.put(key, rec.encode())
    }

    /// Obsahuje aktivni dataset polozku jineho datasetu?
    fn contains_datasets(&mut self) -> Result<bool> {
        let root = self.s.root;
        let mut found = false;
        btree::range(
            &mut self.s,
            &root,
            &Key::new(0, 0, 0),
            &Key::new(u64::MAX, u8::MAX, u64::MAX),
            &mut |k, v| {
                if k.kind == item::DIRENT
                    && let Ok(list) = crate::layout::decode_dirents(v)
                    && list.iter().any(|d| d.kind == KIND_DATASET)
                {
                    found = true;
                    return Ok(false);
                }
                Ok(true)
            },
        )?;
        Ok(found)
    }

    /// Uvolni bloky aktivniho (ulozeneho) datasetu narozene po `floor`:
    /// pruchod stromem, ktery preskoci podstromy narozene do `floor`
    /// (sdilene s puvodnim snapshotem - patri jemu).
    fn free_own_blocks(&mut self, floor: u64) -> Result<()> {
        let Child::Disk(root) = self.s.root else {
            return Err(Error::BadState);
        };
        let mut stack = alloc::vec![root];
        while let Some(p) = stack.pop() {
            if p.birth <= floor {
                continue;
            }
            let node = self.s.load(&p)?.clone();
            match node.body {
                Body::Internal(children) => {
                    for (_, c) in children {
                        if let Child::Disk(cp) = c {
                            stack.push(cp);
                        }
                    }
                }
                Body::Leaf(items) => {
                    for (k, v) in items {
                        if k.kind == item::EXTENT {
                            let e = crate::layout::Extent::decode(&v)?;
                            if e.birth > floor {
                                self.s.release(e.phys, e.len(), e.birth);
                            }
                        }
                    }
                }
            }
            self.s.release(p.block, NODE_BLOCKS, p.birth);
        }
        self.s.mark_changed();
        Ok(())
    }

    /// Datasety ve svazku (krome korenoveho). S kontrolou opravneni jen
    /// ty, jejichz koren volajici vidi.
    pub fn datasets(&mut self) -> Result<Vec<DatasetInfo>> {
        let all = self.datasets_all()?;
        if !self.checking() {
            return Ok(all);
        }
        let mut out = Vec::new();
        for d in all {
            if !d.path.is_empty() && self.walk(&d.path).is_ok() {
                out.push(d);
            }
        }
        self.activate(ROOT_DATASET)?;
        Ok(out)
    }

    pub(super) fn datasets_all(&mut self) -> Result<Vec<DatasetInfo>> {
        let root = self.root_tree();
        let mut out = Vec::new();
        for (k, v) in btree::collect(
            &mut self.s,
            &root,
            &Key::new(META_OID, item::DATASET, 0),
            &Key::new(META_OID, item::DATASET + 1, 0),
        )? {
            let r = DatasetRecord::decode(&v)?;
            out.push(DatasetInfo {
                id: k.offset,
                path: alloc::string::String::new(),
                origin: (r.origin_ds != 0).then_some((r.origin_ds, r.origin_snap, r.origin_gen)),
            });
        }
        // Nove datasety, ktere jeste nemaji zaznam na disku.
        for id in self.loaded() {
            if id != ROOT_DATASET && !out.iter().any(|d| d.id == id) {
                let origin = if id == self.cur_ds { self.cur_origin } else { self.others[&id].origin };
                out.push(DatasetInfo {
                    id,
                    path: alloc::string::String::new(),
                    origin: (origin.0 != 0).then_some(origin),
                });
            }
        }
        out.sort_by_key(|d| d.id);
        if !out.is_empty() {
            let paths = self.dataset_paths()?;
            for d in &mut out {
                if let Some(p) = paths.get(&d.id) {
                    d.path = p.clone();
                }
            }
        }
        Ok(out)
    }

    /// Cesty datasetu: pruchod adresari zive verze od korene (pres hranice
    /// datasetu). Cena podle poctu adresaru - jen pro vypis datasetu.
    fn dataset_paths(&mut self) -> Result<BTreeMap<u64, alloc::string::String>> {
        let back = self.cur_ds;
        let mut out = BTreeMap::new();
        let mut stack: Vec<(u64, u64, alloc::string::String)> = alloc::vec![(ROOT_DATASET, ROOT_OID, alloc::string::String::new())];
        while let Some((ds, oid, path)) = stack.pop() {
            self.activate(ds)?;
            let root = self.s.root;
            for e in self.list_oid(&root, oid)? {
                let p = alloc::format!("{path}/{}", e.name);
                match e.kind {
                    super::Kind::Dir => stack.push((ds, e.oid, p)),
                    super::Kind::Dataset => {
                        out.insert(e.oid, p.clone());
                        stack.push((e.oid, ROOT_OID, p));
                    }
                    _ => {}
                }
            }
        }
        self.activate(back)?;
        Ok(out)
    }
}
