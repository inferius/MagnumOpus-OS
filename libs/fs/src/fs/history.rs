//! Historie: verze souboru napric snapshoty, obnova verze, vraceni operaci
//! podle zurnalu a automaticke snapshoty s pravidly uchovani.
//!
//! - **Verze souboru:** objekt ma v datasetu stabilni oid, takze se ve
//!   snapshotech hleda primo podle nej (presun ani prejmenovani nevadi).
//!   Verze = ruzna hodnota `Inode::version` (pocitadlo zmen obsahu).
//! - **Vraceni operace** (`undo`) podle cisla v zurnalu: presun se vrati
//!   zpet, kos obnovi, vytvoreni presune do kose, trvale smazani a zmena
//!   obsahu se obnovi z posledniho snapshotu pred operaci. Nic se nemaze
//!   natrvalo: co by se prepsalo, jde do kose.
//! - **Automaticke snapshoty:** pravidla jsou nastaveni datasetu
//!   (`Settings::rules`), rozhoduje deterministicky krok `auto_snapshot_tick`
//!   (sluzba ho vola periodicky): novy snapshot, kdyz uplynul nejkratsi
//!   interval a dataset se zmenil; pak prorezani - kazde pravidlo drzi
//!   nejnovejsi snapshot v kazdem z poslednich `keep` intervalu.

use alloc::collections::BTreeSet;
use alloc::string::String;
use alloc::vec::Vec;

use abi::{Error, Result};

use super::{Fs, split_parent};
use crate::btree;
use crate::device::BlockDevice;
use crate::layout::{Inode, Key, META_OID, SnapshotRecord, Timestamp, item};
use crate::node::Child;
use crate::perm::rights;
use crate::records::{JournalEntry, jop, snapkind};

/// Jedna verze souboru.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileVersion {
    /// Nejstarsi snapshot, ktery tuto verzi obsahuje.
    pub snapshot: String,
    pub snapshot_id: u64,
    /// Kdy snapshot vznikl.
    pub time: Timestamp,
    pub size: u64,
    pub mtime: Timestamp,
    /// Pocitadlo zmen obsahu (`Revision::version`).
    pub version: u64,
}

/// Co udelalo vraceni operace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Undone {
    /// Objekt se presunul zpet (puvodni cesta).
    MovedBack(String),
    /// Obnoveny z kose (cesta).
    Restored(String),
    /// Objekt se presunul do kose (cislo zaznamu kose).
    Trashed(u64),
    /// Obnoveno ze snapshotu jako novy objekt (cesta); predchozi obsah je
    /// v kosi (cislo zaznamu), kdyz nejaky byl.
    FromSnapshot { path: String, snapshot: String, trashed: Option<u64> },
}

/// Jak obnovit verzi souboru.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RestoreMode<'a> {
    /// Jako kopie na ceste `dest` (vedle originalu).
    Copy(&'a str),
    /// Nahradit aktualni: obnovena kopie dostane jeho jmeno, aktualni jde
    /// do kose (nic se neztrati).
    Replace,
}

fn join(dir: &str, name: &str) -> String {
    let mut p = String::from(dir.trim_end_matches('/'));
    p.push('/');
    p.push_str(name);
    p
}

/// Jmeno automatickeho snapshotu: `auto-RRRR-MM-DD-HHMMSS` (UTC).
fn auto_name(t: Timestamp) -> String {
    let c = t.to_civil();
    alloc::format!(
        "auto-{:04}-{:02}-{:02}-{:02}{:02}{:02}",
        c.year, c.month, c.day, c.hour, c.minute, c.second
    )
}

impl<D: BlockDevice> Fs<D> {
    /// Snapshoty aktivniho datasetu se zaznamem (bez skrytych a pohledu),
    /// od nejstarsiho.
    fn history_snapshots(&mut self) -> Result<Vec<(u64, SnapshotRecord, crate::records::SnapMeta)>> {
        let mut out = Vec::new();
        for (id, r) in self.records_active()? {
            if r.hidden() {
                continue;
            }
            let m = self.snap_meta(id)?;
            if m.kind != snapkind::VIEW {
                out.push((id, r, m));
            }
        }
        Ok(out)
    }

