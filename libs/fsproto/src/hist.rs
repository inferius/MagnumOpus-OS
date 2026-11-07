//! Protokol v2: historie (faze 2 FS) - zurnal zmen a jeho sledovani,
//! pohled s pozici, kos, vraceni operaci, verze souboru, nastaveni
//! datasetu, verzovane commity, vetve a merge, "zkusit zmeny bokem",
//! rozsirene atributy a expirace; vytvoreni a smazani datasetu.
//!
//! Cesty jsou cesty relace (preklad pres pripojene cesty jako u ostatnich
//! operaci). "Dataset" = cesta ke koreni datasetu. Prava kontroluje FS
//! (operace nad historii datasetu vyzaduji `MANAGE`).
//!
//! **Sledovani zmen** (`JOURNAL_WATCH`): odpoved prijde, az je k dispozici
//! aspon jeden zaznam od zadaneho cisla, nebo po casovem limitu (prazdna
//! stranka). Zaznamy se zverejnuji po potvrzeni (commit kazdych 5 s nebo
//! fsync klienta). Relace s cekajicim `WATCH` dalsi pozadavky nezpracuje,
//! dokud nedostane odpoved - pro sledovani je vhodna vlastni relace
//! (`CONNECT`).

use alloc::string::String;
use alloc::vec::Vec;

use libos::time::WallTime;
use libos::{Error, Result};

use crate::{FsClient, Reader, Writer, get_time, put_time};

/// Kody operaci (navazuji na `perm::op`).
pub mod op {
    /// dataset, od u64, nejvic u32 -> `JournalPage`
    pub const JOURNAL_READ: u32 = 54;
    /// dataset, od u64, nejvic u32, limit ms u32 -> `JournalPage` (ceka)
    pub const JOURNAL_WATCH: u32 = 55;
    /// dataset -> snapshot str, id u64, posledni zaznam u64
    pub const VIEW: u32 = 56;
    /// dataset, snapshot str
    pub const VIEW_RELEASE: u32 = 57;
    /// cesta -> cislo zaznamu kose u64
    pub const TRASH: u32 = 58;
    /// dataset -> pocet u32 + `TrashEntry`
    pub const TRASH_LIST: u32 = 59;
    /// dataset, cislo u64, cil str (prazdny = puvodni misto) -> cesta str
    pub const TRASH_RESTORE: u32 = 60;
    /// dataset, cislo u64
    pub const TRASH_PURGE: u32 = 61;
    /// dataset, cislo zaznamu zurnalu u64 -> `Undone`
    pub const UNDO: u32 = 62;
    /// cesta -> pocet u32 + `FileVersion`
    pub const FILE_VERSIONS: u32 = 63;
    /// cesta, snapshot str, nahradit u8, cil str -> cesta str
    pub const RESTORE_VERSION: u32 = 64;
    /// dataset -> `Settings`
    pub const SETTINGS: u32 = 65;
    /// dataset, `Settings`
    pub const SET_SETTINGS: u32 = 66;
    /// dataset, zprava str -> `CommitInfo`
    pub const COMMIT_VERSION: u32 = 67;
    /// dataset, nejvic u32 -> pocet u32 + `CommitInfo`
    pub const LOG: u32 = 68;
    /// dataset, jmeno str, cesta str -> dataset vetve u64
    pub const BRANCH: u32 = 69;
    /// dataset -> pocet u32 + (jmeno str, dataset u64, cesta str)
    pub const BRANCHES: u32 = 70;
    /// dataset, jmeno str
    pub const DELETE_BRANCH: u32 = 71;
    /// cil str, zdroj str -> `MergeReport`
    pub const MERGE: u32 = 72;
    /// cil str
    pub const MERGE_ABORT: u32 = 73;
    /// dataset, cesta klonu str -> dataset klonu u64
    pub const ASIDE_BEGIN: u32 = 74;
    /// klon, vse u8, pocet u32 + cesty str -> `MergeReport`
    pub const ASIDE_APPLY: u32 = 75;
    /// klon
    pub const ASIDE_DISCARD: u32 = 76;
    /// klon -> pocet u32 + `Change`
    pub const ASIDE_CHANGES: u32 = 77;
    /// cesta, jmeno str -> hodnota (zbytek zpravy)
    pub const XATTR_GET: u32 = 78;
    /// cesta, jmeno str, hodnota (zbytek zpravy)
    pub const XATTR_SET: u32 = 79;
    /// cesta, jmeno str
    pub const XATTR_REMOVE: u32 = 80;
    /// cesta -> pocet u32 + jmena str
    pub const XATTR_LIST: u32 = 81;
    /// cesta, ma u8, cas
    pub const SET_EXPIRES: u32 = 82;
    /// cesta -> ma u8, cas
    pub const EXPIRES: u32 = 83;
    /// dataset -> ma u8, `CommitInfo`
    pub const HEAD: u32 = 84;
    /// cesta -> cislo datasetu u64 (nova slozka s vlastni historii)
    pub const DATASET_CREATE: u32 = 85;
    /// cesta (i se snapshoty; ne s vnorenymi datasety a klony)
    pub const DATASET_DELETE: u32 = 86;
}

