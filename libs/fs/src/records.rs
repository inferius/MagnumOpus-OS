//! Zaznamy historie na disku (faze 2): zurnal zmen, metadata snapshotu
//! a commitu, nastaveni datasetu, kos, rozsirene atributy, vetve.
//!
//! Vsechny lezi ve strome datasetu (vetsinou pod `META_OID`), takze se
//! ukladaji atomicky se zmenou, ke ktere patri, a snapshot je zachyti take.
//! Cela cisla little-endian; neplatna hodnota = `CORRUPTED`.

use alloc::string::String;
use alloc::vec::Vec;

use abi::{Error, Result};

use crate::layout::{Timestamp, u32_at, u64_at};

fn put_u64(b: &mut Vec<u8>, v: u64) {
    b.extend_from_slice(&v.to_le_bytes());
}

fn put_time(b: &mut Vec<u8>, t: Timestamp) {
    b.extend_from_slice(&t.secs.to_le_bytes());
    b.extend_from_slice(&t.nanos.to_le_bytes());
}

fn get_time(b: &[u8], o: usize) -> Result<Timestamp> {
    let nanos = u32_at(b, o + 8);
    if nanos >= 1_000_000_000 {
        return Err(Error::Corrupted);
    }
    Ok(Timestamp { secs: u64_at(b, o) as i64, nanos })
}

fn utf8(b: &[u8]) -> Result<String> {
    Ok(String::from(core::str::from_utf8(b).map_err(|_| Error::Corrupted)?))
}

// --- zurnal zmen ---

/// Druh operace v zurnalu.
pub mod jop {
    /// Novy objekt (`parent`, `name`).
    pub const CREATE: u8 = 1;
    /// Trvale smazani (`parent`, `name`); u koseho objektu `parent` = kos.
    pub const REMOVE: u8 = 2;
    /// Presun nebo prejmenovani (`parent`, `name` -> `new_parent`, `new_name`).
    pub const RENAME: u8 = 3;
    /// Zmenil se obsah (slouceno za commit nebo fsync); `size` = nova velikost.
    pub const CONTENT: u8 = 4;
    /// Zmenila se metadata (`aux` = `jattr::*`).
    pub const ATTR: u8 = 5;
    /// Presun do kose (`parent`, `name`; `aux` = cislo zaznamu kose).
    pub const TRASH: u8 = 6;
    /// Obnova z kose (`new_parent`, `new_name`; `aux` = cislo zaznamu kose).
    pub const RESTORE: u8 = 7;
    /// Novy snapshot datasetu (`aux` = cislo snapshotu, `name` = jmeno).
    pub const SNAPSHOT: u8 = 8;
    pub const MAX: u8 = SNAPSHOT;
}

/// Co se zmenilo u `jop::ATTR`.
pub mod jattr {
    pub const TIMES: u64 = 1;
    pub const ACL: u64 = 2;
    pub const OWNER: u64 = 3;
    pub const XATTR: u64 = 4;
}

/// Zaznam zurnalu zmen.
///
/// ```text
/// 0   operace u8 (jop::*)      1   druh objektu u8 (KIND_*)
/// 2   rezerva u16              4   nanosekundy casu u32
/// 8   sekundy casu i64         16  kdo (uzivatel) u64
/// 24  aplikace u64 (0 = neznama; vyhrazeno)
/// 32  oid u64                  40  generace objektu u64
/// 48  rodic u64                56  novy rodic u64
/// 64  velikost u64             72  doplnek u64 (podle operace)
/// 80  delka jmena u16          82  delka noveho jmena u16
/// 84  jmeno, nove jmeno (UTF-8)
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct JournalEntry {
    pub op: u8,
    pub kind: u8,
    pub time: Timestamp,
    pub actor: u64,
    pub app: u64,
    pub oid: u64,
    pub generation: u64,
    pub parent: u64,
    pub new_parent: u64,
    pub size: u64,
    pub aux: u64,
    pub name: String,
    pub new_name: String,
}

const JE_HEADER: usize = 84;

