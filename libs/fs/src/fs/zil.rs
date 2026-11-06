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
use crate::layout::{BLOCK_SIZE, Extent, INLINE_MAX, Key, csum_at, item, u32_at, u64_at};

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
        kind: u8,
        name: String,
        time: u64,
    },
    Remove {
        parent: u64,
        name: String,
        time: u64,
    },
    Rename {
        src: u64,
        name: String,
        dst: u64,
        new_name: String,
        time: u64,
    },
    Truncate {
        oid: u64,
        size: u64,
        time: u64,
    },
    Symlink {
        parent: u64,
        oid: u64,
        name: String,
        target: String,
        time: u64,
    },
}

/// Zaznam davky.
enum Record {
    Op(Op),
    Inline {
        oid: u64,
        size: u64,
        mtime: u64,
        data: Vec<u8>,
    },
    Extents {
        oid: u64,
        size: u64,
        mtime: u64,
        list: Vec<(u64, Extent)>,
    },
}

/// Zmeny od posledniho zapisu do logu.
#[derive(Default)]
pub struct Pending {
    ops: Vec<Op>,
    /// Soubory se zmenenym obsahem/velikosti -> zmenene logicke bloky.
    touched: BTreeMap<u64, BTreeSet<u64>>,
}

impl Pending {
    pub fn op(&mut self, op: Op) {
        self.ops.push(op);
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

    fn is_empty(&self) -> bool {
        self.ops.is_empty() && self.touched.is_empty()
    }
}

// --- kodovani zaznamu ---

fn put_u64(b: &mut Vec<u8>, v: u64) {
    b.extend_from_slice(&v.to_le_bytes());
}

fn put_str(b: &mut Vec<u8>, s: &str) {
    b.extend_from_slice(&(s.len() as u16).to_le_bytes());
    b.extend_from_slice(s.as_bytes());
}

fn encode(records: &[Record]) -> Vec<u8> {
    let mut b = Vec::new();
    for r in records {
        match r {
            Record::Op(Op::Create {
                parent,
                oid,
                kind,
                name,
                time,
            }) => {
                b.push(1);
                put_u64(&mut b, *parent);
                put_u64(&mut b, *oid);
                b.push(*kind);
                put_u64(&mut b, *time);
                put_str(&mut b, name);
            }
            Record::Op(Op::Remove { parent, name, time }) => {
                b.push(2);
                put_u64(&mut b, *parent);
                put_u64(&mut b, *time);
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
                put_u64(&mut b, *time);
                put_str(&mut b, name);
                put_str(&mut b, new_name);
            }
            Record::Op(Op::Truncate { oid, size, time }) => {
                b.push(4);
                put_u64(&mut b, *oid);
                put_u64(&mut b, *size);
                put_u64(&mut b, *time);
            }
            Record::Op(Op::Symlink {
                parent,
                oid,
                name,
                target,
                time,
            }) => {
                b.push(7);
                put_u64(&mut b, *parent);
                put_u64(&mut b, *oid);
                put_u64(&mut b, *time);
                put_str(&mut b, name);
                put_str(&mut b, target);
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
                put_u64(&mut b, *mtime);
                b.extend_from_slice(&(data.len() as u32).to_le_bytes());
                b.extend_from_slice(data);
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
                put_u64(&mut b, *mtime);
                b.extend_from_slice(&(list.len() as u32).to_le_bytes());
                for (lb, e) in list {
                    put_u64(&mut b, *lb);
                    put_u64(&mut b, e.phys);
                    b.extend_from_slice(&(e.csums.len() as u32).to_le_bytes());
                    for c in &e.csums {
                        b.extend_from_slice(&c.0);
                    }
                }
            }
        }
    }
    b
}

struct Reader<'a> {
    b: &'a [u8],
    pos: usize,
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
    fn str(&mut self) -> Result<String> {
        let n = u16::from_le_bytes(self.take(2)?.try_into().unwrap()) as usize;
        Ok(core::str::from_utf8(self.take(n)?)
            .map_err(|_| Error::Corrupted)?
            .into())
    }
}

