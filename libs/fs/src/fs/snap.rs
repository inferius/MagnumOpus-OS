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
    DEAD_PER_ITEM, DeadExtent, Key, META_OID, SnapshotRecord, decode_dead, encode_dead, item,
};
use crate::names::{fold_name, stored_name, validate_name};
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
        btree::insert(&mut self.s, key, encode_dead(entries))
    }

    fn records(&mut self, root: &Child) -> Result<Vec<(u64, SnapshotRecord)>> {
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

    /// Ulozi aktualni stav a zachyti ho jako snapshot `name` (jen pro cteni).
    pub fn snapshot(&mut self, name: &str) -> Result<u64> {
        validate_name(name)?;
        if self
            .snapshots()?
            .iter()
            .any(|s| fold_name(&s.name) == fold_name(name))
        {
            return Err(Error::AlreadyExists);
        }
        self.ensure_room(8)?;
        self.commit()?;
        let record = SnapshotRecord {
            root: self.s.sb.root,
            generation: self.s.sb.generation,
            name: stored_name(name),
        };
        let id = self.s.next_snapshot;
        self.s.next_snapshot += 1;
        // Bloky narozene do teto generace odted drzi snapshot.
        self.s.snapshot_generation = self.s.sb.generation;
        btree::insert(
            &mut self.s,
            Key::new(META_OID, item::SNAPSHOT, id),
            record.encode(),
        )?;
        self.commit()?;
        Ok(id)
    }

    pub fn snapshots(&mut self) -> Result<Vec<SnapshotInfo>> {
        let root = self.s.root;
        Ok(self
            .records(&root)?
            .into_iter()
            .map(|(id, r)| SnapshotInfo {
                id,
                name: r.name,
                generation: r.generation,
            })
            .collect())
    }

    fn find_snapshot(&mut self, name: &str) -> Result<(u64, SnapshotRecord)> {
        let folded = fold_name(name);
        let root = self.s.root;
        self.records(&root)?
            .into_iter()
            .find(|(_, r)| fold_name(&r.name) == folded)
            .ok_or(Error::NotFound)
    }

    /// Smaze snapshot a uvolni bloky, ktere drzel jen on.
    pub fn delete_snapshot(&mut self, name: &str) -> Result<()> {
        let (id, _) = self.find_snapshot(name)?;
        self.ensure_room(16)?;
        // Mrtve bloky cekajici v pameti musi byt v deadlistu, jinak by je
        // zpracovani seznamu minulo.
        self.flush_dead()?;
        let root = self.s.root;
        let all = self.records(&root)?;
        let prev_gen = all
            .iter()
            .filter(|(i, _)| *i < id)
            .map(|(_, r)| r.generation)
            .max()
            .unwrap_or(0);
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
                self.s.space.free(d.start, d.len, txg, false);
            }
            if keep.is_empty() {
                btree::remove(&mut self.s, &k)?;
            } else {
                btree::insert(&mut self.s, k, encode_dead(&keep))?;
            }
        }
        // Seznam S se presune do N.
        let (a, b) = list_range(id);
        let root = self.s.root;
        for (k, v) in btree::collect(&mut self.s, &root, &a, &b)? {
            btree::remove(&mut self.s, &k)?;
            let entries = decode_dead(&v)?;
            self.put_dead_item(next_list, &entries)?;
        }
        btree::remove(&mut self.s, &Key::new(META_OID, item::SNAPSHOT, id))?;
        if next.is_none() {
            // Mazal se posledni snapshot: hranici vlastnictvi posunout zpet.
            self.s.snapshot_generation = prev_gen;
        }
        self.s.mark_changed();
        Ok(())
    }

    fn snapshot_root(&mut self, name: &str) -> Result<Child> {
        Ok(Child::Disk(self.find_snapshot(name)?.1.root))
    }

    pub fn snapshot_stat(&mut self, snapshot: &str, path: &str) -> Result<Stat> {
        let root = self.snapshot_root(snapshot)?;
        let oid = self.resolve(&root, path)?;
        self.stat_oid(&root, oid)
    }

    pub fn snapshot_list(&mut self, snapshot: &str, path: &str) -> Result<Vec<DirEntry>> {
        let root = self.snapshot_root(snapshot)?;
        let oid = self.resolve(&root, path)?;
        self.list_oid(&root, oid)
    }

    pub fn snapshot_read(
        &mut self,
        snapshot: &str,
        path: &str,
        offset: u64,
        len: usize,
    ) -> Result<Vec<u8>> {
        let root = self.snapshot_root(snapshot)?;
        let oid = self.resolve(&root, path)?;
        self.read_oid(&root, oid, offset, len, false)
    }

    /// Koreny vsech snapshotu (kontrola konzistence).
    pub(super) fn snapshot_roots(&mut self) -> Result<Vec<(String, Child)>> {
        let root = self.s.root;
        Ok(self
            .records(&root)?
            .into_iter()
            .map(|(_, r)| (r.name, Child::Disk(r.root)))
            .collect())
    }
}
