//! Intent log: rychle trvale ulozeni zmen bez celeho commitu (`fsync`).
//!
//! Od posledniho commitu se v pameti vede seznam operaci s adresari
//! (vytvoreni, smazani, prejmenovani, zmena velikosti) a mnozina zmenenych
//! bloku souboru. `fsync` zmenena data zapise do novych bloku, operace a
//! mapovani dat zakoduje do davky a zapise ji do retezu bloku logu; jeden
//! flush pak vse ucini trvalym.
//!
//! ```text
//! blok logu: 0 magic "ZL", 2 priznak (1 = posledni blok davky),
//!            8 poradi u64, 16 dalsi blok u64, 24 delka dat u32,
//!            32 soucet XXH3-128 se seminkem (epocha + poradi), 64 data
//! ```
//!
//! Dalsi blok retezu se alokuje predem a jeho cislo je v predchozim bloku.
//! Konec logu = prvni blok se spatnym souctem nebo poradim (stary obsah z
//! jine epochy nebo poradi nikdy neprojde). Pri pripojeni se uplne davky
//! prehraji nad stavem posledniho commitu a hned se ulozi commitem; davka,
//! jejiz data nejsou cela na disku (vypadek pred flush), se zahodi.

use alloc::collections::{BTreeMap, BTreeSet};
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use abi::{Error, Result};

use super::{Fs, Kind};
use crate::btree;
use crate::device::BlockDevice;
use crate::hash::{Csum, xxh3_seeded};
use crate::layout::{
    BLOCK_SIZE, Extent, INLINE_MAX, Key, TimeFormat, Timestamp, csum_at, from_legacy_nanos, item, legacy_nanos, u32_at, u64_at,
};

const MAGIC: u16 = u16::from_le_bytes(*b"ZL");
const HEADER: usize = 64;
const PAYLOAD: usize = BLOCK_SIZE - HEADER;
/// Kdyz log naroste nad tolik bloku, `fsync` radeji udela commit.
const MAX_LOG_BLOCKS: usize = 2048;

/// Operace s adresari a velikosti (prehravaji se v poradi).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Op {
    Create {
        parent: u64,
        oid: u64,
        /// Generace vzniku (po prehrani musi objekt mit tu puvodni).
        generation: u64,
        kind: u8,
        name: String,
        time: Timestamp,
        /// Vlastnik noveho objektu (0 = system).
        owner: u64,
    },
    Remove {
        parent: u64,
        name: String,
        time: Timestamp,
    },
    Rename {
        src: u64,
        name: String,
        dst: u64,
        new_name: String,
        time: Timestamp,
    },
    Truncate {
        oid: u64,
        size: u64,
        time: Timestamp,
    },
    Symlink {
        parent: u64,
        oid: u64,
        generation: u64,
        name: String,
        target: String,
        time: Timestamp,
        owner: u64,
    },
    /// Nastaveni casu objektu (`Fs::set_times`); `time` = nova hodnota ctime.
    SetTimes {
        oid: u64,
        times: super::SetTimes,
        time: Timestamp,
    },
    /// Novy vlastni popis opravneni objektu (`None` = jen dedit).
    SetAcl {
        oid: u64,
        acl: Option<crate::perm::Acl>,
        time: Timestamp,
    },
    SetOwner {
        oid: u64,
        owner: u64,
        time: Timestamp,
    },
    /// Presun objektu `parent`/`name` do kose pod cislem `id`.
    Trash {
        parent: u64,
        name: String,
        id: u64,
        time: Timestamp,
    },
    /// Obnova zaznamu kose `id` do `dst`/`name`.
    Restore {
        id: u64,
        dst: u64,
        name: String,
        time: Timestamp,
    },
    /// Trvale smazani zaznamu kose `id` (i s podstromem).
    Purge {
        id: u64,
        time: Timestamp,
    },
    /// Nastaveni (`Some`) nebo smazani (`None`) rozsireneho atributu.
    SetXattr {
        oid: u64,
        name: String,
        value: Option<Vec<u8>>,
        time: Timestamp,
    },
}

/// Zaznam davky.
enum Record {
    /// Nasledujici zaznamy patri datasetu s timto cislem (az do dalsiho).
    Dataset(u64),
    /// Nasledujici zaznamy provedl tento uzivatel (pro zurnal zmen: po
    /// prehrani musi zaznamy vyjit stejne jako pred padem).
    Actor(u64),
    Op(Op),
    Inline {
        oid: u64,
        size: u64,
        mtime: Timestamp,
        data: Vec<u8>,
    },
    Extents {
        oid: u64,
        size: u64,
        mtime: Timestamp,
        list: Vec<(u64, Extent)>,
    },
}