    /// Zaznam objektu `oid` ve strome `root`, kdyz ma generaci `generation`.
    fn object_in(&mut self, root: &Child, oid: u64, generation: u64) -> Result<Option<Inode>> {
        let Some(raw) = btree::get(&mut self.s, root, &Key::new(oid, item::INODE, 0))? else {
            return Ok(None);
        };
        let i = Inode::decode(&raw, self.s.time_format())?;
        Ok((i.generation == generation).then_some(i))
    }

    /// Verze souboru `path` ve snapshotech jeho datasetu, od nejstarsi
    /// (kazda verze jednou, s nejstarsim snapshotem, ktery ji obsahuje).
    /// Aktualni (ziva) verze v seznamu neni. Vyzaduje `READ`.
    pub fn file_versions(&mut self, path: &str) -> Result<Vec<FileVersion>> {
        let oid = self.require(path, rights::READ)?;
        let root = self.s.root;
        let live = self.inode(&root, oid)?;
        let mut out: Vec<FileVersion> = Vec::new();
        for (id, rec, meta) in self.history_snapshots()? {
            let Some(i) = self.object_in(&Child::Disk(rec.root), oid, live.generation)? else {
                continue;
            };
            if out.last().is_some_and(|v| v.version == i.version) {
                continue;
            }
            out.push(FileVersion {
                snapshot: rec.name,
                snapshot_id: id,
                time: meta.time,
                size: i.size,
                mtime: i.mtime,
                version: i.version,
            });
        }
        Ok(out)
    }

    /// Kopie objektu `oid` ze snapshotu `snapshot` aktivniho datasetu na
    /// cestu `dest` v zive verzi - pres `restore_copy`, ktere kontroluje
    /// prava u kazde polozky (kopie slozky nevezme, co volajici nevidi).
    /// Vraci zkopirovane bajty.
    fn copy_from(&mut self, snapshot: &str, root: &Child, oid: u64, dest: &str) -> Result<u64> {
        let ds = self.dataset_path(self.cur_ds)?;
        let src = self.path_in(root, oid)?.ok_or(Error::NotFound)?;
        self.restore_copy(&ds, snapshot, &src, dest)
    }

    /// Najde snapshot `snapshot` aktivniho datasetu (bez pohledu).
    fn history_root(&mut self, snapshot: &str) -> Result<Child> {
        Ok(Child::Disk(self.find_snapshot(snapshot)?.1.root))
    }

    /// Obnovi verzi souboru (nebo slozky) `path` ze snapshotu `snapshot`
    /// jeho datasetu: jako kopii (`Copy`) nebo misto aktualni (`Replace` -
    /// aktualni jde do kose). Objekt se ve snapshotu hleda podle oid,
    /// takze prejmenovani mezitim nevadi. Vraci cestu obnoveneho objektu.
    pub fn restore_version(&mut self, path: &str, snapshot: &str, mode: RestoreMode<'_>) -> Result<String> {
        let oid = self.require(path, rights::READ)?;
        let root = self.s.root;
        let live = self.inode(&root, oid)?;
        let snap = self.history_root(snapshot)?;
        if self.object_in(&snap, oid, live.generation)?.is_none() {
            return Err(Error::NotFound);
        }
        match mode {
            RestoreMode::Copy(dest) => {
                self.copy_from(snapshot, &snap, oid, dest)?;
                Ok(dest.into())
            }
            RestoreMode::Replace => {
                self.replace_from(snapshot, &snap, oid, path)?;
                Ok(path.into())
            }
        }
    }

    /// Nahradi objekt na `path` kopii objektu `oid` ze stromu `snap`:
    /// kopie vedle (docasne jmeno), aktualni do kose, kopie na jeho misto.
    fn replace_from(&mut self, snapshot: &str, snap: &Child, oid: u64, path: &str) -> Result<u64> {
        let (dir, name) = split_parent(path)?;
        let temp = join(dir, &alloc::format!(".obnova-{}-{}", oid, self.s.next_oid));
        self.copy_from(snapshot, snap, oid, &temp)?;
        let id = self.trash(path)?;
        self.rename(&temp, &join(dir, name))?;
        Ok(id)
    }

    /// Posledni snapshot aktivniho datasetu pred zaznamem zurnalu `seq`
    /// (obsahuje stav pred operaci), ve kterem existuje objekt (oid, generace).
    fn snapshot_before(&mut self, seq: u64, oid: u64, generation: u64) -> Result<(String, Child)> {
        let mut best = None;
        for (_, rec, meta) in self.history_snapshots()? {
            if meta.journal_seq < seq && meta.journal_seq > 0 {
                let root = Child::Disk(rec.root);
                if self.object_in(&root, oid, generation)?.is_some() {
                    best = Some((meta.journal_seq, rec.name, root));
                }
            }
        }
        best.map(|(_, n, r)| (n, r)).ok_or(Error::NotFound)
    }