fn decode(b: &[u8]) -> Result<Vec<Record>> {
    let mut r = Reader { b, pos: 0 };
    let mut out = Vec::new();
    while r.pos < b.len() {
        let rec = match r.u8()? {
            1 => {
                let (parent, oid, kind, time) = (r.u64()?, r.u64()?, r.u8()?, r.u64()?);
                Record::Op(Op::Create {
                    parent,
                    oid,
                    kind,
                    time,
                    name: r.str()?,
                })
            }
            2 => {
                let (parent, time) = (r.u64()?, r.u64()?);
                Record::Op(Op::Remove {
                    parent,
                    time,
                    name: r.str()?,
                })
            }
            3 => {
                let (src, dst, time) = (r.u64()?, r.u64()?, r.u64()?);
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
                let (parent, oid, time) = (r.u64()?, r.u64()?, r.u64()?);
                let (name, target) = (r.str()?, r.str()?);
                Record::Op(Op::Symlink {
                    parent,
                    oid,
                    name,
                    target,
                    time,
                })
            }
            4 => Record::Op(Op::Truncate {
                oid: r.u64()?,
                size: r.u64()?,
                time: r.u64()?,
            }),
            5 => {
                let (oid, size, mtime) = (r.u64()?, r.u64()?, r.u64()?);
                let n = r.u32()? as usize;
                Record::Inline {
                    oid,
                    size,
                    mtime,
                    data: r.take(n)?.to_vec(),
                }
            }
            6 => {
                let (oid, size, mtime) = (r.u64()?, r.u64()?, r.u64()?);
                let count = r.u32()? as usize;
                let mut list = Vec::with_capacity(count.min(1024));
                for _ in 0..count {
                    let (lb, phys) = (r.u64()?, r.u64()?);
                    let n = r.u32()? as usize;
                    if n == 0 || n as u64 > crate::layout::MAX_EXTENT {
                        return Err(Error::Corrupted);
                    }
                    let csums = (0..n)
                        .map(|_| r.take(16).map(|c| Csum(c.try_into().unwrap())))
                        .collect::<Result<_>>()?;
                    list.push((
                        lb,
                        Extent {
                            phys,
                            birth: 0,
                            csums,
                        },
                    ));
                }
                Record::Extents {
                    oid,
                    size,
                    mtime,
                    list,
                }
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
    pub fn fsync(&mut self) -> Result<()> {
        self.s.check_writable()?;
        if self.zil.is_empty() {
            return Ok(());
        }
        if self.s.zil_blocks.len() > MAX_LOG_BLOCKS {
            return self.commit();
        }
        let mut records: Vec<Record> = self.zil.ops.drain(..).map(Record::Op).collect();
        let touched = core::mem::take(&mut self.zil.touched);
        for (oid, blocks) in touched {
            let root = self.s.root;
            let Some(raw) = btree::get(&mut self.s, &root, &Key::new(oid, item::INODE, 0))? else {
                continue;
            };
            let inode = crate::layout::Inode::decode(&raw)?;
            if inode.size as usize <= INLINE_MAX {
                let data = btree::get(&mut self.s, &root, &Key::new(oid, item::INLINE, 0))?
                    .unwrap_or_default();
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
                    if e.birth == txg && blocks.range(ks..ks + e.len()).next().is_some() {
                        self.s.zil_logged.insert(e.phys, e.len());
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
        let payload = encode(&records);
        let chunks: Vec<&[u8]> = payload.chunks(PAYLOAD).collect();
        let seed = self.s.sb.zil_seed;
        for (i, chunk) in chunks.iter().enumerate() {
            let next = self.s.alloc_meta()?;
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
        while block != 0 && block < self.s.sb.block_count {
            if self.s.dev.read_block(block, &mut buf).is_err() {
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
            let Ok(records) = decode(&batch) else { break };
            if !self.batch_durable(&records)? {
                break;
            }
            for r in records {
                self.apply(r)?;
                self.replayed += 1;
            }
        }
        if found {
            self.s.mark_changed();
            self.commit()?;
        }
        Ok(())
    }

    /// Jsou vsechna data davky na disku (souhlasi soucty)?
    fn batch_durable(&mut self, records: &[Record]) -> Result<bool> {
        for r in records {
            if let Record::Extents { list, .. } = r {
                for (_, e) in list {
                    let mut buf = vec![0u8; e.csums.len() * BLOCK_SIZE];
                    match self.s.read_data(e.phys, &e.csums, &mut buf) {
                        Ok(()) => {}
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
            Record::Op(Op::Create {
                parent,
                oid,
                kind,
                name,
                time,
            }) => {
                self.now = time;
                let kind = match kind {
                    crate::layout::KIND_DIR => Kind::Dir,
                    crate::layout::KIND_FILE => Kind::File,
                    _ => return Err(Error::Corrupted),
                };
                self.create_at(parent, &name, kind, oid).map_err(bad)
            }
            Record::Op(Op::Symlink {
                parent,
                oid,
                name,
                target,
                time,
            }) => {
                self.now = time;
                self.symlink_at(parent, &name, &target, oid).map_err(bad)
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
                    self.punch_range(oid, lb, lb + n)?;
                    self.s.space.claim(e.phys, n, txg)?;
                    self.s.mark_changed();
                    e.birth = txg;
                    self.put(Key::new(oid, item::EXTENT, lb), e.encode())?;
                    // Mapovani z logu ma prednost pred prevedenym obsahem.
                    if let Some(fd) = self.data.get_mut(&oid) {
                        let before = fd.blocks.len();
                        fd.blocks.retain(|&b, _| b < lb || b >= lb + n);
                        self.dirty_blocks -= (before - fd.blocks.len()) as u64;
                    }
                }
                let mut inode = self.live_inode(oid)?;
                inode.mtime = mtime;
                self.put_inode(oid, &inode)
            }
        }
    }
}