/// Zmeny od posledniho zapisu do logu.
#[derive(Default)]
pub struct Pending {
    /// (kdo, operace)
    ops: Vec<(u64, Op)>,
    /// Soubory se zmenenym obsahem/velikosti -> zmenene logicke bloky.
    touched: BTreeMap<u64, BTreeSet<u64>>,
}

impl Pending {
    pub fn op(&mut self, actor: u64, op: Op) {
        self.ops.push((actor, op));
    }

    pub fn touch(&mut self, oid: u64) {
        self.touched.entry(oid).or_default();
    }

    pub fn touch_block(&mut self, oid: u64, lb: u64) {
        self.touched.entry(oid).or_default().insert(lb);
    }

    pub fn drop_blocks_from(&mut self, oid: u64, from: u64) {
        if let Some(set) = self.touched.get_mut(&oid) {
            set.split_off(&from);
        }
    }

    pub fn forget(&mut self, oid: u64) {
        self.touched.remove(&oid);
    }

    pub fn clear(&mut self) {
        self.ops.clear();
        self.touched.clear();
    }

    pub(super) fn is_empty(&self) -> bool {
        self.ops.is_empty() && self.touched.is_empty()
    }
}

// --- kodovani zaznamu ---

fn put_u64(b: &mut Vec<u8>, v: u64) {
    b.extend_from_slice(&v.to_le_bytes());
}

/// Cas v zaznamu: stary tvar u64 ns, novy i64 sekund + u32 ns (12 B).
fn put_time(b: &mut Vec<u8>, t: Timestamp, tf: TimeFormat) {
    match tf {
        TimeFormat::LegacyNanos => put_u64(b, legacy_nanos(t)),
        TimeFormat::Secs => {
            put_u64(b, t.secs as u64);
            b.extend_from_slice(&t.nanos.to_le_bytes());
        }
    }
}

fn put_str(b: &mut Vec<u8>, s: &str) {
    b.extend_from_slice(&(s.len() as u16).to_le_bytes());
    b.extend_from_slice(s.as_bytes());
}