impl JournalEntry {
    pub fn encode(&self) -> Vec<u8> {
        let mut b = Vec::with_capacity(JE_HEADER + self.name.len() + self.new_name.len());
        b.push(self.op);
        b.push(self.kind);
        b.extend_from_slice(&[0, 0]);
        b.extend_from_slice(&self.time.nanos.to_le_bytes());
        b.extend_from_slice(&self.time.secs.to_le_bytes());
        for v in [self.actor, self.app, self.oid, self.generation, self.parent, self.new_parent, self.size, self.aux] {
            put_u64(&mut b, v);
        }
        b.extend_from_slice(&(self.name.len() as u16).to_le_bytes());
        b.extend_from_slice(&(self.new_name.len() as u16).to_le_bytes());
        b.extend_from_slice(self.name.as_bytes());
        b.extend_from_slice(self.new_name.as_bytes());
        b
    }

    pub fn decode(b: &[u8]) -> Result<Self> {
        if b.len() < JE_HEADER || b[0] == 0 || b[0] > jop::MAX {
            return Err(Error::Corrupted);
        }
        let nanos = u32_at(b, 4);
        if nanos >= 1_000_000_000 {
            return Err(Error::Corrupted);
        }
        let nl = u16::from_le_bytes([b[80], b[81]]) as usize;
        let ml = u16::from_le_bytes([b[82], b[83]]) as usize;
        if b.len() != JE_HEADER + nl + ml {
            return Err(Error::Corrupted);
        }
        Ok(Self {
            op: b[0],
            kind: b[1],
            time: Timestamp { secs: u64_at(b, 8) as i64, nanos },
            actor: u64_at(b, 16),
            app: u64_at(b, 24),
            oid: u64_at(b, 32),
            generation: u64_at(b, 40),
            parent: u64_at(b, 48),
            new_parent: u64_at(b, 56),
            size: u64_at(b, 64),
            aux: u64_at(b, 72),
            name: utf8(&b[JE_HEADER..JE_HEADER + nl])?,
            new_name: utf8(&b[JE_HEADER + nl..])?,
        })
    }
}

/// Stav zurnalu datasetu: dalsi poradove cislo a nejstarsi ulozene.
/// Ulozene zaznamy maji cisla `first..next`.
///
/// ```text
/// 0 dalsi cislo u64   8 nejstarsi ulozene u64
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct JournalState {
    pub next: u64,
    pub first: u64,
}

impl Default for JournalState {
    fn default() -> Self {
        Self { next: 1, first: 1 }
    }
}

impl JournalState {
    pub fn encode(&self) -> Vec<u8> {
        let mut b = Vec::with_capacity(16);
        put_u64(&mut b, self.next);
        put_u64(&mut b, self.first);
        b
    }

    pub fn decode(b: &[u8]) -> Result<Self> {
        if b.len() != 16 {
            return Err(Error::Corrupted);
        }
        let s = Self { next: u64_at(b, 0), first: u64_at(b, 8) };
        if s.first == 0 || s.first > s.next {
            return Err(Error::Corrupted);
        }
        Ok(s)
    }
}

// --- metadata snapshotu a commitu ---

/// Druh snapshotu.
pub mod snapkind {
    /// Rucni (uzivatel, aplikace).
    pub const MANUAL: u8 = 0;
    /// Automaticky podle pravidel datasetu (smi ho prorezat `auto_snapshot_tick`).
    pub const AUTO: u8 = 1;
    /// Verzovany commit (autor, zprava, rodice, logicke ID).
    pub const COMMIT: u8 = 2;
    /// Docasny pohled (`Fs::view`): skryty ve vypisu, smaze se pri
    /// uvolneni nebo pri pripojeni po padu.
    pub const VIEW: u8 = 3;
    /// Vychozi bod "zkusit zmeny bokem" (klon z nej je pracovni kopie).
    pub const ASIDE: u8 = 4;
    pub const MAX: u8 = ASIDE;
}

/// Logicke ID commitu (BLAKE3, 32 B). Nuly = nespocitane.
pub type CommitId = [u8; 32];

/// Rodic commitu: logicke ID a kde lezi lokalne (dataset, snapshot;
/// snapshot uz nemusi existovat).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Parent {
    pub id: CommitId,
    pub dataset: u64,
    pub snapshot: u64,
}

/// Nejvic rodicu commitu (merge vic vetvi najednou).
pub const MAX_PARENTS: usize = 8;
/// Nejdelsi zprava commitu (bajty UTF-8).
pub const MAX_MESSAGE: usize = 4096;

