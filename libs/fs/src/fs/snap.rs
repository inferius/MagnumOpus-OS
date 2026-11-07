//! Snapshoty a seznamy mrtvych bloku (deadlisty).
//!
//! Snapshot je ulozeny koren stromu. Bloky narozene nejpozdeji v generaci
//! posledniho snapshotu (`snapshot_generation`) zivy stav pri prepsani
//! neuvolni, ale zapise do deadlistu: seznamu `L` patri bloky "zabite" mezi
//! predchozim snapshotem a snapshotem `L` (zivy stav ma seznam s cislem
//! `next_snapshot`).
//!
//! Smazani snapshotu S (predchozi P, nasledujici N nebo zivy stav):
//! - bloky v seznamu N narozene po P drzel jen S - uvolni se,
//! - ostatni zustavaji v N a cely seznam S se presune do N.
//!
//! Neni potreba prochazet stromy ani pocitat reference.

use alloc::string::String;
use alloc::vec::Vec;

use abi::{Error, Result};

use super::{DirEntry, Fs, SnapshotInfo, Stat};
use crate::btree;
use crate::device::BlockDevice;
use crate::layout::{
    DEAD_PER_ITEM, DeadExtent, Key, META_OID, SNAP_HIDDEN, SnapshotRecord, decode_dead, encode_dead,
    item,
};
use crate::names::{fold_name, stored_name, validate_name};
use crate::records::{SnapMeta, snapkind};
use crate::node::Child;

fn list_range(list: u64) -> (Key, Key) {
    (
        Key::new(META_OID, item::DEADLIST, list << 32),
        Key::new(META_OID, item::DEADLIST, (list + 1) << 32),
    )
}

impl<D: BlockDevice> Fs<D> {
    /// Zapise mrtve bloky z pameti do deadlistu zive verze. Vkladani samo
    /// meni strom (a muze zabit dalsi bloky), proto ve smycce.
    pub(super) fn flush_dead(&mut self) -> Result<()> {
        loop {
            let mut dead = core::mem::take(&mut self.s.dead);
            if dead.is_empty() {
                return Ok(());
            }
            dead.sort_unstable_by_key(|d| (d.birth, d.start));
            let mut merged: Vec<DeadExtent> = Vec::with_capacity(dead.len());
            for d in dead {
                match merged.last_mut() {
                    Some(p) if p.birth == d.birth && p.start + p.len == d.start => p.len += d.len,
                    _ => merged.push(d),
                }
            }
            let list = self.s.next_snapshot;
            for chunk in merged.chunks(DEAD_PER_ITEM) {
                self.put_dead_item(list, chunk)?;
            }
        }
    }

    fn put_dead_item(&mut self, list: u64, entries: &[DeadExtent]) -> Result<()> {
        let seq = self.s.dead_seq;
        self.s.dead_seq = self.s.dead_seq.wrapping_add(1);
        let key = Key::new(META_OID, item::DEADLIST, (list << 32) | (seq & 0xFFFF_FFFF));
        self.put(key, encode_dead(entries))
    }

    pub(super) fn records(&mut self, root: &Child) -> Result<Vec<(u64, SnapshotRecord)>> {
        let items = btree::collect(
            &mut self.s,
            root,
            &Key::new(META_OID, item::SNAPSHOT, 0),
            &Key::new(META_OID, item::SNAPSHOT + 1, 0),
        )?;
        items
            .into_iter()
            .map(|(k, v)| Ok((k.offset, SnapshotRecord::decode(&v)?)))
            .collect()
    }

    /// Ulozi aktualni stav a zachyti korenovy dataset jako snapshot `name`
    /// (jen pro cteni). Jiny dataset: `snapshot_of`.
    pub fn snapshot(&mut self, name: &str) -> Result<u64> {
        self.snapshot_of("/", name)
    }

