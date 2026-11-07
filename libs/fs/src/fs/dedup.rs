//! Deduplikace (faze 3): stejne kusy dat ulozene jen jednou.
//!
//! Kus = zarovnanych `COMP_CHUNK` (32) logickych bloku jednoho extentu
//! (128 KiB, fyzicky souvisle). Otisk kusu:
//! - nekomprimovany: soucet (algoritmus svazku) z prefixu "P" a souctu
//!   vsech 32 bloku - nic se necte, soucty uz jsou v extentu,
//! - komprimovany extent: soucet z prefixu "C", algoritmu, delek a souctu
//!   dat na disku (stejny vstup se stejne zkomprimuje).
//!
//! Index (`item::DEDUP`) vede otisk -> zaznam tabulky sdileni (kus
//! s poctem odkazu). Shoda: u XXH3 se data porovnaji bajt po bajtu
//! (ochrana proti zamerne kolizi), u BLAKE3 ne. Pak se extent souboru
//! prepoji na nalezeny kus (priznak SHARED, generace vzniku = tato
//! transakce) a vlastni bloky se uvolni.
//!
//! **Na pozadi** (`dedup_step`, vychozi): prochazi zive stromy datasetu
//! jen v podstromech narozenych po posledni dokoncene pruchodu
//! (`DEDUPPOS`), takze cena je umerna novym datum. Soubory s rozpracovanymi
//! daty preskoci (prijdou na radu priste). **Pri zapisu** (volba
//! `DedupMode::Inline`): kus se hleda pred alokaci (`dedup_inline`).
//!
//! Pamet: index je na disku, v pameti jen cache uzlu stromu.

use alloc::vec::Vec;

use abi::{Error, Result};

use super::Fs;
use crate::btree;
use crate::datarec::DedupMode;
use crate::device::BlockDevice;
use crate::hash::{Algo, Csum};
use crate::layout::{COMP_CHUNK, EXTENT_SHARED, Extent, Key, META_OID, ROOT_DATASET, item, u64_at};

/// Vysledek kroku deduplikace na pozadi.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DedupReport {
    /// Prohlednute kusy.
    pub chunks: u64,
    /// Kusy nahrazene odkazem na stejny kus (usetrene bloky = 32 x).
    pub merged: u64,
    /// Usetrene bloky.
    pub saved_blocks: u64,
    /// Pruchod vsech datasetu skoncil (pristi krok zacne novy).
    pub done: bool,
}

/// Rozpracovany pruchod deduplikace na pozadi (v pameti; po restartu
/// zacne pruchod znovu od posledni dokoncene generace).
#[derive(Default)]
pub(super) struct DedupPass {
    /// Datasety k projiti (zbyvajici) a generace na zacatku pruchodu.
    pending: Vec<u64>,
    generation: u64,
    /// Kde v aktualnim datasetu pokracovat.
    next_key: Option<Key>,
}

/// Otisk nekomprimovaneho kusu ze souctu jeho bloku.
pub(super) fn plain_hash(algo: Algo, csums: &[Csum]) -> Csum {
    let mut b = Vec::with_capacity(1 + csums.len() * 16);
    b.push(b'P');
    for c in csums {
        b.extend_from_slice(&c.0);
    }
    algo.sum(&b)
}

/// Otisk komprimovaneho extentu.
fn packed_hash(algo: Algo, e: &Extent) -> Option<Csum> {
    let c = e.comp?;
    let mut b = Vec::with_capacity(32);
    b.push(b'C');
    b.push(c.algo as u8);
    b.extend_from_slice(&e.blocks.to_le_bytes());
    b.extend_from_slice(&c.plen.to_le_bytes());
    b.extend_from_slice(&c.bytes.to_le_bytes());
    b.extend_from_slice(&e.csums[0].0);
    Some(algo.sum(&b))
}

/// Kandidat na deduplikaci: (logicky zacatek, extent kusu, otisk).
type Candidate = (u64, Extent, Csum);