/// Metadata snapshotu.
///
/// ```text
/// 0   druh u8 (snapkind::*)    1   pocet rodicu u8
/// 2   rezerva u16              4   nanosekundy vzniku u32
/// 8   sekundy vzniku i64       16  posledni zahrnuty zaznam zurnalu u64
/// 24  logicke ID (32 B)        56  autor u64
/// 64  delka zpravy u32         68  rezerva u32
/// 72  rodice: (ID 32 B, dataset u64, snapshot u64) * pocet
/// ..  zprava (UTF-8)
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SnapMeta {
    pub kind: u8,
    pub time: Timestamp,
    pub journal_seq: u64,
    pub id: CommitId,
    pub author: u64,
    pub parents: Vec<Parent>,
    pub message: String,
}

const SM_HEADER: usize = 72;
const PARENT_SIZE: usize = 48;

impl SnapMeta {
    pub fn encode(&self) -> Vec<u8> {
        let mut b = Vec::with_capacity(SM_HEADER + self.parents.len() * PARENT_SIZE + self.message.len());
        b.push(self.kind);
        b.push(self.parents.len() as u8);
        b.extend_from_slice(&[0, 0]);
        b.extend_from_slice(&self.time.nanos.to_le_bytes());
        b.extend_from_slice(&self.time.secs.to_le_bytes());
        put_u64(&mut b, self.journal_seq);
        b.extend_from_slice(&self.id);
        put_u64(&mut b, self.author);
        b.extend_from_slice(&(self.message.len() as u32).to_le_bytes());
        b.extend_from_slice(&[0; 4]);
        for p in &self.parents {
            b.extend_from_slice(&p.id);
            put_u64(&mut b, p.dataset);
            put_u64(&mut b, p.snapshot);
        }
        b.extend_from_slice(self.message.as_bytes());
        b
    }

    pub fn decode(b: &[u8]) -> Result<Self> {
        if b.len() < SM_HEADER || b[0] > snapkind::MAX || b[1] as usize > MAX_PARENTS {
            return Err(Error::Corrupted);
        }
        let n = b[1] as usize;
        let ml = u32_at(b, 64) as usize;
        if ml > MAX_MESSAGE || b.len() != SM_HEADER + n * PARENT_SIZE + ml {
            return Err(Error::Corrupted);
        }
        let nanos = u32_at(b, 4);
        if nanos >= 1_000_000_000 {
            return Err(Error::Corrupted);
        }
        let parents = (0..n)
            .map(|i| {
                let o = SM_HEADER + i * PARENT_SIZE;
                Parent {
                    id: b[o..o + 32].try_into().unwrap(),
                    dataset: u64_at(b, o + 32),
                    snapshot: u64_at(b, o + 40),
                }
            })
            .collect();
        Ok(Self {
            kind: b[0],
            time: Timestamp { secs: u64_at(b, 8) as i64, nanos },
            journal_seq: u64_at(b, 16),
            id: b[24..56].try_into().unwrap(),
            author: u64_at(b, 56),
            parents,
            message: utf8(&b[SM_HEADER + n * PARENT_SIZE..])?,
        })
    }
}

/// Posledni commit datasetu (`head`) a rozpracovany merge (`merging`:
/// druhy rodic pristiho commitu, dokud uzivatel neresi konflikty).
///
/// ```text
/// 0 head: ID 32 B, dataset u64, snapshot u64     48 pro dataset u64
/// 56 merging (48 B, nuly = zadny)
/// ```
///
/// `dataset` = pro ktery dataset zaznam plati: klon zdedi zaznam puvodu ve
/// svem strome (jiny dataset) - ten pak neplati a posledni commit klonu je
/// jeho puvodni snapshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Head {
    pub head: Option<Parent>,
    pub merging: Option<Parent>,
    pub dataset: u64,
}

const HEAD_SIZE: usize = 104;

fn put_parent(b: &mut Vec<u8>, p: Option<Parent>) {
    let p = p.unwrap_or_default();
    b.extend_from_slice(&p.id);
    put_u64(b, p.dataset);
    put_u64(b, p.snapshot);
}

fn get_parent(b: &[u8], o: usize) -> Option<Parent> {
    let p = Parent {
        id: b[o..o + 32].try_into().unwrap(),
        dataset: u64_at(b, o + 32),
        snapshot: u64_at(b, o + 40),
    };
    (p.dataset != 0).then_some(p)
}