    /// Snapshot datasetu, jehoz korenem je `dataset` (napr. "/projekt").
    pub fn snapshot_of(&mut self, dataset: &str, name: &str) -> Result<u64> {
        validate_name(name)?;
        self.require(dataset, crate::perm::rights::MANAGE)?;
        self.enter_dataset(dataset)?;
        Ok(self.snapshot_internal(name, snapkind::MANUAL)?.0)
    }

    /// Snapshot aktivniho datasetu daneho druhu (rucni, automaticky,
    /// pohled, ...). Vraci cislo a ulozena metadata.
    pub(super) fn snapshot_internal(&mut self, name: &str, kind: u8) -> Result<(u64, SnapMeta)> {
        self.snapshot_with(name, SnapMeta { kind, ..SnapMeta::default() }, Vec::new())
    }

    /// Snapshot aktivniho datasetu s metadaty `meta` (cas a pozici
    /// zurnalu doplni). Dva commity: prvni ulozi stav (koren snapshotu),
    /// druhy zaznam snapshotu a polozky `extra` (napr. `HEAD` commitu).
    pub(super) fn snapshot_with(&mut self, name: &str, mut meta: SnapMeta, extra: Vec<(Key, Vec<u8>)>) -> Result<(u64, SnapMeta)> {
        if self
            .records_active()?
            .iter()
            .any(|(_, r)| !r.hidden() && fold_name(&r.name) == fold_name(name))
        {
            return Err(Error::AlreadyExists);
        }
        self.ensure_room(8)?;
        self.commit()?;
        let Child::Disk(root) = self.s.root else {
            return Err(Error::BadState);
        };
        let record = SnapshotRecord {
            root,
            generation: self.s.sb.generation,
            clones: 0,
            flags: 0,
            name: stored_name(name),
        };
        let id = self.s.next_snapshot;
        self.s.next_snapshot += 1;
        // Bloky narozene do teto generace odted drzi snapshot.
        self.s.snapshot_generation = self.s.sb.generation;
        meta.time = self.now;
        // Po commitu: posledni zaznam zurnalu, ktery koren obsahuje.
        meta.journal_seq = self.journal_published()?;
        self.put(Key::new(META_OID, item::SNAPSHOT, id), record.encode())?;
        self.put(Key::new(META_OID, item::SNAPMETA, id), meta.encode())?;
        for (k, v) in extra {
            self.put(k, v)?;
        }
        self.jappend(crate::records::JournalEntry {
            op: crate::records::jop::SNAPSHOT,
            aux: id,
            name: record.name.clone(),
            ..Default::default()
        })?;
        self.commit()?;
        Ok((id, meta))
    }

    /// Metadata snapshotu `id` aktivniho datasetu (snapshot z doby pred
    /// fazi 2 je nema: rucni, cas neznamy).
    pub(super) fn snap_meta(&mut self, id: u64) -> Result<SnapMeta> {
        let root = self.s.root;
        match btree::get(&mut self.s, &root, &Key::new(META_OID, item::SNAPMETA, id))? {
            Some(raw) => SnapMeta::decode(&raw),
            None => Ok(SnapMeta::default()),
        }
    }

    /// Snapshoty korenoveho datasetu.
    pub fn snapshots(&mut self) -> Result<Vec<SnapshotInfo>> {
        self.snapshots_of("/")
    }

    /// Snapshoty datasetu `dataset` (skryte - drzene jen kvuli klonum - ne).
    pub fn snapshots_of(&mut self, dataset: &str) -> Result<Vec<SnapshotInfo>> {
        self.require(dataset, crate::perm::rights::READ)?;
        self.enter_dataset(dataset)?;
        self.snapshots_active()
    }

    /// Snapshoty aktivniho datasetu (bez skrytych a docasnych pohledu).
    pub(super) fn snapshots_active(&mut self) -> Result<Vec<SnapshotInfo>> {
        let mut out = Vec::new();
        for (id, r) in self.records_active()? {
            if r.hidden() {
                continue;
            }
            let m = self.snap_meta(id)?;
            if m.kind == snapkind::VIEW {
                continue;
            }
            out.push(SnapshotInfo {
                id,
                name: r.name,
                generation: r.generation,
                clones: r.clones,
                kind: m.kind,
                time: m.time,
                journal_seq: m.journal_seq,
            });
        }
        Ok(out)
    }

