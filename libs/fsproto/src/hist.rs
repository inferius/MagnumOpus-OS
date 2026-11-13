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

use crate::{DirEntry, FsClient, MAX_IO, Reader, Stat, Writer, decode_list, get_time, put_time};

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
    /// dataset, cesta klonu str -> dataset klonu u64, snapshot puvodu str,
    /// zurnal seq u64 (posledni zaznam ve snapshotu, jako `View::seq`)
    pub const ASIDE_BEGIN: u32 = 74;
    /// klon, vse u8, pocet u32 + cesty str, [priznaky u8 `aside_flags`]
    /// -> `MergeReport`
    pub const ASIDE_APPLY: u32 = 75;
    /// klon; snapshot puvodu smaze, jen kdyz neni ponechany jako pohled
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
    /// dataset, od snapshot str, do u8 (0 = ziva verze, 1 = nasleduje
    /// jmeno snapshotu str) -> pocet u32 + `Change` (cesty od korene
    /// datasetu, serazene). Vyzaduje `MANAGE` ke koreni datasetu. Snapshot
    /// muze byt i docasny snapshot pohledu (`VIEW`). Kdyz se rozdil nevejde
    /// do odpovedi (`MAX_ITEMS` polozek nebo `MAX_REPLY` bajtu), vrati
    /// `BUFFER_TOO_SMALL` (nestrankuje se: rozdil se pocita cely).
    pub const DIFF: u32 = 88;
    /// cesta -> cesta korene datasetu, ve kterem `cesta` lezi (str; cesta
    /// relace). Prava jako `STAT` cesty: cesta, kterou relace nevidi,
    /// i koren datasetu mimo pripojene cesty relace = `NOT_FOUND`.
    pub const DATASET_OF: u32 = 89;
    /// Cteni snapshotu libovolneho datasetu (vcetne docasneho snapshotu
    /// pohledu z `VIEW`); `SNAP_STAT` / `SNAP_LIST_PAGE` / `SNAP_READ` umi
    /// jen korenovy dataset. Dataset je cesta relace, cesta ve snapshotu je
    /// cesta uvnitr datasetu (`/` = koren datasetu). Prava jako cteni zive
    /// verze (pruchod k datasetu a ACL ve snapshotu kontroluje FS); relace
    /// musi mit dataset pripojeny (staci jen pro cteni).
    ///
    /// dataset, snapshot str, cesta -> `Stat`
    pub const SNAP_STAT_OF: u32 = 90;
    /// dataset, snapshot str, cesta, kurzor str, nejvic u32 -> jako
    /// `LIST_PAGE` (dalsi u8, pocet u32, polozky)
    pub const SNAP_LIST_OF: u32 = 91;
    /// dataset, snapshot str, cesta, offset u64, delka u32 (nejvic `MAX_IO`)
    /// -> data (kratsi na konci souboru)
    pub const SNAP_READ_OF: u32 = 92;
    // 93 = `DATASETS` (modul `crate::datasets`).
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

/// Priznaky `ASIDE_APPLY`.
pub mod aside_flags {
    /// Po prevzeti ponechat snapshot puvodu (stav pred radkem) jako pohled
    /// pro undo; uvolni ho `VIEW_RELEASE` (ne `ASIDE_DISCARD`).
    pub const KEEP_VIEW: u8 = 1;
}

/// Odpoved `ASIDE_BEGIN`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AsideBegun {
    /// Cislo datasetu klonu.
    pub dataset: u64,
    /// Snapshot puvodu v puvodnim datasetu (s `KEEP_VIEW` pohled pro undo).
    pub snapshot: String,
    /// Posledni zaznam zurnalu, ktery snapshot obsahuje.
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
    /// Kolik konfliktu bylo celkem (u klienta; `conflicts` muze byt zkraceny
    /// na `MAX_ITEMS / 4`). Pri kodovani se bere `conflicts.len()`.
    pub conflicts_total: usize,
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
        // Celkovy pocet: klient pozna, ze seznam je zkraceny.
        w.u32(self.conflicts.len().min(u32::MAX as usize) as u32);
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
        let conflicts: Vec<MergeConflict> = (0..n)
            .map(|_| Ok(MergeConflict { path: r.str()?, kind: r.u8()?, resolved_as: r.str()? }))
            .collect::<Result<_>>()?;
        let conflicts_total = (r.u32()? as usize).max(conflicts.len());
        let commit = match r.u8()? {
            0 => None,
            _ => Some(CommitInfo::get(r)?),
        };
        Ok(Self { applied, conflicts, commit, conflicts_total })
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