fn encode(records: &[Record], tf: TimeFormat) -> Vec<u8> {
    let mut b = Vec::new();
    for r in records {
        match r {
            Record::Op(Op::Create {
                parent,
                oid,
                generation,
                kind,
                name,
                time,
                owner,
            }) => {
                // 10 = 1 s vlastnikem (jen kdyz neni system: svazky bez
                // uzivatelu zustanou citelne i starsim ovladacem).
                if *owner != 0 {
                    b.push(10);
                    put_u64(&mut b, *owner);
                }
                b.push(1);
                put_u64(&mut b, *parent);
                put_u64(&mut b, *oid);
                put_u64(&mut b, *generation);
                b.push(*kind);
                put_time(&mut b, *time, tf);
                put_str(&mut b, name);
            }
            Record::Op(Op::Remove { parent, name, time }) => {
                b.push(2);
                put_u64(&mut b, *parent);
                put_time(&mut b, *time, tf);
                put_str(&mut b, name);
            }
            Record::Op(Op::Rename {
                src,
                name,
                dst,
                new_name,
                time,
            }) => {
                b.push(3);
                put_u64(&mut b, *src);
                put_u64(&mut b, *dst);
                put_time(&mut b, *time, tf);
                put_str(&mut b, name);
                put_str(&mut b, new_name);
            }
            Record::Op(Op::Truncate { oid, size, time }) => {
                b.push(4);
                put_u64(&mut b, *oid);
                put_u64(&mut b, *size);
                put_time(&mut b, *time, tf);
            }
            Record::Op(Op::Symlink {
                parent,
                oid,
                generation,
                name,
                target,
                time,
                owner,
            }) => {
                if *owner != 0 {
                    b.push(11);
                    put_u64(&mut b, *owner);
                }
                b.push(7);
                put_u64(&mut b, *parent);
                put_u64(&mut b, *oid);
                put_u64(&mut b, *generation);
                put_time(&mut b, *time, tf);
                put_str(&mut b, name);
                put_str(&mut b, target);
            }
            Record::Op(Op::SetAcl { oid, acl, time }) => {
                // 12: oid, cas, ma popis u8, delka u32, popis (bez odkazu).
                b.push(12);
                put_u64(&mut b, *oid);
                put_time(&mut b, *time, tf);
                match acl {
                    Some(a) => {
                        let e = crate::perm::encode_acl(a, 0);
                        b.push(1);
                        b.extend_from_slice(&(e.len() as u32).to_le_bytes());
                        b.extend_from_slice(&e);
                    }
                    None => b.push(0),
                }
            }
            Record::Op(Op::SetOwner { oid, owner, time }) => {
                b.push(13);
                put_u64(&mut b, *oid);
                put_u64(&mut b, *owner);
                put_time(&mut b, *time, tf);
            }
            Record::Op(Op::SetTimes { oid, times, time }) => {
                // 9: oid, ktere casy u8 (bit 0 mtime, 1 btime, 2 atime),
                // cas zmeny, pak jen pritomne casy v tomto poradi.
                b.push(9);
                put_u64(&mut b, *oid);
                let present = [times.mtime, times.btime, times.atime];
                let mask = present.iter().enumerate().fold(0u8, |m, (i, t)| m | ((t.is_some() as u8) << i));
                b.push(mask);
                put_time(&mut b, *time, tf);
                for t in present.into_iter().flatten() {
                    put_time(&mut b, t, tf);
                }
            }
            Record::Inline {
                oid,
                size,
                mtime,
                data,
            } => {
                b.push(5);
                put_u64(&mut b, *oid);
                put_u64(&mut b, *size);
                put_time(&mut b, *mtime, tf);
                b.extend_from_slice(&(data.len() as u32).to_le_bytes());
                b.extend_from_slice(data);
            }
            Record::Dataset(id) => {
                b.push(8);
                put_u64(&mut b, *id);
            }
            Record::Actor(a) => {
                b.push(14);
                put_u64(&mut b, *a);
            }
            Record::Op(Op::Trash { parent, name, id, time }) => {
                b.push(15);
                put_u64(&mut b, *parent);
                put_u64(&mut b, *id);
                put_time(&mut b, *time, tf);
                put_str(&mut b, name);
            }
            Record::Op(Op::Restore { id, dst, name, time }) => {
                b.push(16);
                put_u64(&mut b, *id);
                put_u64(&mut b, *dst);
                put_time(&mut b, *time, tf);
                put_str(&mut b, name);
            }
            Record::Op(Op::Purge { id, time }) => {
                b.push(17);
                put_u64(&mut b, *id);
                put_time(&mut b, *time, tf);
            }
            Record::Op(Op::SetXattr { oid, name, value, time }) => {
                // 18: oid, cas, jmeno, ma hodnotu u8, delka u32, hodnota.
                b.push(18);
                put_u64(&mut b, *oid);
                put_time(&mut b, *time, tf);
                put_str(&mut b, name);
                match value {
                    Some(v) => {
                        b.push(1);
                        b.extend_from_slice(&(v.len() as u32).to_le_bytes());
                        b.extend_from_slice(v);
                    }
                    None => b.push(0),
                }
            }
            Record::Extents {
                oid,
                size,
                mtime,
                list,
            } => {
                b.push(6);
                put_u64(&mut b, *oid);
                put_u64(&mut b, *size);
                put_time(&mut b, *mtime, tf);
                b.extend_from_slice(&(list.len() as u32).to_le_bytes());
                for (lb, e) in list {
                    put_u64(&mut b, *lb);
                    let raw = e.encode();
                    b.extend_from_slice(&(raw.len() as u32).to_le_bytes());
                    b.extend_from_slice(&raw);
                }
            }
        }
    }
    b
}

struct Reader<'a> {
    b: &'a [u8],
    pos: usize,
    tf: TimeFormat,
}

impl Reader<'_> {
    fn take(&mut self, n: usize) -> Result<&[u8]> {
        let s = self.b.get(self.pos..self.pos + n).ok_or(Error::Corrupted)?;
        self.pos += n;
        Ok(s)
    }
    fn u8(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }
    fn u32(&mut self) -> Result<u32> {
        Ok(u32_at(self.take(4)?, 0))
    }
    fn u64(&mut self) -> Result<u64> {
        Ok(u64_at(self.take(8)?, 0))
    }
    fn time(&mut self) -> Result<Timestamp> {
        let raw = self.u64()?;
        match self.tf {
            TimeFormat::LegacyNanos => Ok(from_legacy_nanos(raw)),
            TimeFormat::Secs => {
                let nanos = self.u32()?;
                if nanos >= 1_000_000_000 {
                    return Err(Error::Corrupted);
                }
                Ok(Timestamp { secs: raw as i64, nanos })
            }
        }
    }
    fn str(&mut self) -> Result<String> {
        let n = u16::from_le_bytes(self.take(2)?.try_into().unwrap()) as usize;
        Ok(core::str::from_utf8(self.take(n)?)
            .map_err(|_| Error::Corrupted)?
            .into())
    }
}

