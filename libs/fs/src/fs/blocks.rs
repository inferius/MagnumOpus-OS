//! Data na disku (faze 3): cteni extentu a zapis rozpracovanych bloku.
//!
//! **Cteni** (`extent_data`): nezapsany (predalokovany) extent = nuly;
//! komprimovany kus se precte cely (jeden soucet nad daty na disku),
//! rozbali a drzi v male mezipameti (`ChunkCache`), takze sekvencni cteni
//! po 4 KiB ho nerozbaluje znovu; nekomprimovany extent overi soucet
//! kazdeho bloku. Spatna prvni kopie = druha kopie a oprava (`Store`).
//!
//! **Zapis** (`writeback`) behu po sobe jdoucich bloku:
//! 1. Komprimovane kusy, do kterych se zapisuje, se cele vezmou do behu
//!    (kus je nedelitelny) a jejich extent se uvolni.
//! 2. Cast behu nad predalokovanym extentem, ktery nedrzi snapshot ani
//!    sdileni, se zapise **na misto** (zadna nova alokace, soubor zustane
//!    souvisly); po vypadku pred commitem extent porad cte nuly.
//! 3. Zbytek jde do novych bloku: samé nuly = **dira** (nic se nezapise,
//!    stara data se uvolni); s kompresi po kusech `COMP_CHUNK` zarovnanych
//!    na logicky offset, kus, ktery neusetri 1/8, se zapise nekomprimovany;
//!    s volbou kopii se data zapisou i podruhe (od konce disku).
//!
//! Poradi zustava "nejdriv data, pak prepojeni": kdyz alokace nebo zapis
//! selze, stare mapovani plati a nove bloky se hned vrati.

use alloc::collections::{BTreeMap, BTreeSet};
use alloc::vec;
use alloc::vec::Vec;

use abi::{Error, Result};

use super::Fs;
use crate::btree;
use crate::compress;
use crate::datarec::{DataSettings, DedupMode};
use crate::device::BlockDevice;
use crate::hash::Csum;
use crate::layout::{BLOCK_SIZE, COMP_CHUNK, Comp, Extent, Key, MAX_EXTENT, META_OID, item};
use crate::space::Class;

/// Kolik rozbalenych kusu drzet v pameti (8 x 128 KiB).
const CHUNK_CACHE: usize = 8;

/// Mezipamet rozbalenych komprimovanych kusu: (fyzicky zacatek, soucet)
/// -> data. Soucet je soucasti klice, takze znovu pouzity blok s jinym
/// obsahem se nikdy nesplete.
#[derive(Default)]
pub(super) struct ChunkCache(Vec<(u64, Csum, Vec<u8>)>);

fn is_zero(b: &[u8]) -> bool {
    b.iter().all(|&x| x == 0)
}

impl<D: BlockDevice> Fs<D> {
    /// Nastaveni dat aktivniho datasetu (v pameti po prvnim cteni).
    pub(super) fn data_settings(&mut self) -> Result<DataSettings> {
        if let Some(s) = self.dcfg.get(&self.cur_ds) {
            return Ok(*s);
        }
        let root = self.s.root;
        let s = match btree::get(&mut self.s, &root, &Key::new(META_OID, item::DATAOPT, 0))? {
            Some(v) => DataSettings::decode(&v)?,
            None => DataSettings::default(),
        };
        self.dcfg.insert(self.cur_ds, s);
        Ok(s)
    }

    /// Cely rozbaleny obsah komprimovaneho extentu (`len()` bloku).
    pub(super) fn chunk_data(&mut self, e: &Extent) -> Result<Vec<u8>> {
        let c = e.comp.ok_or(Error::Corrupted)?;
        let csum = e.csums[0];
        if let Some(i) = self.chunks.0.iter().position(|(p, s, _)| *p == e.phys && *s == csum) {
            let hit = self.chunks.0.remove(i);
            let data = hit.2.clone();
            self.chunks.0.push(hit);
            return Ok(data);
        }
        let packed = self.s.read_packed(e.phys, e.copy, c.plen as u64, csum)?;
        let data = compress::unpack(c.algo, &packed, c.bytes as usize, e.len() as usize * BLOCK_SIZE)?;
        if self.chunks.0.len() >= CHUNK_CACHE {
            self.chunks.0.remove(0);
        }
        self.chunks.0.push((e.phys, csum, data.clone()));
        Ok(data)
    }