/// Nejvic polozek ve vypisu (ochrana pameti sluzby i klienta).
pub const MAX_ITEMS: usize = 4096;

/// Druh operace v zurnalu (stejne hodnoty jako `fs::jop`).
pub mod jop {
    pub const CREATE: u8 = 1;
    pub const REMOVE: u8 = 2;
    pub const RENAME: u8 = 3;
    pub const CONTENT: u8 = 4;
    pub const ATTR: u8 = 5;
    pub const TRASH: u8 = 6;
    pub const RESTORE: u8 = 7;
    pub const SNAPSHOT: u8 = 8;
}

/// Zaznam zurnalu zmen.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct JournalEntry {
    pub seq: u64,
    pub op: u8,
    pub kind: u8,
    pub time: WallTime,
    pub actor: u64,
    pub oid: u64,
    pub generation: u64,
    pub parent: u64,
    pub new_parent: u64,
    pub size: u64,
    pub aux: u64,
    pub name: String,
    pub new_name: String,
}

impl JournalEntry {
    pub fn put(&self, w: &mut Writer) {
        w.u64(self.seq).u8(self.op).u8(self.kind);
        put_time(w, self.time);
        for v in [self.actor, self.oid, self.generation, self.parent, self.new_parent, self.size, self.aux] {
            w.u64(v);
        }
        w.str(&self.name).str(&self.new_name);
    }

    pub fn get(r: &mut Reader) -> Result<Self> {
        let (seq, op, kind, time) = (r.u64()?, r.u8()?, r.u8()?, get_time(r)?);
        let mut v = [0u64; 7];
        for x in &mut v {
            *x = r.u64()?;
        }
        Ok(Self {
            seq,
            op,
            kind,
            time,
            actor: v[0],
            oid: v[1],
            generation: v[2],
            parent: v[3],
            new_parent: v[4],
            size: v[5],
            aux: v[6],
            name: r.str()?,
            new_name: r.str()?,
        })
    }
}

/// Stranka zurnalu.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct JournalPage {
    pub first: u64,
    pub last: u64,
    pub entries: Vec<JournalEntry>,
}

impl JournalPage {
    pub fn put(&self, w: &mut Writer) {
        w.u64(self.first).u64(self.last).u32(self.entries.len() as u32);
        for e in &self.entries {
            e.put(w);
        }
    }

    pub fn get(r: &mut Reader) -> Result<Self> {
        let (first, last, n) = (r.u64()?, r.u64()?, r.u32()? as usize);
        if n > MAX_ITEMS {
            return Err(Error::InvalidArgs);
        }
        let entries = (0..n).map(|_| JournalEntry::get(r)).collect::<Result<_>>()?;
        Ok(Self { first, last, entries })
    }
}

/// Pohled na dataset (docasny snapshot) a pozice v zurnalu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct View {
    pub snapshot: String,
    pub id: u64,
    pub seq: u64,
}

/// Polozka kose.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrashEntry {
    pub id: u64,
    pub kind: u8,
    pub oid: u64,
    pub time: WallTime,
    pub actor: u64,
    pub size: u64,
    pub name: String,
    pub original_dir: String,
}

impl TrashEntry {
    pub fn put(&self, w: &mut Writer) {
        w.u64(self.id).u8(self.kind).u64(self.oid);
        put_time(w, self.time);
        w.u64(self.actor).u64(self.size).str(&self.name).str(&self.original_dir);
    }