impl Head {
    pub fn encode(&self) -> Vec<u8> {
        let mut b = Vec::with_capacity(HEAD_SIZE);
        put_parent(&mut b, self.head);
        put_u64(&mut b, self.dataset);
        put_parent(&mut b, self.merging);
        b
    }

    pub fn decode(b: &[u8]) -> Result<Self> {
        if b.len() != HEAD_SIZE {
            return Err(Error::Corrupted);
        }
        Ok(Self { head: get_parent(b, 0), merging: get_parent(b, 56), dataset: u64_at(b, 48) })
    }
}

// --- nastaveni datasetu ---

/// Pravidlo automatickych snapshotu: drzet nejnovejsi snapshot v kazdem
/// z poslednich `keep` intervalu delky `interval` sekund.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AutoRule {
    pub interval: u64,
    pub keep: u32,
}

/// Nejvic pravidel automatickych snapshotu.
pub const MAX_RULES: usize = 8;
/// Nejkratsi interval automatickych snapshotu (s).
pub const MIN_INTERVAL: u64 = 60;
pub const DEFAULT_JOURNAL_ENTRIES: u64 = 1_000_000;
pub const DEFAULT_JOURNAL_AGE: u64 = 30 * 86_400;
pub const DEFAULT_TRASH_AGE: u64 = 30 * 86_400;

/// Nastaveni datasetu. FS je uklada atomicky, rozhoduji podle nich kroky
/// `auto_snapshot_tick`, `trash_tick` a uklid zurnalu pri commitu.
/// Hodnota 0 = vychozi, `u64::MAX` = bez limitu.
///
/// ```text
/// 0  verze u32 (1)               4  pocet pravidel u32
/// 8  zurnal: nejvic zaznamu u64  16 zurnal: nejvic stari (s) u64
/// 24 kos: nejvic stari (s) u64
/// 32 pravidla: (interval s u64, kolik u32, rezerva u32) * pocet
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Settings {
    pub journal_max_entries: u64,
    pub journal_max_age: u64,
    pub trash_max_age: u64,
    pub rules: Vec<AutoRule>,
}

const SET_HEADER: usize = 32;

impl Settings {
    pub fn journal_entries(&self) -> u64 {
        if self.journal_max_entries == 0 { DEFAULT_JOURNAL_ENTRIES } else { self.journal_max_entries }
    }

    pub fn journal_age(&self) -> u64 {
        if self.journal_max_age == 0 { DEFAULT_JOURNAL_AGE } else { self.journal_max_age }
    }

    pub fn trash_age(&self) -> u64 {
        if self.trash_max_age == 0 { DEFAULT_TRASH_AGE } else { self.trash_max_age }
    }

    /// Kontrola pred ulozenim (stejna jako pri cteni).
    pub fn validate(&self) -> Result<()> {
        if self.rules.len() > MAX_RULES
            || self.rules.iter().any(|r| r.interval < MIN_INTERVAL || r.keep == 0)
        {
            return Err(Error::InvalidArgs);
        }
        Ok(())
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut b = Vec::with_capacity(SET_HEADER + self.rules.len() * 16);
        b.extend_from_slice(&1u32.to_le_bytes());
        b.extend_from_slice(&(self.rules.len() as u32).to_le_bytes());
        put_u64(&mut b, self.journal_max_entries);
        put_u64(&mut b, self.journal_max_age);
        put_u64(&mut b, self.trash_max_age);
        for r in &self.rules {
            put_u64(&mut b, r.interval);
            b.extend_from_slice(&r.keep.to_le_bytes());
            b.extend_from_slice(&[0; 4]);
        }
        b
    }

    pub fn decode(b: &[u8]) -> Result<Self> {
        if b.len() < SET_HEADER || u32_at(b, 0) != 1 {
            return Err(Error::Corrupted);
        }
        let n = u32_at(b, 4) as usize;
        if n > MAX_RULES || b.len() != SET_HEADER + n * 16 {
            return Err(Error::Corrupted);
        }
        let s = Self {
            journal_max_entries: u64_at(b, 8),
            journal_max_age: u64_at(b, 16),
            trash_max_age: u64_at(b, 24),
            rules: (0..n)
                .map(|i| AutoRule { interval: u64_at(b, SET_HEADER + i * 16), keep: u32_at(b, SET_HEADER + i * 16 + 8) })
                .collect(),
        };
        s.validate().map_err(|_| Error::Corrupted)?;
        Ok(s)
    }
}