    /// Logicke bloky `rel..rel+n` extentu `e` (overene, rozbalene).
    pub(super) fn extent_data(&mut self, e: &Extent, rel: u64, n: u64) -> Result<Vec<u8>> {
        debug_assert!(rel + n <= e.len());
        if e.is_unwritten() {
            return Ok(vec![0u8; n as usize * BLOCK_SIZE]);
        }
        if e.is_compressed() {
            let data = self.chunk_data(e)?;
            return Ok(data[rel as usize * BLOCK_SIZE..(rel + n) as usize * BLOCK_SIZE].to_vec());
        }
        let mut buf = vec![0u8; n as usize * BLOCK_SIZE];
        let copy = if e.copy != 0 { e.copy + rel } else { 0 };
        self.s.read_data(e.phys + rel, copy, &e.csums[rel as usize..(rel + n) as usize], &mut buf)?;
        Ok(buf)
    }

    /// Odstrani mapovani logickych bloku `a..b` (rozdeli castecne prekryte
    /// extenty) a uvolni jejich fyzicke bloky.
    pub(super) fn punch_range(&mut self, oid: u64, a: u64, b: u64) -> Result<()> {
        // Po davkach (pamet nezavisi na velikosti souboru); kazda davka
        // zacina za koncem posledniho zpracovaneho extentu.
        const BATCH: usize = 256;
        let mut from = a;
        while from < b {
            let root = self.s.root;
            let batch = self.extents_batch(&root, oid, from, b, BATCH)?;
            let full = batch.len() == BATCH;
            let Some(last_end) = batch.last().map(|(ks, e)| ks + e.len()) else {
                break;
            };
            self.punch_batch(oid, a, b, batch)?;
            if !full {
                break;
            }
            from = last_end.max(from + 1);
        }
        Ok(())
    }

    fn punch_batch(&mut self, oid: u64, a: u64, b: u64, batch: Vec<(u64, Extent)>) -> Result<()> {
        for (ks, e) in batch {
            let ke = ks + e.len();
            let lo = ks.max(a);
            let hi = ke.min(b);
            if lo >= hi {
                continue;
            }
            if e.is_compressed() && (lo > ks || hi < ke) {
                // Kus nejde rozdelit: zachovane bloky do bufferu (zapisou se
                // znovu), cely kus pryc.
                let data = self.chunk_data(&e)?;
                let fd = self.data.entry(oid).or_default();
                let mut added = 0;
                for j in 0..e.len() {
                    let lb = ks + j;
                    if (lb < lo || lb >= hi) && !fd.blocks.contains_key(&lb) {
                        let at = j as usize * BLOCK_SIZE;
                        fd.blocks.insert(lb, data[at..at + BLOCK_SIZE].to_vec());
                        added += 1;
                        self.zil.touch_block(oid, lb);
                    }
                }
                self.dirty_blocks += added;
                self.s.release_extent(&e, 0, e.plen());
                self.del(&Key::new(oid, item::EXTENT, ks))?;
                continue;
            }
            if e.is_compressed() {
                self.s.release_extent(&e, 0, e.plen());
            } else {
                self.s.release_extent(&e, lo - ks, hi - ks);
            }
            match e.slice(0, lo - ks) {
                Some(left) => self.put(Key::new(oid, item::EXTENT, ks), left.encode())?,
                None => {
                    self.del(&Key::new(oid, item::EXTENT, ks))?;
                }
            }
            if let Some(right) = e.slice(hi - ks, e.len()) {
                self.put(Key::new(oid, item::EXTENT, hi), right.encode())?;
            }
        }
        Ok(())
    }

    /// Fyzicky blok hned za extentem, ktery mapuje logicky blok `lb`.
    fn goal_after(&mut self, oid: u64, lb: u64) -> Result<Option<u64>> {
        let root = self.s.root;
        Ok(self
            .extents_in(&root, oid, lb, lb + 1)?
            .into_iter()
            .next()
            .map(|(ks, e)| if e.is_compressed() { e.phys + e.plen() } else { e.phys + (lb - ks) + 1 }))
    }