fn decode(b: &[u8], tf: TimeFormat) -> Result<Vec<Record>> {
    let mut r = Reader { b, pos: 0, tf };
    let mut out = Vec::new();
    while r.pos < b.len() {
        let mut code = r.u8()?;
        let mut owner = 0;
        if code == 10 || code == 11 {
            owner = r.u64()?;
            let inner = r.u8()?;
            if (code == 10 && inner != 1) || (code == 11 && inner != 7) || owner == 0 {
                return Err(Error::Corrupted);
            }
            code = inner;
        }
        let rec = match code {
            1 => {
                let (parent, oid, generation) = (r.u64()?, r.u64()?, r.u64()?);
                let (kind, time) = (r.u8()?, r.time()?);
                Record::Op(Op::Create {
                    parent,
                    oid,
                    generation,
                    kind,
                    time,
                    name: r.str()?,
                    owner,
                })
            }
            2 => {
                let (parent, time) = (r.u64()?, r.time()?);
                Record::Op(Op::Remove {
                    parent,
                    time,
                    name: r.str()?,
                })
            }
            3 => {
                let (src, dst, time) = (r.u64()?, r.u64()?, r.time()?);
                let (name, new_name) = (r.str()?, r.str()?);
                Record::Op(Op::Rename {
                    src,
                    name,
                    dst,
                    new_name,
                    time,
                })
            }
            7 => {
                let (parent, oid, generation, time) = (r.u64()?, r.u64()?, r.u64()?, r.time()?);
                let (name, target) = (r.str()?, r.str()?);
                Record::Op(Op::Symlink {
                    parent,
                    oid,
                    generation,
                    name,
                    target,
                    time,
                    owner,
                })
            }
            4 => Record::Op(Op::Truncate {
                oid: r.u64()?,
                size: r.u64()?,
                time: r.time()?,
            }),
            5 => {
                let (oid, size, mtime) = (r.u64()?, r.u64()?, r.time()?);
                let n = r.u32()? as usize;
                Record::Inline {
                    oid,
                    size,
                    mtime,
                    data: r.take(n)?.to_vec(),
                }
            }
            6 => {
                let (oid, size, mtime) = (r.u64()?, r.u64()?, r.time()?);
                let count = r.u32()? as usize;
                let mut list = Vec::with_capacity(count.min(1024));
                for _ in 0..count {
                    let lb = r.u64()?;
                    let n = r.u32()? as usize;
                    let e = Extent::decode(r.take(n)?).map_err(|_| Error::Corrupted)?;
                    // Sdilene a nezapsane extenty log nenese (fsync s nimi
                    // dela commit).
                    if e.is_shared() || e.is_unwritten() {
                        return Err(Error::Corrupted);
                    }
                    list.push((lb, e));
                }
                Record::Extents {
                    oid,
                    size,
                    mtime,
                    list,
                }
            }
            8 => Record::Dataset(r.u64()?),
            14 => Record::Actor(r.u64()?),
            15 => {
                let (parent, id, time) = (r.u64()?, r.u64()?, r.time()?);
                Record::Op(Op::Trash { parent, id, time, name: r.str()? })
            }
            16 => {
                let (id, dst, time) = (r.u64()?, r.u64()?, r.time()?);
                Record::Op(Op::Restore { id, dst, time, name: r.str()? })
            }
            17 => Record::Op(Op::Purge { id: r.u64()?, time: r.time()? }),
            18 => {
                let (oid, time, name) = (r.u64()?, r.time()?, r.str()?);
                let value = match r.u8()? {
                    0 => None,
                    1 => {
                        let n = r.u32()? as usize;
                        if n > crate::records::XATTR_VALUE_MAX {
                            return Err(Error::Corrupted);
                        }
                        Some(r.take(n)?.to_vec())
                    }
                    _ => return Err(Error::Corrupted),
                };
                Record::Op(Op::SetXattr { oid, name, value, time })
            }
            12 => {
                let (oid, time) = (r.u64()?, r.time()?);
                let acl = match r.u8()? {
                    0 => None,
                    1 => {
                        let n = r.u32()? as usize;
                        let (a, _) = crate::perm::decode_acl(r.take(n)?)?;
                        Some(a)
                    }
                    _ => return Err(Error::Corrupted),
                };
                Record::Op(Op::SetAcl { oid, acl, time })
            }
            13 => {
                let (oid, owner, time) = (r.u64()?, r.u64()?, r.time()?);
                Record::Op(Op::SetOwner { oid, owner, time })
            }
            9 => {
                let (oid, mask) = (r.u64()?, r.u8()?);
                if mask & !0b111 != 0 {
                    return Err(Error::Corrupted);
                }
                let time = r.time()?;
                let mut t = [None; 3];
                for (i, slot) in t.iter_mut().enumerate() {
                    if mask & (1 << i) != 0 {
                        *slot = Some(r.time()?);
                    }
                }
                Record::Op(Op::SetTimes {
                    oid,
                    times: super::SetTimes { mtime: t[0], btime: t[1], atime: t[2] },
                    time,
                })
            }
            _ => return Err(Error::Corrupted),
        };
        out.push(rec);
    }
    Ok(out)
}