// --- kos ---

/// Zaznam kose: odkud objekt prisel, kdy a kdo ho smazal.
///
/// ```text
/// 0  oid u64          8  generace u64     16 puvodni rodic u64
/// 24 sekundy i64      32 nanosekundy u32  36 druh u8, rezerva 3 B
/// 40 kdo u64          48 velikost (u adresare souhrn podstromu) u64
/// 56 puvodni jmeno (UTF-8)
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TrashRecord {
    pub oid: u64,
    pub generation: u64,
    pub parent: u64,
    pub time: Timestamp,
    pub kind: u8,
    pub actor: u64,
    pub size: u64,
    pub name: String,
}

const TR_HEADER: usize = 56;

impl TrashRecord {
    pub fn encode(&self) -> Vec<u8> {
        let mut b = Vec::with_capacity(TR_HEADER + self.name.len());
        put_u64(&mut b, self.oid);
        put_u64(&mut b, self.generation);
        put_u64(&mut b, self.parent);
        put_time(&mut b, self.time);
        b.push(self.kind);
        b.extend_from_slice(&[0; 3]);
        put_u64(&mut b, self.actor);
        put_u64(&mut b, self.size);
        b.extend_from_slice(self.name.as_bytes());
        b
    }

    pub fn decode(b: &[u8]) -> Result<Self> {
        if b.len() < TR_HEADER {
            return Err(Error::Corrupted);
        }
        Ok(Self {
            oid: u64_at(b, 0),
            generation: u64_at(b, 8),
            parent: u64_at(b, 16),
            time: get_time(b, 24)?,
            kind: b[36],
            actor: u64_at(b, 40),
            size: u64_at(b, 48),
            name: utf8(&b[TR_HEADER..])?,
        })
    }
}

// --- rozsirene atributy ---

/// Nejdelsi jmeno atributu (bajty) a nejvetsi hodnota.
pub const XATTR_NAME_MAX: usize = 255;
pub const XATTR_VALUE_MAX: usize = 4096;
/// Atribut expirace: sekundy i64 + nanosekundy u32 (12 B).
pub const XATTR_EXPIRES: &str = "sys.expires";

/// Platne jmeno atributu: jmenny prostor `user.` nebo `sys.`, neprazdny
/// zbytek, bez NUL, do `XATTR_NAME_MAX` bajtu.
pub fn valid_xattr_name(name: &str) -> bool {
    let rest = name.strip_prefix("user.").or_else(|| name.strip_prefix("sys."));
    rest.is_some_and(|r| !r.is_empty()) && name.len() <= XATTR_NAME_MAX && !name.contains('\0')
}

/// Seznam atributu se stejnym hashem jmena: (delka jmena u16, delka
/// hodnoty u16, jmeno, hodnota) * n.
pub fn encode_xattrs(list: &[(String, Vec<u8>)]) -> Vec<u8> {
    let mut b = Vec::new();
    for (n, v) in list {
        b.extend_from_slice(&(n.len() as u16).to_le_bytes());
        b.extend_from_slice(&(v.len() as u16).to_le_bytes());
        b.extend_from_slice(n.as_bytes());
        b.extend_from_slice(v);
    }
    b
}

pub fn decode_xattrs(mut b: &[u8]) -> Result<Vec<(String, Vec<u8>)>> {
    let mut out = Vec::new();
    while !b.is_empty() {
        if b.len() < 4 {
            return Err(Error::Corrupted);
        }
        let nl = u16::from_le_bytes([b[0], b[1]]) as usize;
        let vl = u16::from_le_bytes([b[2], b[3]]) as usize;
        if b.len() < 4 + nl + vl || vl > XATTR_VALUE_MAX {
            return Err(Error::Corrupted);
        }
        let name = utf8(&b[4..4 + nl])?;
        if !valid_xattr_name(&name) {
            return Err(Error::Corrupted);
        }
        out.push((name, b[4 + nl..4 + nl + vl].to_vec()));
        b = &b[4 + nl + vl..];
    }
    Ok(out)
}