    pub fn get(r: &mut Reader) -> Result<Self> {
        Ok(Self {
            id: r.u64()?,
            kind: r.u8()?,
            oid: r.u64()?,
            time: get_time(r)?,
            actor: r.u64()?,
            size: r.u64()?,
            name: r.str()?,
            original_dir: r.str()?,
        })
    }
}

/// Vysledek vraceni operace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Undone {
    MovedBack(String),
    Restored(String),
    Trashed(u64),
    FromSnapshot { path: String, snapshot: String, trashed: Option<u64> },
}

impl Undone {
    pub fn put(&self, w: &mut Writer) {
        match self {
            Undone::MovedBack(p) => w.u8(1).str(p).str("").u64(0),
            Undone::Restored(p) => w.u8(2).str(p).str("").u64(0),
            Undone::Trashed(id) => w.u8(3).str("").str("").u64(*id),
            Undone::FromSnapshot { path, snapshot, trashed } => w.u8(4).str(path).str(snapshot).u64(trashed.unwrap_or(0)),
        };
    }

    pub fn get(r: &mut Reader) -> Result<Self> {
        let (k, path, snapshot, id) = (r.u8()?, r.str()?, r.str()?, r.u64()?);
        Ok(match k {
            1 => Undone::MovedBack(path),
            2 => Undone::Restored(path),
            3 => Undone::Trashed(id),
            4 => Undone::FromSnapshot { path, snapshot, trashed: (id != 0).then_some(id) },
            _ => return Err(Error::InvalidArgs),
        })
    }
}

/// Verze souboru ve snapshotu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileVersion {
    pub snapshot: String,
    pub snapshot_id: u64,
    pub time: WallTime,
    pub size: u64,
    pub mtime: WallTime,
    pub version: u64,
}

impl FileVersion {
    pub fn put(&self, w: &mut Writer) {
        w.str(&self.snapshot).u64(self.snapshot_id);
        put_time(w, self.time);
        w.u64(self.size);
        put_time(w, self.mtime);
        w.u64(self.version);
    }

    pub fn get(r: &mut Reader) -> Result<Self> {
        Ok(Self {
            snapshot: r.str()?,
            snapshot_id: r.u64()?,
            time: get_time(r)?,
            size: r.u64()?,
            mtime: get_time(r)?,
            version: r.u64()?,
        })
    }
}

/// Pravidlo automatickych snapshotu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AutoRule {
    pub interval: u64,
    pub keep: u32,
}

/// Nastaveni datasetu (0 = vychozi, `u64::MAX` = bez limitu).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Settings {
    pub journal_max_entries: u64,
    pub journal_max_age: u64,
    pub trash_max_age: u64,
    pub rules: Vec<AutoRule>,
}

impl Settings {
    pub fn put(&self, w: &mut Writer) {
        w.u64(self.journal_max_entries).u64(self.journal_max_age).u64(self.trash_max_age).u32(self.rules.len() as u32);
        for r in &self.rules {
            w.u64(r.interval).u32(r.keep);
        }
    }

    pub fn get(r: &mut Reader) -> Result<Self> {
        let (a, b, c, n) = (r.u64()?, r.u64()?, r.u64()?, r.u32()? as usize);
        if n > 8 {
            return Err(Error::InvalidArgs);
        }
        let rules = (0..n).map(|_| Ok(AutoRule { interval: r.u64()?, keep: r.u32()? })).collect::<Result<_>>()?;
        Ok(Self { journal_max_entries: a, journal_max_age: b, trash_max_age: c, rules })
    }
}

/// Logicke ID commitu (BLAKE3).
pub type CommitId = [u8; 32];

/// Commit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitInfo {
    pub id: CommitId,
    pub snapshot: String,
    pub dataset: u64,
    pub snapshot_id: u64,
    pub time: WallTime,
    pub author: u64,
    pub message: String,
    pub parents: Vec<CommitId>,
}

fn get_id(r: &mut Reader) -> Result<CommitId> {
    let mut id = [0u8; 32];
    for chunk in id.chunks_mut(8) {
        chunk.copy_from_slice(&r.u64()?.to_le_bytes());
    }
    Ok(id)
}

fn put_id(w: &mut Writer, id: &CommitId) {
    for c in id.chunks(8) {
        w.u64(u64::from_le_bytes(c.try_into().unwrap()));
    }
}