    pub(super) fn records_active(&mut self) -> Result<Vec<(u64, SnapshotRecord)>> {
        let root = self.s.root;
        self.records(&root)
    }

    /// Snapshot podle jmena v aktivnim datasetu (skryte se nehledaji).
    pub(super) fn find_snapshot(&mut self, name: &str) -> Result<(u64, SnapshotRecord)> {
        let folded = fold_name(name);
        self.records_active()?
            .into_iter()
            .find(|(_, r)| !r.hidden() && fold_name(&r.name) == folded)
            .ok_or(Error::NotFound)
    }

    /// Smaze snapshot korenoveho datasetu a uvolni bloky, ktere drzel jen on.
    pub fn delete_snapshot(&mut self, name: &str) -> Result<()> {
        self.delete_snapshot_of("/", name)
    }

    /// Smaze snapshot datasetu `dataset`. Snapshot, ze ktereho vznikl klon,
    /// se jen skryje (bloky drzi dal pro klon) a smaze se se zanikem
    /// posledniho klonu.
    pub fn delete_snapshot_of(&mut self, dataset: &str, name: &str) -> Result<()> {
        self.require(dataset, crate::perm::rights::MANAGE)?;
        self.guarded(|fs| {
            fs.enter_dataset(dataset)?;
            fs.ensure_room_freeing(16)?;
            fs.delete_snapshot_active(name)
        })
    }

    pub(super) fn delete_snapshot_active(&mut self, name: &str) -> Result<()> {
        let (id, mut rec) = self.find_snapshot(name)?;
        if rec.clones > 0 {
            rec.flags |= SNAP_HIDDEN;
            return self.put(Key::new(META_OID, item::SNAPSHOT, id), rec.encode());
        }
        self.destroy_snapshot(id)
    }

    /// Doopravdy smaze snapshot `id` aktivniho datasetu (deadlisty).
    pub(super) fn destroy_snapshot(&mut self, id: u64) -> Result<()> {
        // Mrtve bloky cekajici v pameti musi byt v deadlistu, jinak by je
        // zpracovani seznamu minulo.
        self.flush_dead()?;
        let root = self.s.root;
        let all = self.records(&root)?;
        // Spodni hranice: u klonu generace puvodu (bloky do ni patri
        // puvodnimu snapshotu, ne klonu).
        let floor = self.cur_origin.2;
        let prev_gen = all
            .iter()
            .filter(|(i, _)| *i < id)
            .map(|(_, r)| r.generation)
            .max()
            .unwrap_or(0)
            .max(floor);
        let next = all.iter().map(|(i, _)| *i).filter(|&i| i > id).min();
        let next_list = next.unwrap_or(self.s.next_snapshot);
        // Seznam N: bloky narozene po P drzel jen mazany snapshot.
        let (a, b) = list_range(next_list);
        let root = self.s.root;
        for (k, v) in btree::collect(&mut self.s, &root, &a, &b)? {
            let (free, keep): (Vec<DeadExtent>, Vec<DeadExtent>) = decode_dead(&v)?
                .into_iter()
                .partition(|d| d.birth > prev_gen);
            if free.is_empty() {
                continue;
            }
            for d in &free {
                let txg = self.s.txg;
                let _ = txg;
                self.s.free_final(d.start, d.len, d.copy, true);
            }
            if keep.is_empty() {
                self.del(&k)?;
            } else {
                self.put(k, encode_dead(&keep))?;
            }
        }
        // Seznam S se presune do N.
        let (a, b) = list_range(id);
        let root = self.s.root;
        for (k, v) in btree::collect(&mut self.s, &root, &a, &b)? {
            self.del(&k)?;
            let entries = decode_dead(&v)?;
            self.put_dead_item(next_list, &entries)?;
        }
        self.del(&Key::new(META_OID, item::SNAPSHOT, id))?;
        self.del(&Key::new(META_OID, item::SNAPMETA, id))?;
        if next.is_none() {
            // Mazal se posledni snapshot: hranici vlastnictvi posunout zpet.
            self.s.snapshot_generation = prev_gen;
        }
        self.s.mark_changed();
        Ok(())
    }