fn block_sum(b: &[u8], seed: u64, seq: u64) -> Csum {
    let mut tmp = b.to_vec();
    tmp[32..48].fill(0);
    xxh3_seeded(&tmp, seed.wrapping_add(seq))
}

impl<D: BlockDevice> Fs<D> {
    /// Ucini vsechny dosavadni zmeny trvalymi (prezijou vypadek proudu).
    /// Rychle: zapise jen zmenena data a davku logu, jeden flush.
    ///
    /// Atomicita: seznam cekajicich zmen se smaze az po uspesnem flush. Chyba
    /// pred zapisem prvniho bloku logu nic nerozbije (dalsi `fsync` to
    /// zkusi znovu). Chyba pri zapisu logu nebo flush = nejisty stav (cast
    /// davky muze byt na disku): svazek prejde do stavu chyby (`IO_ERROR` pro
    /// vsechny dalsi zmeny), aby zadny pozdejsi `fsync` nepotvrdil neco, co
    /// trvale neni. Obnova = znovu pripojit (prehraji se jen cele davky).
    ///
    /// Davka obsahuje zmeny vsech datasetu; zaznamy kazdeho zacinaji
    /// `Record::Dataset(cislo)`.
    pub fn fsync(&mut self) -> Result<()> {
        self.s.check_writable()?;
        let back = self.cur_ds;
        let pending: Vec<u64> = self.loaded_datasets().into_iter().filter(|&id| !self.pending_empty(id)).collect();
        if pending.is_empty() {
            return Ok(());
        }
        if self.s.zil_blocks.len() > MAX_LOG_BLOCKS {
            return self.commit();
        }
        let mut records = Vec::new();
        for &id in &pending {
            self.activate(id)?;
            records.push(Record::Dataset(id));
            // Zaznamy logu potrebuji, kdo menil obsah (`content_actor`).
            records.extend(self.log_records()?);
        }
        self.activate(back)?;
        // Sdileny extent z teto transakce (deduplikace pri zapisu) log
        // neumi prehrat (pocty odkazu): radeji commit.
        let shared = records
            .iter()
            .any(|r| matches!(r, Record::Extents { list, .. } if list.iter().any(|(_, e)| e.is_shared())));
        if shared {
            return self.commit();
        }
        // Zapis dat narazil na neplatne uvolneni (poskozena hodnota): nic
        // nepotvrdit - stejne jako commit.
        if self.s.space.corrupt {
            self.s.poisoned = true;
            return Err(Error::Corrupted);
        }
        let payload = encode(&records, self.s.time_format());
        // Bloky retezu predem: nedostatek mista se ohlasi drive, nez se na
        // disk cokoli zapise (bez prechodu do stavu chyby).
        let count = payload.len().div_ceil(PAYLOAD);
        let mut nexts = Vec::with_capacity(count);
        for _ in 0..count {
            match self.s.alloc_meta() {
                Ok(b) => nexts.push(b),
                Err(e) => {
                    let txg = self.s.txg;
                    for &b in &nexts {
                        self.s.space.free(b, 1, txg, true);
                    }
                    return Err(e);
                }
            }
        }
        if let Err(e) = self.write_batch(&payload, &nexts) {
            self.s.poisoned = true;
            return Err(e);
        }
        self.clear_all_pending();
        // Sloucene zaznamy CONTENT dostanou cisla az po uspesnem zapisu
        // davky (po prehrani na konci davky znovu stejna - `replay_log`).
        // Pri chybe pred zapisem zustane, kdo menil obsah, pro dalsi fsync.
        for &id in &pending {
            self.activate(id)?;
            self.jflush()?;
        }
        self.activate(back)?;
        self.jpublish_all();
        Ok(())
    }