impl CommitInfo {
    pub fn put(&self, w: &mut Writer) {
        put_id(w, &self.id);
        w.str(&self.snapshot).u64(self.dataset).u64(self.snapshot_id);
        put_time(w, self.time);
        w.u64(self.author).str(&self.message).u32(self.parents.len() as u32);
        for p in &self.parents {
            put_id(w, p);
        }
    }

    pub fn get(r: &mut Reader) -> Result<Self> {
        let id = get_id(r)?;
        let (snapshot, dataset, snapshot_id, time) = (r.str()?, r.u64()?, r.u64()?, get_time(r)?);
        let (author, message, n) = (r.u64()?, r.str()?, r.u32()? as usize);
        if n > 8 {
            return Err(Error::InvalidArgs);
        }
        let parents = (0..n).map(|_| get_id(r)).collect::<Result<_>>()?;
        Ok(Self { id, snapshot, dataset, snapshot_id, time, author, message, parents })
    }
}

/// Vetev.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Branch {
    pub name: String,
    pub dataset: u64,
    pub path: String,
}

/// Druh konfliktu (stejne poradi jako `fs::ConflictKind`).
pub mod conflict {
    pub const CONTENT: u8 = 1;
    pub const MODIFIED_DELETED: u8 = 2;
    pub const DELETED_MODIFIED: u8 = 3;
    pub const BOTH_ADDED: u8 = 4;
    pub const BOTH_MOVED: u8 = 5;
    pub const PARENT_MISSING: u8 = 6;
    pub const DIR_NOT_EMPTY: u8 = 7;
    pub const CYCLE: u8 = 8;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MergeConflict {
    pub path: String,
    pub kind: u8,
    pub resolved_as: String,
}

/// Vysledek merge nebo prevzeti zmen.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MergeReport {
    pub applied: Vec<String>,
    pub conflicts: Vec<MergeConflict>,
    pub commit: Option<CommitInfo>,
}

impl MergeReport {
    pub fn put(&self, w: &mut Writer) {
        // Do odpovedi se vejde omezeny pocet cest (zbytek se zkrati).
        let applied = self.applied.len().min(MAX_ITEMS / 4);
        w.u32(applied as u32);
        for p in &self.applied[..applied] {
            w.str(p);
        }
        let conflicts = self.conflicts.len().min(MAX_ITEMS / 4);
        w.u32(conflicts as u32);
        for c in &self.conflicts[..conflicts] {
            w.str(&c.path).u8(c.kind).str(&c.resolved_as);
        }
        match &self.commit {
            Some(c) => {
                w.u8(1);
                c.put(w);
            }
            None => {
                w.u8(0);
            }
        }
    }

    pub fn get(r: &mut Reader) -> Result<Self> {
        let n = r.u32()? as usize;
        if n > MAX_ITEMS {
            return Err(Error::InvalidArgs);
        }
        let applied = (0..n).map(|_| r.str()).collect::<Result<_>>()?;
        let n = r.u32()? as usize;
        if n > MAX_ITEMS {
            return Err(Error::InvalidArgs);
        }
        let conflicts = (0..n)
            .map(|_| Ok(MergeConflict { path: r.str()?, kind: r.u8()?, resolved_as: r.str()? }))
            .collect::<Result<_>>()?;
        let commit = match r.u8()? {
            0 => None,
            _ => Some(CommitInfo::get(r)?),
        };
        Ok(Self { applied, conflicts, commit })
    }
}

/// Zmena v klonu proti puvodu (`ASIDE_CHANGES`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    /// 1 pridano, 2 smazano, 3 zmeneno, 4 presunuto, 5 presunuto a zmeneno.
    pub kind: u8,
    pub path: String,
    pub old_path: String,
    pub oid: u64,
    pub is_dir: bool,
}

pub fn put_opt_time(w: &mut Writer, t: Option<WallTime>) {
    match t {
        Some(t) => {
            w.u8(1);
            put_time(w, t);
        }
        None => {
            w.u8(0);
            put_time(w, WallTime::UNIX_EPOCH);
        }
    }
}

pub fn get_opt_time(r: &mut Reader) -> Result<Option<WallTime>> {
    let has = r.u8()? != 0;
    let t = get_time(r)?;
    Ok(has.then_some(t))
}