    /// Cesta v zive verzi: koren datasetu `dataset` + cesta uvnitr.
    fn full_path(dataset: &str, inner: &str) -> String {
        let mut p = String::from(dataset.trim_end_matches('/'));
        if inner != "/" {
            p.push_str(inner);
        }
        if p.is_empty() {
            p.push('/');
        }
        p
    }

    /// Vrati operaci zurnalu `seq` datasetu `dataset` (viz hlavicka modulu).
    /// Prava jako pri provedeni opacne operace rucne (`DELETE`, `CREATE`).
    /// `CONFLICT`, kdyz objekt mezitim zmenil misto nebo cil je obsazeny
    /// jinym objektem; `NOT_SUPPORTED` pro operace bez opaku (metadata,
    /// snapshot); `NOT_FOUND`, kdyz neni snapshot pred operaci.
    pub fn undo(&mut self, dataset: &str, seq: u64) -> Result<Undone> {
        let page = self.journal_read(dataset, seq, 1)?;
        let Some((_, e)) = page.entries.into_iter().next() else {
            return Err(Error::NotFound);
        };
        match e.op {
            jop::RENAME => self.undo_rename(dataset, &e),
            jop::TRASH => {
                let path = self.trash_restore(dataset, e.aux, None)?;
                Ok(Undone::Restored(path))
            }
            jop::CREATE | jop::RESTORE => {
                let (parent, name) = if e.op == jop::CREATE { (e.parent, &e.name) } else { (e.new_parent, &e.new_name) };
                let path = self.current_path(dataset, parent, name, e.oid)?;
                Ok(Undone::Trashed(self.trash(&path)?))
            }
            jop::REMOVE => {
                self.enter_dataset(dataset)?;
                let (snapshot, root) = self.snapshot_before(seq, e.oid, e.generation)?;
                let dir = self.path_of(e.parent)?.ok_or(Error::NotFound)?;
                let path = join(&Self::full_path(dataset, &dir), &e.name);
                self.copy_from(&snapshot, &root, e.oid, &path)?;
                Ok(Undone::FromSnapshot { path, snapshot, trashed: None })
            }
            jop::CONTENT => {
                self.enter_dataset(dataset)?;
                let root = self.s.root;
                let live = self.object_in(&root, e.oid, e.generation)?.ok_or(Error::NotFound)?;
                let dir = self.path_of(live.parent)?.ok_or(Error::NotFound)?;
                let name = self.name_of(live.parent, e.oid)?;
                let path = join(&Self::full_path(dataset, &dir), &name);
                self.require(&path, rights::WRITE)?;
                self.enter_dataset(dataset)?;
                let (snapshot, snap) = self.snapshot_before(seq, e.oid, e.generation)?;
                let id = self.replace_from(&snapshot, &snap, e.oid, &path)?;
                Ok(Undone::FromSnapshot { path, snapshot, trashed: Some(id) })
            }
            _ => Err(Error::NotSupported),
        }
    }

    /// Cesta objektu, ktery ma lezet v `parent`/`name` (jinak `CONFLICT`).
    fn current_path(&mut self, dataset: &str, parent: u64, name: &str, oid: u64) -> Result<String> {
        self.enter_dataset(dataset)?;
        let root = self.s.root;
        match self.lookup_child(&root, parent, name)? {
            Some(d) if d.oid == oid => {}
            _ => return Err(Error::Conflict),
        }
        let dir = self.path_of(parent)?.ok_or(Error::Conflict)?;
        Ok(join(&Self::full_path(dataset, &dir), name))
    }

    fn undo_rename(&mut self, dataset: &str, e: &JournalEntry) -> Result<Undone> {
        let now = self.current_path(dataset, e.new_parent, &e.new_name, e.oid)?;
        self.enter_dataset(dataset)?;
        let dir = self.path_of(e.parent)?.ok_or(Error::NotFound)?;
        let back = join(&Self::full_path(dataset, &dir), &e.name);
        match self.rename(&now, &back) {
            Err(Error::AlreadyExists) => Err(Error::Conflict),
            r => r.map(|()| Undone::MovedBack(back)),
        }
    }