    /// Zapise rozpracovane bloky souboru na disk (vsechny, nebo jen `only`).
    pub(super) fn writeback(&mut self, oid: u64, only: Option<&BTreeSet<u64>>) -> Result<()> {
        let Some(fd) = self.data.get_mut(&oid) else {
            return Ok(());
        };
        let mut picked: BTreeMap<u64, Vec<u8>> = match only {
            None => core::mem::take(&mut fd.blocks),
            Some(set) => {
                let keys: Vec<u64> = set.iter().copied().filter(|lb| fd.blocks.contains_key(lb)).collect();
                keys.into_iter().map(|lb| (lb, fd.blocks.remove(&lb).unwrap())).collect()
            }
        };
        if fd.blocks.is_empty() {
            self.data.remove(&oid);
        }
        self.dirty_blocks -= picked.len() as u64;
        if let Err(e) = self.writeback_picked(oid, &mut picked) {
            // Nezapsane bloky vratit do bufferu: chyba zapisu nesmi ztratit
            // data, ktera klient uz predal (zkusi se znovu).
            let fd = self.data.entry(oid).or_default();
            for (lb, b) in picked {
                if fd.blocks.insert(lb, b).is_none() {
                    self.dirty_blocks += 1;
                }
            }
            return Err(e);
        }
        Ok(())
    }

    fn writeback_picked(&mut self, oid: u64, picked: &mut BTreeMap<u64, Vec<u8>>) -> Result<()> {
        if picked.is_empty() {
            return Ok(());
        }
        let cfg = self.data_settings()?;
        self.pull_packed(oid, picked)?;
        let keys: Vec<u64> = picked.keys().copied().collect();
        let mut i = 0;
        while i < keys.len() {
            let mut j = i + 1;
            while j < keys.len() && keys[j] == keys[j - 1] + 1 {
                j += 1;
            }
            self.write_run(oid, keys[i], keys[j - 1] + 1, picked, &cfg)?;
            i = j;
        }
        Ok(())
    }

    /// Komprimovane kusy, do kterych se zapisuje, vezme cele do `picked`
    /// (novejsi obsah z bufferu ma prednost) a jejich extent uvolni.
    fn pull_packed(&mut self, oid: u64, picked: &mut BTreeMap<u64, Vec<u8>>) -> Result<()> {
        let (Some(&lo), Some(&hi)) = (picked.keys().next(), picked.keys().next_back()) else {
            return Ok(());
        };
        let mut from = lo;
        let mut packed: Vec<(u64, Extent)> = Vec::new();
        loop {
            let root = self.s.root;
            let batch = self.extents_batch(&root, oid, from, hi + 1, 256)?;
            let full = batch.len() == 256;
            let next = batch.last().map(|(ks, e)| ks + e.len());
            packed.extend(
                batch
                    .into_iter()
                    .filter(|(ks, e)| e.is_compressed() && picked.range(*ks..ks + e.len()).next().is_some()),
            );
            match next {
                Some(n) if full => from = n.max(from + 1),
                _ => break,
            }
        }
        for (ks, e) in packed {
            let data = self.chunk_data(&e)?;
            for j in 0..e.len() {
                let lb = ks + j;
                if picked.contains_key(&lb) {
                    continue;
                }
                let newer = self.data.get_mut(&oid).and_then(|fd| fd.blocks.remove(&lb));
                let block = match newer {
                    Some(b) => {
                        self.dirty_blocks -= 1;
                        b
                    }
                    None => data[j as usize * BLOCK_SIZE..(j + 1) as usize * BLOCK_SIZE].to_vec(),
                };
                picked.insert(lb, block);
            }
            if self.data.get(&oid).is_some_and(|fd| fd.blocks.is_empty()) {
                self.data.remove(&oid);
            }
            self.s.release_extent(&e, 0, e.plen());
            self.del(&Key::new(oid, item::EXTENT, ks))?;
        }
        Ok(())
    }