fn list<T>(body: &[u8], f: impl Fn(&mut Reader) -> Result<T>) -> Result<Vec<T>> {
    let mut r = Reader::new(body);
    let n = r.u32()? as usize;
    if n > MAX_ITEMS {
        return Err(Error::InvalidArgs);
    }
    (0..n).map(|_| f(&mut r)).collect()
}

impl FsClient {
    pub fn journal_read(&self, dataset: &str, from: u64, max: u32) -> Result<JournalPage> {
        let mut w = Self::req(op::JOURNAL_READ);
        w.str(dataset).u64(from).u32(max);
        JournalPage::get(&mut Reader::new(&self.call(w)?))
    }

    /// Ceka na zaznam od `from` nejvic `timeout_ms` (prazdna stranka = limit).
    pub fn journal_watch(&self, dataset: &str, from: u64, max: u32, timeout_ms: u32) -> Result<JournalPage> {
        let mut w = Self::req(op::JOURNAL_WATCH);
        w.str(dataset).u64(from).u32(max).u32(timeout_ms);
        JournalPage::get(&mut Reader::new(&self.call(w)?))
    }

    pub fn view(&self, dataset: &str) -> Result<View> {
        let mut w = Self::req(op::VIEW);
        w.str(dataset);
        let body = self.call(w)?;
        let mut r = Reader::new(&body);
        Ok(View { snapshot: r.str()?, id: r.u64()?, seq: r.u64()? })
    }

    pub fn release_view(&self, dataset: &str, snapshot: &str) -> Result<()> {
        let mut w = Self::req(op::VIEW_RELEASE);
        w.str(dataset).str(snapshot);
        self.call(w).map(|_| ())
    }

    pub fn trash(&self, path: &str) -> Result<u64> {
        Reader::new(&self.path_op(op::TRASH, path)?).u64()
    }

    pub fn trash_list(&self, dataset: &str) -> Result<Vec<TrashEntry>> {
        list(&self.path_op(op::TRASH_LIST, dataset)?, TrashEntry::get)
    }

    /// `dest = None` = na puvodni misto.
    pub fn trash_restore(&self, dataset: &str, id: u64, dest: Option<&str>) -> Result<String> {
        let mut w = Self::req(op::TRASH_RESTORE);
        w.str(dataset).u64(id).str(dest.unwrap_or(""));
        Reader::new(&self.call(w)?).str()
    }

    pub fn trash_purge(&self, dataset: &str, id: u64) -> Result<()> {
        let mut w = Self::req(op::TRASH_PURGE);
        w.str(dataset).u64(id);
        self.call(w).map(|_| ())
    }

    pub fn undo(&self, dataset: &str, seq: u64) -> Result<Undone> {
        let mut w = Self::req(op::UNDO);
        w.str(dataset).u64(seq);
        Undone::get(&mut Reader::new(&self.call(w)?))
    }

    pub fn file_versions(&self, path: &str) -> Result<Vec<FileVersion>> {
        list(&self.path_op(op::FILE_VERSIONS, path)?, FileVersion::get)
    }

    /// `dest = None` = nahradit aktualni (aktualni do kose).
    pub fn restore_version(&self, path: &str, snapshot: &str, dest: Option<&str>) -> Result<String> {
        let mut w = Self::req(op::RESTORE_VERSION);
        w.str(path).str(snapshot).u8(dest.is_none() as u8).str(dest.unwrap_or(""));
        Reader::new(&self.call(w)?).str()
    }

    pub fn settings(&self, dataset: &str) -> Result<Settings> {
        Settings::get(&mut Reader::new(&self.path_op(op::SETTINGS, dataset)?))
    }

    pub fn set_settings(&self, dataset: &str, s: &Settings) -> Result<()> {
        let mut w = Self::req(op::SET_SETTINGS);
        w.str(dataset);
        s.put(&mut w);
        self.call(w).map(|_| ())
    }

    pub fn commit_version(&self, dataset: &str, message: &str) -> Result<CommitInfo> {
        let mut w = Self::req(op::COMMIT_VERSION);
        w.str(dataset).str(message);
        CommitInfo::get(&mut Reader::new(&self.call(w)?))
    }

    pub fn head(&self, dataset: &str) -> Result<Option<CommitInfo>> {
        let body = self.path_op(op::HEAD, dataset)?;
        let mut r = Reader::new(&body);
        if r.u8()? == 0 { Ok(None) } else { Ok(Some(CommitInfo::get(&mut r)?)) }
    }