    /// Zaznamy davky pro `fsync` (cekajici seznam zustava nedotceny).
    /// Zmenena data souboru zapise do novych bloku.
    fn log_records(&mut self) -> Result<Vec<Record>> {
        let mut records: Vec<Record> = Vec::new();
        let mut last_actor = None;
        for (actor, op) in &self.zil.ops {
            if last_actor != Some(*actor) {
                records.push(Record::Actor(*actor));
                last_actor = Some(*actor);
            }
            records.push(Record::Op(op.clone()));
        }
        let touched = self.zil.touched.clone();
        for (oid, blocks) in touched {
            // Kdo obsah zmenil (zaznam CONTENT v zurnalu po prehrani).
            let actor = self.jr.content_actor(oid);
            if last_actor != Some(actor) {
                records.push(Record::Actor(actor));
                last_actor = Some(actor);
            }
            let root = self.s.root;
            let Some(raw) = btree::get(&mut self.s, &root, &Key::new(oid, item::INODE, 0))? else {
                continue;
            };
            let inode = crate::layout::Inode::decode(&raw, self.s.time_format())?;
            if inode.size as usize <= INLINE_MAX {
                let data = self.inline_data(&root, oid, inode.size)?;
                records.push(Record::Inline {
                    oid,
                    size: inode.size,
                    mtime: inode.mtime,
                    data,
                });
                continue;
            }
            self.writeback(oid, Some(&blocks))?;
            let txg = self.s.txg;
            let mut list = Vec::new();
            if let (Some(&lo), Some(&hi)) = (blocks.first(), blocks.last()) {
                let root = self.s.root;
                for (ks, e) in self.extents_in(&root, oid, lo, hi + 1)? {
                    // Nove bloky teto transakce, nebo zapis na misto do
                    // predalokovanych (generace vzniku zustala stara).
                    let written_here = e.birth == txg
                        || (!e.is_unwritten()
                            && self.inplace.range(..e.phys + e.plen()).next_back().is_some_and(|(&s, &l)| s + l > e.phys));
                    if written_here && blocks.range(ks..ks + e.len()).next().is_some() {
                        self.s.zil_logged.insert(e.phys, e.plen());
                        if e.copy != 0 {
                            self.s.zil_logged.insert(e.copy, e.plen());
                        }
                        list.push((ks, e));
                    }
                }
            }
            records.push(Record::Extents {
                oid,
                size: inode.size,
                mtime: inode.mtime,
                list,
            });
        }
        Ok(records)
    }

    /// Zapise davku do retezu bloku logu a udela flush.
    /// `nexts[i]` = predem alokovany dalsi blok retezu za blokem `i`.
    fn write_batch(&mut self, payload: &[u8], nexts: &[u64]) -> Result<()> {
        let chunks: Vec<&[u8]> = payload.chunks(PAYLOAD).collect();
        debug_assert_eq!(chunks.len(), nexts.len());
        let seed = self.s.sb.zil_seed;
        for (i, chunk) in chunks.iter().enumerate() {
            let next = nexts[i];
            let mut b = vec![0u8; BLOCK_SIZE];
            b[0..2].copy_from_slice(&MAGIC.to_le_bytes());
            b[2] = (i + 1 == chunks.len()) as u8;
            b[8..16].copy_from_slice(&self.s.zil_seq.to_le_bytes());
            b[16..24].copy_from_slice(&next.to_le_bytes());
            b[24..28].copy_from_slice(&(chunk.len() as u32).to_le_bytes());
            b[HEADER..HEADER + chunk.len()].copy_from_slice(chunk);
            let c = block_sum(&b, seed, self.s.zil_seq);
            b[32..48].copy_from_slice(&c.0);
            self.s.dev.write_block(self.s.zil_next, &b)?;
            self.s.zil_blocks.push(next);
            self.s.zil_next = next;
            self.s.zil_seq += 1;
        }
        self.s.dev.flush()
    }