impl Change {
    /// Velikost na drate bez cest (druh, 2x delka u16, oid, slozka).
    pub const FIXED: usize = 1 + 2 + 2 + 8 + 1;

    pub fn put(&self, w: &mut Writer) {
        w.u8(self.kind).str(&self.path).str(&self.old_path).u64(self.oid).u8(self.is_dir as u8);
    }

    pub fn get(r: &mut Reader) -> Result<Self> {
        Ok(Self { kind: r.u8()?, path: r.str()?, old_path: r.str()?, oid: r.u64()?, is_dir: r.u8()? != 0 })
    }
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

/// Pozadavek `DIFF`.
pub fn diff_request(dataset: &str, from: &str, to: Option<&str>, skip: u32) -> Writer {
    let mut w = FsClient::req(op::DIFF);
    w.str(dataset).str(from);
    match to {
        Some(t) => {
            w.u8(1).str(t);
        }
        None => {
            w.u8(0);
        }
    }
    w.u32(skip);
    w
}

/// Precte parametry `DIFF` (za kodem operace) -> (dataset, od, do,
/// preskocit). `skip` chybi = 0.
pub fn get_diff_request(r: &mut Reader) -> Result<(String, String, Option<String>, usize)> {
    let (ds, from) = (r.str()?, r.str()?);
    let to = match r.u8()? {
        0 => None,
        1 => Some(r.str()?),
        _ => return Err(Error::InvalidArgs),
    };
    Ok((ds, from, to, get_skip(r)?))
}

/// Volitelny posun stranky na konci pozadavku (`DIFF`, `ASIDE_CHANGES`).
pub fn get_skip(r: &mut Reader) -> Result<usize> {
    if r.is_empty() { Ok(0) } else { Ok(r.u32()? as usize) }
}

/// Zakoduje stranku zmen (`DIFF`, `ASIDE_CHANGES`) od `skip`: celkem u32,
/// pocet u32, zmeny - kolik se vejde do `MAX_ITEMS` a `budget` bajtu.
/// Seznam se nikdy tise nezkrati: klient si vyzada dalsi stranky.
pub fn encode_changes_page(w: &mut Writer, changes: &[Change], skip: usize, budget: usize) {
    let rest = changes.get(skip..).unwrap_or(&[]);
    let (mut size, mut n) = (8usize, 0usize);
    for c in rest {
        let one = Change::FIXED + c.path.len() + c.old_path.len();
        // Aspon jedna polozka na stranku (cesta je vzdy mensi nez zprava).
        if n >= MAX_ITEMS || (n > 0 && size + one > budget) {
            break;
        }
        size += one;
        n += 1;
    }
    w.u32(changes.len().min(u32::MAX as usize) as u32).u32(n as u32);
    for c in &rest[..n] {
        c.put(w);
    }
}

/// Precte vsechny stranky zmen; `req(skip)` = pozadavek na stranku.
fn changes_pages(f: &FsClient, mut req: impl FnMut(u32) -> Writer) -> Result<Vec<Change>> {
    let mut out: Vec<Change> = Vec::new();
    loop {
        let body = f.call(req(out.len() as u32))?;
        let mut r = Reader::new(&body);
        let total = r.u32()? as usize;
        let n = r.u32()? as usize;
        if n > MAX_ITEMS {
            return Err(Error::InvalidArgs);
        }
        for _ in 0..n {
            out.push(Change::get(&mut r)?);
        }
        if out.len() >= total || n == 0 {
            return Ok(out);
        }
    }
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
        Ok(self.aside_begin_ex(dataset, path)?.dataset)
    }

    /// Jako `aside_begin`, navic snapshot puvodu a pozice zurnalu (undo bez
    /// `VIEW`: `aside_apply_keep`).
    pub fn aside_begin_ex(&self, dataset: &str, path: &str) -> Result<AsideBegun> {
        let mut w = Self::req(op::ASIDE_BEGIN);
        w.str(dataset).str(path);
        let body = self.call(w)?;
        let mut r = Reader::new(&body);
        Ok(AsideBegun { dataset: r.u64()?, snapshot: r.str()?, seq: r.u64()? })
    }