    /// Zapise beh `a..b`: nad predalokovanymi bloky na misto, jinde nove.
    fn write_run(&mut self, oid: u64, a: u64, b: u64, picked: &mut BTreeMap<u64, Vec<u8>>, cfg: &DataSettings) -> Result<()> {
        let root = self.s.root;
        let snap_gen = self.s.snapshot_generation;
        let mut segs: Vec<(u64, u64, bool)> = Vec::new();
        let mut pos = a;
        for (ks, e) in self.extents_in(&root, oid, a, b)? {
            if !e.is_unwritten() || e.is_shared() || e.birth <= snap_gen {
                continue;
            }
            let lo = ks.max(a);
            let hi = (ks + e.len()).min(b);
            if lo >= hi {
                continue;
            }
            if pos < lo {
                segs.push((pos, lo, false));
            }
            segs.push((lo, hi, true));
            pos = hi;
        }
        if pos < b {
            segs.push((pos, b, false));
        }
        for (lo, hi, in_place) in segs {
            if in_place {
                self.write_in_place(oid, lo, hi, picked)?;
            } else {
                self.write_new(oid, lo, hi, picked, cfg)?;
            }
        }
        Ok(())
    }

    /// Zapis do predalokovaneho (nezapsaneho) extentu na misto. Nulove
    /// bloky zustanou nezapsane (ctou se jako nuly i tak).
    fn write_in_place(&mut self, oid: u64, lo: u64, hi: u64, picked: &mut BTreeMap<u64, Vec<u8>>) -> Result<()> {
        let mut lb = lo;
        while lb < hi {
            let zero = is_zero(&picked[&lb]);
            let mut end = lb + 1;
            while end < hi && is_zero(&picked[&end]) == zero {
                end += 1;
            }
            if zero {
                for k in lb..end {
                    picked.remove(&k);
                }
                lb = end;
                continue;
            }
            let root = self.s.root;
            let (ks, e) = self
                .extents_in(&root, oid, lb, lb + 1)?
                .into_iter()
                .next()
                .filter(|(_, e)| e.is_unwritten())
                .ok_or(Error::Corrupted)?;
            let end = end.min(ks + e.len());
            let mut buf = Vec::with_capacity((end - lb) as usize * BLOCK_SIZE);
            for k in lb..end {
                buf.extend_from_slice(&picked[&k]);
            }
            let rel = lb - ks;
            self.s.dev.write_blocks(e.phys + rel, &buf)?;
            if e.copy != 0 {
                self.s.dev.write_blocks(e.copy + rel, &buf)?;
            }
            let algo = self.s.algo;
            let csums: Vec<Csum> = buf.chunks(BLOCK_SIZE).map(|c| algo.sum(c)).collect();
            let r = self.relink_in_place(oid, ks, &e, rel, csums);
            if r.is_err() {
                self.s.poisoned = true;
                return r;
            }
            self.inplace.insert(e.phys + rel, end - lb);
            for k in lb..end {
                picked.remove(&k);
            }
            lb = end;
        }
        Ok(())
    }

    /// Nezapsany extent `ks` -> [nezapsany][zapsany s `csums`][nezapsany].
    /// Generace vzniku zustava (bloky patri temuz odkazu).
    pub(super) fn relink_in_place(&mut self, oid: u64, ks: u64, e: &Extent, rel: u64, csums: Vec<Csum>) -> Result<()> {
        let n = csums.len() as u64;
        let mut mid = e.slice(rel, rel + n).ok_or(Error::Corrupted)?;
        mid.flags &= !crate::layout::EXTENT_UNWRITTEN;
        mid.csums = csums;
        match e.slice(0, rel) {
            Some(left) => self.put(Key::new(oid, item::EXTENT, ks), left.encode())?,
            None => {
                self.del(&Key::new(oid, item::EXTENT, ks))?;
            }
        }
        self.put(Key::new(oid, item::EXTENT, ks + rel), mid.encode())?;
        if let Some(right) = e.slice(rel + n, e.len()) {
            self.put(Key::new(oid, item::EXTENT, ks + rel + n), right.encode())?;
        }
        self.s.mark_changed();
        Ok(())
    }