    /// Precte log, prehraje uplne davky a ulozi vysledek commitem.
    pub(super) fn replay_log(&mut self) -> Result<()> {
        if self.s.fallback > 0 {
            // Log patri novejsimu (poskozenemu) stavu: zahodit, nova epocha.
            if self.s.read_only {
                return Ok(());
            }
            self.s.mark_changed();
            return self.commit();
        }
        let seed = self.s.sb.zil_seed;
        let mut block = self.s.sb.zil_block;
        let mut seq = 0u64;
        let mut payload = Vec::new();
        let mut batches: Vec<Vec<u8>> = Vec::new();
        let mut buf = vec![0u8; BLOCK_SIZE];
        let mut found = false;
        // Chyba I/O neni konec retezu: za necitelnym blokem muze byt
        // potvrzeny fsync. Pak se prehraje jen to, co jde precist, svazek je
        // jen pro cteni a log zustane na disku pro dalsi pokus.
        let mut unreadable = false;
        while block != 0 && block < self.s.sb.block_count {
            if self.s.dev.read_block(block, &mut buf).is_err() {
                unreadable = true;
                break;
            }
            let valid = u16::from_le_bytes([buf[0], buf[1]]) == MAGIC
                && u64_at(&buf, 8) == seq
                && block_sum(&buf, seed, seq) == csum_at(&buf, 32)
                && (u32_at(&buf, 24) as usize) <= PAYLOAD;
            if !valid {
                break;
            }
            if seq > 0 {
                // Blok alokovany po commitu: podle space logu je volny.
                let txg = self.s.txg;
                if self.s.space.claim(block, 1, txg).is_err() {
                    break;
                }
                self.s.zil_blocks.push(block);
            }
            found = true;
            payload.extend_from_slice(&buf[HEADER..HEADER + u32_at(&buf, 24) as usize]);
            if buf[2] == 1 {
                batches.push(core::mem::take(&mut payload));
            }
            block = u64_at(&buf, 16);
            seq += 1;
        }
        for batch in batches {
            let Ok(records) = decode(&batch, self.s.time_format()) else { break };
            match self.batch_durable(&records) {
                Ok(true) => {}
                Ok(false) => break,
                Err(_) => {
                    unreadable = true;
                    break;
                }
            }
            for r in records {
                let op = !matches!(r, Record::Dataset(_) | Record::Actor(_));
                self.apply(r)?;
                self.replayed += op as u64;
            }
            // Zaznamy CONTENT dostaly cisla na konci davky (pri fsync) -
            // po prehrani take, aby cisla vysla stejne.
            self.actor_override = None;
            self.for_each_dataset(|fs| fs.jflush())?;
        }
        if unreadable {
            if self.discard_unreadable_log && !self.s.read_only {
                // Vedoma volba: zbytek logu se zahodi novou epochou nize.
                self.log_discarded = true;
                found = true;
            } else {
                self.s.log_unreadable = true;
                self.s.read_only = true;
            }
        }
        self.activate(crate::layout::ROOT_DATASET)?;
        // Jen pro cteni: prehrane zmeny zustanou v pameti (vidi je cteni),
        // na disk se nezapisuje nic.
        if found && !self.s.read_only {
            self.s.mark_changed();
            self.commit()?;
        }
        Ok(())
    }

    /// Jsou vsechna data davky na disku (souhlasi soucty)?
    fn object_exists(&mut self, oid: u64) -> Result<bool> {
        let root = self.s.root;
        Ok(btree::get(&mut self.s, &root, &Key::new(oid, item::INODE, 0))?.is_some())
    }

    fn batch_durable(&mut self, records: &[Record]) -> Result<bool> {
        for r in records {
            if let Record::Extents { list, .. } = r {
                for (_, e) in list {
                    match self.extent_data(e, 0, e.len()) {
                        Ok(_) => {}
                        Err(Error::Corrupted) => return Ok(false),
                        Err(e) => return Err(e),
                    }
                }
            }
        }
        Ok(true)
    }