impl<D: BlockDevice> Fs<D> {
    /// Kusy extentu `e` na logickem `ks` vhodne k deduplikaci.
    fn chunks_of(&self, ks: u64, e: &Extent) -> Vec<Candidate> {
        let algo = self.s.algo;
        if e.is_unwritten() || e.is_shared() {
            return Vec::new();
        }
        if e.is_compressed() {
            return packed_hash(algo, e).map(|h| (ks, e.clone(), h)).into_iter().collect();
        }
        let mut out = Vec::new();
        let mut lb = ks.next_multiple_of(COMP_CHUNK);
        while lb + COMP_CHUNK <= ks + e.len() {
            let rel = lb - ks;
            let part = e.slice(rel, rel + COMP_CHUNK).expect("cast extentu");
            let h = plain_hash(algo, &part.csums);
            out.push((lb, part, h));
            lb += COMP_CHUNK;
        }
        out
    }

    /// Jsou data kusu `a` a `b` stejna? U BLAKE3 staci otisk (zde uz
    /// shodny), u XXH3 se porovnaji bajty.
    fn same_data(&mut self, a: &Extent, b_phys: u64, b_copy: u64) -> Result<bool> {
        if self.s.algo == Algo::Blake3 {
            return Ok(true);
        }
        let mut other = a.clone();
        other.phys = b_phys;
        other.copy = b_copy;
        let x = self.raw_of(a)?;
        let y = self.raw_of(&other)?;
        Ok(x == y)
    }

    /// Data kusu tak, jak lezi na disku (overena souctem).
    fn raw_of(&mut self, e: &Extent) -> Result<Vec<u8>> {
        match e.comp {
            Some(c) => self.s.read_packed(e.phys, e.copy, c.plen as u64, e.csums[0]),
            None => self.extent_data(e, 0, e.len()),
        }
    }

    /// Prepoji kus souboru `oid` na `lb` na sdileny usek `start` (odkaz +1,
    /// vlastni bloky se uvolni).
    fn remap_chunk(&mut self, oid: u64, lb: u64, part: &Extent, start: u64, copy: u64) -> Result<()> {
        let mut e = part.clone();
        e.phys = start;
        e.copy = copy;
        e.birth = self.s.txg;
        e.flags |= EXTENT_SHARED;
        self.share_ref(start, e.plen(), copy)?;
        self.punch_range(oid, lb, lb + e.len())?;
        self.put(Key::new(oid, item::EXTENT, lb), e.encode())?;
        self.s.mark_changed();
        Ok(())
    }

    /// Deduplikace kusu pri zapisu: kdyz index zna stejny kus, extent na
    /// `a` ukaze na nej a nic se nezapise (`true`). Jinak `false` a zapis
    /// pokracuje normalne; novy kus se zaindexuje (`dedup_index_written`).
    pub(super) fn dedup_inline(&mut self, oid: u64, a: u64, raw: &[u8], csums: &[Csum]) -> Result<bool> {
        let h = plain_hash(self.s.algo, csums);
        let Some((start, entry)) = self.dedup_lookup(&h)? else {
            return Ok(false);
        };
        if entry.len as u64 != COMP_CHUNK {
            return Ok(false);
        }
        let part = Extent::plain(start, self.s.txg, csums.to_vec());
        if self.s.algo != Algo::Blake3 {
            let mut other = part.clone();
            other.copy = entry.copy;
            if self.raw_of(&other)? != raw {
                return Ok(false);
            }
        }
        self.remap_chunk(oid, a, &part, start, entry.copy)?;
        Ok(true)
    }

    /// Po zapisu kusu na `a` (deduplikace pri zapisu): kdyz lezi v jednom
    /// extentu, zaindexuje ho a extent oznaci jako sdileny (bloky z teto
    /// transakce by se jinak smely uvolnit hned, i kdyz na ne uz ukazuje
    /// index).
    pub(super) fn dedup_index_written(&mut self, oid: u64, a: u64, h: Csum) -> Result<()> {
        let root = self.s.root;
        let Some((ks, e)) = self.extents_in(&root, oid, a, a + 1)?.into_iter().next() else {
            return Ok(());
        };
        if ks != a || e.len() != COMP_CHUNK || e.is_compressed() || e.is_unwritten() {
            return Ok(());
        }
        if !self.share_index(e.phys, COMP_CHUNK, e.copy, h)? {
            return Ok(());
        }
        let mut flagged = e.clone();
        flagged.flags |= EXTENT_SHARED;
        self.put(Key::new(oid, item::EXTENT, a), flagged.encode())
    }