    fn snapshot_root(&mut self, dataset: &str, name: &str) -> Result<Child> {
        self.enter_dataset(dataset)?;
        Ok(Child::Disk(self.find_snapshot(name)?.1.root))
    }

    pub fn snapshot_stat(&mut self, snapshot: &str, path: &str) -> Result<Stat> {
        self.snapshot_stat_of("/", snapshot, path)
    }

    pub fn snapshot_list(&mut self, snapshot: &str, path: &str) -> Result<Vec<DirEntry>> {
        self.snapshot_list_of("/", snapshot, path)
    }

    pub fn snapshot_read(&mut self, snapshot: &str, path: &str, offset: u64, len: usize) -> Result<Vec<u8>> {
        self.snapshot_read_of("/", snapshot, path, offset, len)
    }

    /// `stat` ve snapshotu datasetu `dataset` (cesta uvnitr datasetu).
    pub fn snapshot_stat_of(&mut self, dataset: &str, snapshot: &str, path: &str) -> Result<Stat> {
        let root = self.snapshot_root(dataset, snapshot)?;
        if !self.checking() {
            let oid = self.resolve(&root, path)?;
            return self.stat_oid(&root, oid);
        }
        let w = self.walk_snapshot(dataset, &root, path)?;
        let mut st = self.stat_oid(&root, w.oid)?;
        st.rights = w.rights;
        self.limit_stat(&mut st);
        Ok(st)
    }

    pub fn snapshot_list_of(&mut self, dataset: &str, snapshot: &str, path: &str) -> Result<Vec<DirEntry>> {
        let root = self.snapshot_root(dataset, snapshot)?;
        if !self.checking() {
            let oid = self.resolve(&root, path)?;
            return self.list_oid(&root, oid);
        }
        let w = self.walk_snapshot(dataset, &root, path)?;
        if w.rights & crate::perm::rights::LIST == 0 {
            return Err(Error::AccessDenied);
        }
        let list = self.list_oid(&root, w.oid)?;
        self.visible(&root, &w.below, list)
    }

    /// Objekt ve snapshotu s kontrolou prav `need` (bez kontrol jen cesta).
    pub(super) fn snapshot_object(&mut self, dataset: &str, root: &Child, path: &str, need: u8) -> Result<u64> {
        if !self.checking() {
            return self.resolve(root, path);
        }
        let w = self.walk_snapshot(dataset, root, path)?;
        if w.rights & need != need {
            return Err(Error::AccessDenied);
        }
        Ok(w.oid)
    }

    pub fn snapshot_read_of(
        &mut self,
        dataset: &str,
        snapshot: &str,
        path: &str,
        offset: u64,
        len: usize,
    ) -> Result<Vec<u8>> {
        let root = self.snapshot_root(dataset, snapshot)?;
        let oid = self.snapshot_object(dataset, &root, path, crate::perm::rights::READ)?;
        self.read_oid(&root, oid, offset, len, false)
    }

    /// Koreny vsech snapshotu (kontrola konzistence).
    /// Koreny snapshotu zaznamenanych ve strome `root`.
    pub(super) fn snapshot_roots_of(&mut self, root: &Child) -> Result<Vec<(String, Child)>> {
        let root = *root;
        Ok(self
            .records(&root)?
            .into_iter()
            .map(|(_, r)| (r.name, Child::Disk(r.root)))
            .collect())
    }
}