    fn apply(&mut self, r: Record) -> Result<()> {
        let bad = |_| Error::Corrupted;
        match r {
            Record::Dataset(id) => self.activate(id).map_err(bad),
            Record::Actor(a) => {
                self.actor_override = Some(a);
                Ok(())
            }
            Record::Op(Op::Trash { parent, name, id, time }) => {
                self.now = time;
                self.trash_at(parent, &name, id).map_err(bad)
            }
            Record::Op(Op::Restore { id, dst, name, time }) => {
                self.now = time;
                self.restore_at(id, dst, &name).map_err(bad)
            }
            Record::Op(Op::Purge { id, time }) => {
                self.now = time;
                self.purge_at(id).map_err(bad)
            }
            Record::Op(Op::SetXattr { oid, name, value, time }) => {
                self.now = time;
                if !self.object_exists(oid).map_err(bad)? {
                    return Ok(());
                }
                self.set_xattr_oid(oid, &name, value.as_deref()).map_err(bad)
            }
            Record::Op(Op::Create {
                parent,
                oid,
                generation,
                kind,
                name,
                time,
                owner,
            }) => {
                self.now = time;
                let kind = match kind {
                    crate::layout::KIND_DIR => Kind::Dir,
                    crate::layout::KIND_FILE => Kind::File,
                    _ => return Err(Error::Corrupted),
                };
                self.owner_override = Some(owner);
                let r = self.create_at(parent, &name, kind, oid, generation);
                self.owner_override = None;
                r.map_err(bad)
            }
            Record::Op(Op::Symlink {
                parent,
                oid,
                generation,
                name,
                target,
                time,
                owner,
            }) => {
                self.now = time;
                self.owner_override = Some(owner);
                let r = self.symlink_at(parent, &name, &target, oid, generation);
                self.owner_override = None;
                r.map_err(bad)
            }
            Record::Op(Op::SetAcl { oid, acl, time }) => {
                self.now = time;
                if !self.object_exists(oid).map_err(bad)? {
                    return Ok(());
                }
                let acl = match acl {
                    Some(a) => Some(a.normalized().map_err(bad)?),
                    None => None,
                };
                self.set_acl_oid(oid, acl.as_ref()).map_err(bad)
            }
            Record::Op(Op::SetOwner { oid, owner, time }) => {
                self.now = time;
                if !self.object_exists(oid).map_err(bad)? {
                    return Ok(());
                }
                self.set_owner_oid(oid, owner).map_err(bad)
            }
            Record::Op(Op::Remove { parent, name, time }) => {
                self.now = time;
                self.remove_at(parent, &name).map_err(bad)
            }
            Record::Op(Op::Rename {
                src,
                name,
                dst,
                new_name,
                time,
            }) => {
                self.now = time;
                self.rename_at(src, &name, dst, &new_name).map_err(bad)
            }
            Record::Op(Op::Truncate { oid, size, time }) => {
                self.now = time;
                self.truncate_oid(oid, size).map_err(bad)
            }
            Record::Op(Op::SetTimes { oid, times, time }) => {
                self.now = time;
                // Objekt mohl mezitim zaniknout (sirotek uklizeny pri pripojeni).
                if !self.object_exists(oid).map_err(bad)? {
                    return Ok(());
                }
                self.set_times_oid(oid, times).map_err(bad)
            }
            // Data objektu, ktery v okamziku prehrani uz nema zaznam (zapis do
            // sirotka po smazani jmena; sirotek po restartu zanika): nic.
            Record::Inline { oid, .. } | Record::Extents { oid, .. } if !self.object_exists(oid)? => Ok(()),
            Record::Inline {
                oid,
                size,
                mtime,
                data,
            } => {
                self.now = mtime;
                let inode = self.live_inode(oid).map_err(bad)?;
                let write = (!data.is_empty()).then_some((0u64, data.as_slice()));
                self.set_content(oid, inode, size, write).map_err(bad)
            }
            Record::Extents {
                oid,
                size,
                mtime,
                list,
            } => {
                self.now = mtime;
                let inode = self.live_inode(oid).map_err(bad)?;
                if inode.size != size {
                    self.set_content(oid, inode, size, None).map_err(bad)?;
                }
                let txg = self.s.txg;
                for (lb, mut e) in list {
                    let n = e.len();
                    // Zapis na misto do predalokovaneho extentu: bloky uz
                    // patri souboru, jen se oznaci jako zapsane.
                    let root = self.s.root;
                    let under = self.extents_in(&root, oid, lb, lb + 1)?.into_iter().next();
                    if let Some((ks, x)) = under
                        && x.is_unwritten()
                        && !e.is_compressed()
                        && x.phys + (lb - ks) == e.phys
                        && ks + x.len() >= lb + n
                    {
                        self.relink_in_place(oid, ks, &x, lb - ks, e.csums.clone())?;
                    } else {
                        self.punch_range(oid, lb, lb + n)?;
                        self.s.space.claim(e.phys, e.plen(), txg)?;
                        if e.copy != 0 {
                            self.s.space.claim(e.copy, e.plen(), txg)?;
                        }
                        self.s.mark_changed();
                        e.birth = txg;
                        self.put(Key::new(oid, item::EXTENT, lb), e.encode())?;
                    }
                    // Mapovani z logu ma prednost pred prevedenym obsahem.
                    if let Some(fd) = self.data.get_mut(&oid) {
                        let before = fd.blocks.len();
                        fd.blocks.retain(|&b, _| b < lb || b >= lb + n);
                        self.dirty_blocks -= (before - fd.blocks.len()) as u64;
                    }
                }
                let mut inode = self.live_inode(oid)?;
                inode.mtime = mtime;
                self.put_inode(oid, &inode)?;
                let actor = self.actor();
                self.jr.note_content(oid, actor);
                Ok(())
            }
        }
    }
}