    /// Generace, do ktere je dataset `id` deduplikovany.
    fn dedup_pos(&mut self) -> Result<u64> {
        let root = self.s.root;
        Ok(match btree::get(&mut self.s, &root, &Key::new(META_OID, item::DEDUPPOS, 0))? {
            Some(v) if v.len() == 8 => u64_at(&v, 0),
            Some(_) => return Err(Error::Corrupted),
            None => 0,
        })
    }

    /// Krok deduplikace na pozadi: projde nejvys `budget` kusu novych dat
    /// (vsech datasetu s deduplikaci). Volat opakovane (fsd pri
    /// necinnosti); zmeny ulozi commit.
    pub fn dedup_step(&mut self, budget: u64) -> Result<DedupReport> {
        self.require_system()?;
        self.s.check_writable()?;
        let mut rep = DedupReport::default();
        if self.dedup.pending.is_empty() && self.dedup.next_key.is_none() {
            // Novy pruchod: vsechny datasety, hranice = posledni commit.
            self.commit()?;
            let mut ids = alloc::vec![ROOT_DATASET];
            ids.extend(self.all_dataset_ids()?);
            ids.reverse();
            self.dedup.pending = ids;
            self.dedup.generation = self.s.sb.generation;
        }
        let back = self.cur_ds;
        let r = self.dedup_run(budget, &mut rep);
        self.activate(back)?;
        r?;
        if rep.merged > 0 {
            self.commit()?;
        }
        Ok(rep)
    }

    fn dedup_run(&mut self, budget: u64, rep: &mut DedupReport) -> Result<()> {
        while let Some(&id) = self.dedup.pending.last() {
            self.activate(id)?;
            if self.data_settings()?.dedup == DedupMode::Off {
                self.dedup.pending.pop();
                self.dedup.next_key = None;
                continue;
            }
            let pos = self.dedup_pos()?;
            let from = self.dedup.next_key.unwrap_or(Key::MIN);
            // Kandidati z novych podstromu (od `from`), nejvys `budget`.
            let mut found: Vec<(u64, Candidate)> = Vec::new();
            let mut last: Option<Key> = None;
            let root = self.s.root;
            let txg = self.s.txg;
            let mut bad = false;
            let mut cand: Vec<(Key, Extent)> = Vec::new();
            btree::range_newer(&mut self.s, &root, &from, pos, &mut |k, v| {
                last = Some(*k);
                if k.kind == item::EXTENT {
                    match Extent::decode(v) {
                        // Jen ulozena data: bloky z rozpracovane transakce
                        // by se smely uvolnit hned, i kdyz uz jsou v indexu.
                        Ok(e) if e.birth > pos && e.birth < txg => cand.push((*k, e)),
                        Ok(_) => {}
                        Err(_) => {
                            bad = true;
                            return Ok(false);
                        }
                    }
                }
                Ok((cand.len() as u64) < budget)
            })?;
            if bad {
                return Err(Error::Corrupted);
            }
            let finished = (cand.len() as u64) < budget;
            for (k, e) in cand {
                if self.data.contains_key(&k.oid) {
                    continue;
                }
                for c in self.chunks_of(k.offset, &e) {
                    found.push((k.oid, c));
                }
            }
            for (oid, (lb, part, h)) in found {
                rep.chunks += 1;
                match self.dedup_lookup(&h)? {
                    Some((start, entry)) if start != part.phys && entry.len as u64 == part.plen() => {
                        if self.same_data(&part, start, entry.copy)? {
                            self.remap_chunk(oid, lb, &part, start, entry.copy)?;
                            rep.merged += 1;
                            rep.saved_blocks += part.plen();
                        }
                    }
                    Some(_) => {}
                    None => {
                        self.share_index(part.phys, part.plen(), part.copy, h)?;
                    }
                }
            }
            if finished {
                // Dataset hotovy az do generace zacatku pruchodu.
                let g = self.dedup.generation;
                self.put(Key::new(META_OID, item::DEDUPPOS, 0), g.to_le_bytes().to_vec())?;
                self.dedup.pending.pop();
                self.dedup.next_key = None;
            } else {
                self.dedup.next_key = last.map(|k| Key::new(k.oid, k.kind, k.offset + 1));
                return Ok(());
            }
            if rep.chunks >= budget {
                return Ok(());
            }
        }
        rep.done = true;
        Ok(())
    }
}