pub fn encode_expires(t: Timestamp) -> Vec<u8> {
    let mut b = Vec::with_capacity(12);
    put_time(&mut b, t);
    b
}

pub fn decode_expires(b: &[u8]) -> Result<Timestamp> {
    if b.len() != 12 {
        return Err(Error::Corrupted);
    }
    get_time(b, 0)
}

/// Klic indexu expirace: sekundy s prevracenym znamenkovym bitem
/// (razeni u64 = razeni casu).
pub fn expiry_key(t: Timestamp) -> u64 {
    (t.secs as u64) ^ (1 << 63)
}

pub fn expiry_time(k: u64) -> i64 {
    (k ^ (1 << 63)) as i64
}

/// Seznam (oid, generace) v polozce indexu expirace.
pub fn encode_pairs(list: &[(u64, u64)]) -> Vec<u8> {
    let mut b = Vec::with_capacity(list.len() * 16);
    for &(a, c) in list {
        put_u64(&mut b, a);
        put_u64(&mut b, c);
    }
    b
}

pub fn decode_pairs(b: &[u8]) -> Result<Vec<(u64, u64)>> {
    if !b.len().is_multiple_of(16) {
        return Err(Error::Corrupted);
    }
    Ok(b.as_chunks::<16>().0.iter().map(|c| (u64_at(c, 0), u64_at(c, 8))).collect())
}

// --- pripojeni datasetu ---

/// Zpetny odkaz datasetu na misto, kde je pripojeny.
///
/// ```text
/// 0 dataset rodice u64   8 slozka (oid v rodici) u64   16 jmeno (UTF-8)
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DsLink {
    pub parent_ds: u64,
    pub dir: u64,
    pub name: String,
}

impl DsLink {
    pub fn encode(&self) -> Vec<u8> {
        let mut b = Vec::with_capacity(16 + self.name.len());
        put_u64(&mut b, self.parent_ds);
        put_u64(&mut b, self.dir);
        b.extend_from_slice(self.name.as_bytes());
        b
    }

    pub fn decode(b: &[u8]) -> Result<Self> {
        if b.len() < 16 {
            return Err(Error::Corrupted);
        }
        Ok(Self { parent_ds: u64_at(b, 0), dir: u64_at(b, 8), name: utf8(&b[16..])? })
    }
}

// --- logicky hash a vetve ---

/// Pamet logickeho hashe obsahu souboru: plati, dokud se nezmeni verze
/// obsahu (`Inode::version`) ani generace objektu.
///
/// ```text
/// 0 verze obsahu u64   8 generace u64   16 BLAKE3 obsahu (32 B)
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LogicalHash {
    pub version: u64,
    pub generation: u64,
    pub hash: [u8; 32],
}

impl LogicalHash {
    pub fn encode(&self) -> Vec<u8> {
        let mut b = Vec::with_capacity(48);
        put_u64(&mut b, self.version);
        put_u64(&mut b, self.generation);
        b.extend_from_slice(&self.hash);
        b
    }

    pub fn decode(b: &[u8]) -> Result<Self> {
        if b.len() != 48 {
            return Err(Error::Corrupted);
        }
        Ok(Self { version: u64_at(b, 0), generation: u64_at(b, 8), hash: b[16..48].try_into().unwrap() })
    }
}

/// Vetve se stejnym hashem jmena: (dataset u64, delka jmena u16, jmeno) * n.
pub fn encode_branches(list: &[(u64, String)]) -> Vec<u8> {
    let mut b = Vec::new();
    for (ds, n) in list {
        put_u64(&mut b, *ds);
        b.extend_from_slice(&(n.len() as u16).to_le_bytes());
        b.extend_from_slice(n.as_bytes());
    }
    b
}

pub fn decode_branches(mut b: &[u8]) -> Result<Vec<(u64, String)>> {
    let mut out = Vec::new();
    while !b.is_empty() {
        if b.len() < 10 {
            return Err(Error::Corrupted);
        }
        let ds = u64_at(b, 0);
        let l = u16::from_le_bytes([b[8], b[9]]) as usize;
        let name = utf8(b.get(10..10 + l).ok_or(Error::Corrupted)?)?;
        out.push((ds, name));
        b = &b[10 + l..];
    }
    Ok(out)
}