    /// Krok automatickych snapshotu (sluzba ho vola periodicky, napr.
    /// kazdou minutu): ve vsech datasetech s pravidly vytvori snapshot,
    /// kdyz uplynul nejkratsi interval a dataset se od posledniho
    /// automatickeho zmenil, a prorezava automaticke snapshoty podle
    /// pravidel. Rucni snapshoty a commity nemeni. Vraci (vytvorenych,
    /// smazanych). Jen system.
    pub fn auto_snapshot_tick(&mut self, now: Timestamp) -> Result<(usize, usize)> {
        self.require_system()?;
        let saved = self.now;
        self.now = now;
        let r = self.auto_tick_inner(now);
        self.now = saved;
        r
    }

    fn auto_tick_inner(&mut self, now: Timestamp) -> Result<(usize, usize)> {
        let (mut made, mut dropped) = (0, 0);
        for ds in self.all_dataset_ids()? {
            self.activate(ds)?;
            let set = self.settings_active()?;
            if set.rules.is_empty() {
                continue;
            }
            let autos: Vec<_> = self.history_snapshots()?.into_iter().filter(|(_, _, m)| m.kind == snapkind::AUTO).collect();
            let step = set.rules.iter().map(|r| r.interval).min().unwrap_or(u64::MAX) as i64;
            let due = autos.last().is_none_or(|(_, _, m)| now.secs.div_euclid(step) > m.time.secs.div_euclid(step));
            let changed = match autos.last() {
                None => true,
                Some((_, _, m)) => self.changed_since(m.journal_seq)?,
            };
            if due && changed {
                let mut name = auto_name(now);
                let taken: BTreeSet<String> = self.records_active()?.into_iter().map(|(_, r)| crate::names::fold_name(&r.name)).collect();
                let mut n = 2;
                while taken.contains(&crate::names::fold_name(&name)) {
                    name = alloc::format!("{}-{n}", auto_name(now));
                    n += 1;
                }
                self.guarded(|fs| fs.snapshot_internal(&name, snapkind::AUTO))?;
                made += 1;
            }
            // Prorezani: co nedrzi zadne pravidlo, smazat (nejnovejsi vzdy zustane).
            let autos: Vec<_> = self.history_snapshots()?.into_iter().filter(|(_, _, m)| m.kind == snapkind::AUTO).collect();
            let mut keep = BTreeSet::new();
            if let Some((id, _, _)) = autos.last() {
                keep.insert(*id);
            }
            for rule in &set.rules {
                let iv = rule.interval as i64;
                let current = now.secs.div_euclid(iv);
                let mut seen = BTreeSet::new();
                // Od nejnovejsiho: prvni snapshot v kazdem intervalu.
                for (id, _, m) in autos.iter().rev() {
                    let b = m.time.secs.div_euclid(iv);
                    if b > current - rule.keep as i64 && seen.insert(b) {
                        keep.insert(*id);
                    }
                }
            }
            for (id, rec, _) in autos {
                if !keep.contains(&id) {
                    self.guarded(|fs| {
                        fs.ensure_room_freeing(16)?;
                        fs.delete_snapshot_active(&rec.name)
                    })?;
                    dropped += 1;
                }
            }
            if dropped > 0 {
                self.commit()?;
            }
        }
        self.activate(crate::layout::ROOT_DATASET)?;
        Ok((made, dropped))
    }

    /// Zmenil se aktivni dataset od zaznamu `seq`? (Zaznamy `SNAPSHOT`
    /// samy zmenu dat nejsou.)
    fn changed_since(&mut self, seq: u64) -> Result<bool> {
        // I nepotvrzene zmeny (snapshot je stejne ulozi commitem).
        let (last, pending) = self.journal_pending()?;
        if pending {
            return Ok(true);
        }
        let root = self.s.root;
        let mut changed = false;
        btree::range(
            &mut self.s,
            &root,
            &Key::new(META_OID, item::JOURNAL, seq + 1),
            &Key::new(META_OID, item::JOURNAL, last + 1),
            &mut |_, v| {
                if JournalEntry::decode(v)?.op != jop::SNAPSHOT {
                    changed = true;
                    return Ok(false);
                }
                Ok(true)
            },
        )?;
        Ok(changed)
    }

}