    /// Vsechny zmeny klonu (po strankach, nikdy zkracene).
    pub fn aside_changes(&self, clone: &str) -> Result<Vec<Change>> {
        changes_pages(self, |skip| {
            let mut w = Self::req(op::ASIDE_CHANGES);
            w.str(clone).u32(skip);
            w
        })
    }

    /// Rozdil verzi datasetu: ze snapshotu `from` do snapshotu `to`, nebo do
    /// zive verze (`to = None`). Cesty od korene datasetu.
    pub fn diff(&self, dataset: &str, from: &str, to: Option<&str>) -> Result<Vec<Change>> {
        changes_pages(self, |skip| diff_request(dataset, from, to, skip))
    }

    /// `stat` ve snapshotu `snap` datasetu `dataset` (cesta uvnitr datasetu).
    pub fn snapshot_stat_of(&self, dataset: &str, snap: &str, path: &str) -> Result<Stat> {
        let mut w = Self::req(op::SNAP_STAT_OF);
        w.str(dataset).str(snap).str(path);
        Stat::get(&mut Reader::new(&self.call(w)?))
    }

    /// Cely vypis adresare ve snapshotu datasetu (po strankach).
    pub fn snapshot_list_of(&self, dataset: &str, snap: &str, path: &str) -> Result<Vec<DirEntry>> {
        let mut out: Vec<DirEntry> = Vec::new();
        loop {
            let mut w = Self::req(op::SNAP_LIST_OF);
            let after = out.last().map_or("", |e| e.name.as_str());
            w.str(dataset).str(snap).str(path).str(after).u32(u32::MAX);
            let body = self.call(w)?;
            let mut r = Reader::new(&body);
            let more = r.u8()? != 0;
            let page = decode_list(r.rest())?;
            let empty = page.is_empty();
            out.extend(page);
            if !more || empty {
                return Ok(out);
            }
        }
    }

    /// Precte az `len` bajtu od `offset` ze souboru ve snapshotu datasetu
    /// (vetsi pozadavek rozdeli po `MAX_IO`).
    pub fn snapshot_read_of(&self, dataset: &str, snap: &str, path: &str, offset: u64, len: usize) -> Result<Vec<u8>> {
        let mut out = Vec::new();
        while out.len() < len {
            let n = (len - out.len()).min(MAX_IO);
            let mut w = Self::req(op::SNAP_READ_OF);
            w.str(dataset).str(snap).str(path).u64(offset + out.len() as u64).u32(n as u32);
            let part = self.call(w)?;
            let got = part.len();
            out.extend_from_slice(&part);
            if got < n {
                break;
            }
        }
        Ok(out)
    }

    /// Cely soubor ze snapshotu datasetu (velikost podle `snapshot_stat_of`).
    pub fn snapshot_read_file_of(&self, dataset: &str, snap: &str, path: &str) -> Result<Vec<u8>> {
        let size = self.snapshot_stat_of(dataset, snap, path)?.size;
        let len = usize::try_from(size).map_err(|_| Error::NoMemory)?;
        self.snapshot_read_of(dataset, snap, path, 0, len)
    }

    /// Cesta korene datasetu, ve kterem lezi `path` (`/` = korenovy).
    pub fn dataset_of(&self, path: &str) -> Result<String> {
        Reader::new(&self.path_op(op::DATASET_OF, path)?).str()
    }

    /// `select = None` = prevzit vse.
    pub fn aside_apply(&self, clone: &str, select: Option<&[&str]>) -> Result<MergeReport> {
        self.aside_apply_flags(clone, select, 0)
    }

    /// Prevezme zmeny a snapshot puvodu ponecha jako pohled pro undo
    /// (`View { snapshot, seq }` z `aside_begin_ex`; uvolnit `release_view`).
    pub fn aside_apply_keep(&self, clone: &str, select: Option<&[&str]>) -> Result<MergeReport> {
        self.aside_apply_flags(clone, select, aside_flags::KEEP_VIEW)
    }

    fn aside_apply_flags(&self, clone: &str, select: Option<&[&str]>, flags: u8) -> Result<MergeReport> {
        let mut w = Self::req(op::ASIDE_APPLY);
        w.str(clone).u8(select.is_none() as u8);
        let sel = select.unwrap_or(&[]);
        w.u32(sel.len() as u32);
        for p in sel {
            w.str(p);
        }
        w.u8(flags);
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