    /// Zapis `lo..hi` do novych bloku: diry, kusy komprese, nekomprimovane
    /// useky.
    fn write_new(&mut self, oid: u64, lo: u64, hi: u64, picked: &mut BTreeMap<u64, Vec<u8>>, cfg: &DataSettings) -> Result<()> {
        let mut lb = lo;
        while lb < hi {
            let end;
            let zero;
            if cfg.compress.is_some() {
                // Kus: do konce zarovnaneho kusu; nuly uvnitr se komprimuji
                // s nim, cely nulovy kus je dira.
                end = hi.min((lb / COMP_CHUNK + 1) * COMP_CHUNK);
                zero = (lb..end).all(|k| is_zero(&picked[&k]));
            } else {
                zero = is_zero(&picked[&lb]);
                let mut e = lb + 1;
                while e < hi && is_zero(&picked[&e]) == zero {
                    e += 1;
                }
                end = e;
            }
            if zero {
                self.punch_range(oid, lb, end)?;
                for k in lb..end {
                    picked.remove(&k);
                }
            } else {
                self.write_piece(oid, lb, end, picked, cfg)?;
            }
            lb = end;
        }
        Ok(())
    }

    /// Alokace `n` bloku pro kopii dat (od konce disku, jeden souvisly usek).
    fn alloc_copy(&mut self, n: u64) -> Result<u64> {
        let txg = self.s.txg;
        self.s.mark_changed();
        let (s, got) = self.s.space.alloc_min(n, n, Class::MetaHigh, txg)?;
        debug_assert_eq!(got, n);
        Ok(s)
    }

    fn write_piece(&mut self, oid: u64, a: u64, b: u64, picked: &mut BTreeMap<u64, Vec<u8>>, cfg: &DataSettings) -> Result<()> {
        let n = b - a;
        let mut raw = Vec::with_capacity(n as usize * BLOCK_SIZE);
        for k in a..b {
            raw.extend_from_slice(&picked[&k]);
        }
        if let Some(algo) = cfg.compress
            && let Some((packed, bytes)) = compress::pack(algo, &raw)
            && self.write_packed(oid, a, n, &packed, bytes, algo, cfg)?
        {
            for k in a..b {
                picked.remove(&k);
            }
            return Ok(());
        }
        let csums: Vec<Csum> = {
            let algo = self.s.algo;
            raw.chunks(BLOCK_SIZE).map(|c| algo.sum(c)).collect()
        };
        if cfg.dedup == DedupMode::Inline && n == COMP_CHUNK && a % COMP_CHUNK == 0 && self.dedup_inline(oid, a, &raw, &csums)? {
            for k in a..b {
                picked.remove(&k);
            }
            return Ok(());
        }
        let inline = cfg.dedup == DedupMode::Inline && n == COMP_CHUNK && a % COMP_CHUNK == 0;
        let hash = inline.then(|| super::dedup::plain_hash(self.s.algo, &csums));
        self.write_plain(oid, a, &raw, csums, cfg)?;
        for k in a..b {
            picked.remove(&k);
        }
        if let Some(h) = hash {
            self.dedup_index_written(oid, a, h)?;
        }
        Ok(())
    }

    /// Komprimovany kus do jednoho souvisleho useku. `false` = souvisle misto
    /// neni (zapise se nekomprimovany po kusech).
    #[allow(clippy::too_many_arguments, reason = "jeden kus komprese: misto, data a nastaveni")]
    fn write_packed(
        &mut self,
        oid: u64,
        a: u64,
        n: u64,
        packed: &[u8],
        bytes: usize,
        algo: crate::layout::Compress,
        cfg: &DataSettings,
    ) -> Result<bool> {
        let plen = (packed.len() / BLOCK_SIZE) as u64;
        let goal = if a > 0 { self.goal_after(oid, a - 1)? } else { None };
        let txg = self.s.txg;
        let (phys, got) = self.s.alloc_data(plen, goal)?;
        if got < plen {
            self.s.space.free(phys, got, txg, true);
            return Ok(false);
        }
        let copy = if cfg.copies > 1 {
            match self.alloc_copy(plen) {
                Ok(c) => c,
                Err(e) => {
                    self.s.space.free(phys, plen, txg, true);
                    return Err(e);
                }
            }
        } else {
            0
        };
        let w = self.s.dev.write_blocks(phys, packed).and_then(|()| {
            if copy != 0 { self.s.dev.write_blocks(copy, packed) } else { Ok(()) }
        });
        if let Err(e) = w {
            self.s.space.free(phys, plen, txg, true);
            if copy != 0 {
                self.s.space.free(copy, plen, txg, true);
            }
            return Err(e);
        }
        let e = Extent {
            phys,
            birth: txg,
            csums: vec![self.s.algo.sum(packed)],
            blocks: n as u32,
            flags: 0,
            copy,
            comp: Some(Comp {
                algo,
                plen: plen as u32,
                bytes: bytes as u32,
            }),
        };
        let r = self.punch_range(oid, a, a + n).and_then(|()| self.put(Key::new(oid, item::EXTENT, a), e.encode()));
        if r.is_err() {
            self.s.poisoned = true;
        }
        r.map(|()| true)
    }