    pub fn log(&self, dataset: &str, max: u32) -> Result<Vec<CommitInfo>> {
        let mut w = Self::req(op::LOG);
        w.str(dataset).u32(max);
        list(&self.call(w)?, CommitInfo::get)
    }

    pub fn branch(&self, dataset: &str, name: &str, path: &str) -> Result<u64> {
        let mut w = Self::req(op::BRANCH);
        w.str(dataset).str(name).str(path);
        Reader::new(&self.call(w)?).u64()
    }

    pub fn branches(&self, dataset: &str) -> Result<Vec<Branch>> {
        list(&self.path_op(op::BRANCHES, dataset)?, |r| Ok(Branch { name: r.str()?, dataset: r.u64()?, path: r.str()? }))
    }

    pub fn delete_branch(&self, dataset: &str, name: &str) -> Result<()> {
        let mut w = Self::req(op::DELETE_BRANCH);
        w.str(dataset).str(name);
        self.call(w).map(|_| ())
    }

    pub fn merge(&self, target: &str, source: &str) -> Result<MergeReport> {
        let mut w = Self::req(op::MERGE);
        w.str(target).str(source);
        MergeReport::get(&mut Reader::new(&self.call(w)?))
    }

    pub fn merge_abort(&self, target: &str) -> Result<()> {
        self.path_op(op::MERGE_ABORT, target).map(|_| ())
    }

    pub fn aside_begin(&self, dataset: &str, path: &str) -> Result<u64> {
        let mut w = Self::req(op::ASIDE_BEGIN);
        w.str(dataset).str(path);
        Reader::new(&self.call(w)?).u64()
    }

    pub fn aside_changes(&self, clone: &str) -> Result<Vec<Change>> {
        list(&self.path_op(op::ASIDE_CHANGES, clone)?, |r| {
            Ok(Change { kind: r.u8()?, path: r.str()?, old_path: r.str()?, oid: r.u64()?, is_dir: r.u8()? != 0 })
        })
    }

    /// `select = None` = prevzit vse.
    pub fn aside_apply(&self, clone: &str, select: Option<&[&str]>) -> Result<MergeReport> {
        let mut w = Self::req(op::ASIDE_APPLY);
        w.str(clone).u8(select.is_none() as u8);
        let sel = select.unwrap_or(&[]);
        w.u32(sel.len() as u32);
        for p in sel {
            w.str(p);
        }
        MergeReport::get(&mut Reader::new(&self.call(w)?))
    }

    pub fn aside_discard(&self, clone: &str) -> Result<()> {
        self.path_op(op::ASIDE_DISCARD, clone).map(|_| ())
    }

    pub fn get_xattr(&self, path: &str, name: &str) -> Result<Vec<u8>> {
        let mut w = Self::req(op::XATTR_GET);
        w.str(path).str(name);
        self.call(w)
    }

    pub fn set_xattr(&self, path: &str, name: &str, value: &[u8]) -> Result<()> {
        let mut w = Self::req(op::XATTR_SET);
        w.str(path).str(name).bytes(value);
        self.call(w).map(|_| ())
    }

    pub fn remove_xattr(&self, path: &str, name: &str) -> Result<()> {
        let mut w = Self::req(op::XATTR_REMOVE);
        w.str(path).str(name);
        self.call(w).map(|_| ())
    }

    pub fn list_xattrs(&self, path: &str) -> Result<Vec<String>> {
        list(&self.path_op(op::XATTR_LIST, path)?, |r| r.str())
    }

    pub fn set_expires(&self, path: &str, when: Option<WallTime>) -> Result<()> {
        let mut w = Self::req(op::SET_EXPIRES);
        w.str(path);
        put_opt_time(&mut w, when);
        self.call(w).map(|_| ())
    }

    /// Nova slozka s vlastni historii (dataset) na ceste `path`.
    pub fn create_dataset(&self, path: &str) -> Result<u64> {
        Reader::new(&self.path_op(op::DATASET_CREATE, path)?).u64()
    }

    pub fn delete_dataset(&self, path: &str) -> Result<()> {
        self.path_op(op::DATASET_DELETE, path).map(|_| ())
    }

    pub fn expires(&self, path: &str) -> Result<Option<WallTime>> {
        get_opt_time(&mut Reader::new(&self.path_op(op::EXPIRES, path)?))
    }
}