    /// Nekomprimovana data od logickeho bloku `a` do novych bloku (tolik
    /// useku, kolik da alokator), pak prepojeni extentu.
    fn write_plain(&mut self, oid: u64, a: u64, raw: &[u8], csums: Vec<Csum>, cfg: &DataSettings) -> Result<()> {
        let n = csums.len() as u64;
        let mut goal = if a > 0 { self.goal_after(oid, a - 1)? } else { None };
        let txg = self.s.txg;
        // (logicky offset, fyzicky zacatek, pocet, kopie)
        let mut pieces: Vec<(u64, u64, u64, u64)> = Vec::new();
        let mut done = 0u64;
        while done < n {
            let r = self.s.alloc_data(n - done, goal).and_then(|(start, got)| {
                let copy = if cfg.copies > 1 {
                    match self.alloc_copy(got) {
                        Ok(c) => c,
                        Err(e) => {
                            self.s.space.free(start, got, txg, true);
                            return Err(e);
                        }
                    }
                } else {
                    0
                };
                pieces.push((done, start, got, copy));
                let buf = &raw[done as usize * BLOCK_SIZE..(done + got) as usize * BLOCK_SIZE];
                self.s.dev.write_blocks(start, buf)?;
                if copy != 0 {
                    self.s.dev.write_blocks(copy, buf)?;
                }
                Ok((start, got))
            });
            match r {
                Ok((start, got)) => {
                    done += got;
                    goal = Some(start + got);
                }
                Err(e) => {
                    for &(_, start, got, copy) in &pieces {
                        self.s.space.free(start, got, txg, true);
                        if copy != 0 {
                            self.s.space.free(copy, got, txg, true);
                        }
                    }
                    return Err(e);
                }
            }
        }
        // Od ted se meni strom: chyba (cteni uzlu) by nechala beh napul
        // prepojeny - svazek do stavu chyby, nic se nepotvrdi.
        let r = self.relink_plain(oid, a, n, &csums, &pieces);
        if r.is_err() {
            self.s.poisoned = true;
        }
        r
    }

    fn relink_plain(&mut self, oid: u64, a: u64, n: u64, csums: &[Csum], pieces: &[(u64, u64, u64, u64)]) -> Result<()> {
        let txg = self.s.txg;
        self.punch_range(oid, a, a + n)?;
        for &(off0, start, got, copy) in pieces {
            let piece = &csums[off0 as usize..(off0 + got) as usize];
            for (k, chunk) in piece.chunks(MAX_EXTENT as usize).enumerate() {
                let off = k as u64 * MAX_EXTENT;
                let mut e = Extent::plain(start + off, txg, chunk.to_vec());
                if copy != 0 {
                    e.copy = copy + off;
                }
                self.put(Key::new(oid, item::EXTENT, a + off0 + off), e.encode())?;
            }
        }
        Ok(())
    }

    /// Zapise vsechna rozpracovana data.
    pub(super) fn flush_all_data(&mut self) -> Result<()> {
        let oids: Vec<u64> = self.data.keys().copied().collect();
        for oid in oids {
            self.writeback(oid, None)?;
        }
        Ok(())
    }
}
