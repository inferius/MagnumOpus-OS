//! Clustery exFAT: retezy (FAT nebo souvisle `NoFatChain`), bitmapa
//! alokace, prideleni a uvolneni, cteni a zapis dat po clusterech.

use volproto::{Error, Result};

use super::entry::{FileSet, sflag};
use super::{ExFatVolume, read_at, write_at};
use crate::dev::Sectors;

/// Konec retezu (hodnoty >= `0xFFFFFFF8`; `0xFFFFFFF7` = vadny cluster).
const EOC: u32 = 0xFFFF_FFFF;
const LAST: u32 = 0xFFFF_FFF8;

impl<D: Sectors> ExFatVolume<D> {
    fn fat_get(&mut self, c: u32) -> Result<u32> {
        let mut b = [0u8; 4];
        read_at(&mut self.dev, self.g.fat_pos(c), &mut b)?;
        Ok(u32::from_le_bytes(b))
    }

    fn fat_set(&mut self, c: u32, v: u32) -> Result<()> {
        write_at(&mut self.dev, self.g.fat_pos(c), &v.to_le_bytes())
    }

    /// Retez az po znacku konce (koren, uvolneni retezu FAT).
    pub(super) fn chain_end(&mut self, first: u32) -> Result<Vec<u32>> {
        let mut out = Vec::new();
        let mut c = first;
        loop {
            if !self.g.valid(c) || out.len() >= self.g.cluster_count as usize {
                return Err(Error::Corrupted);
            }
            out.push(c);
            let next = self.fat_get(c)?;
            if next >= LAST {
                return Ok(out);
            }
            c = next;
        }
    }

    /// Clustery dat delky `len` od `first`.
    pub(super) fn chain(&mut self, first: u32, no_fat: bool, len: u64) -> Result<Vec<u32>> {
        let n = len.div_ceil(self.g.cluster_size());
        if n == 0 {
            return Ok(Vec::new());
        }
        if first == 0 || n > self.g.cluster_count as u64 {
            return Err(Error::Corrupted);
        }
        let n = n as u32;
        if no_fat {
            let last = first.checked_add(n - 1).ok_or(Error::Corrupted)?;
            if !self.g.valid(first) || !self.g.valid(last) {
                return Err(Error::Corrupted);
            }
            return Ok((first..=last).collect());
        }
        let mut out = Vec::with_capacity(n as usize);
        let mut c = first;
        for i in 0..n {
            if !self.g.valid(c) {
                return Err(Error::Corrupted);
            }
            out.push(c);
            if i + 1 < n {
                c = self.fat_get(c)?;
            }
        }
        Ok(out)
    }

    /// Clustery souboru (posledni retez v pameti: cteni po kouscich).
    pub(super) fn file_chain(&mut self, f: &FileSet) -> Result<Vec<u32>> {
        let key = (f.first_cluster, f.no_fat_chain(), f.len);
        if let Some((k, c)) = &self.chain_cache
            && *k == key
        {
            return Ok(c.clone());
        }
        let c = self.chain(f.first_cluster, f.no_fat_chain(), f.len)?;
        self.chain_cache = Some((key, c.clone()));
        Ok(c)
    }

    fn is_used(&self, c: u32) -> bool {
        let i = (c - 2) as usize;
        self.bitmap.bits[i / 8] & (1 << (i % 8)) != 0
    }

    /// Bit clusteru v bitmape (v pameti i na mediu).
    fn mark(&mut self, c: u32, used: bool) -> Result<()> {
        if !self.g.valid(c) {
            return Err(Error::Corrupted);
        }
        if self.is_used(c) == used {
            return Ok(());
        }
        let i = (c - 2) as usize;
        let byte = i / 8;
        self.bitmap.bits[byte] ^= 1 << (i % 8);
        if used {
            self.bitmap.free -= 1;
        } else {
            self.bitmap.free += 1;
        }
        let cs = self.g.cluster_size() as usize;
        let bc = *self.bitmap.clusters.get(byte / cs).ok_or(Error::Corrupted)?;
        let pos = self.g.cluster_pos(bc) + (byte % cs) as u64;
        let v = self.bitmap.bits[byte];
        write_at(&mut self.dev, pos, &[v])
    }

    /// `n` volnych clusteru, pokud mozno souvisle za `after`.
    fn alloc(&mut self, n: usize, after: Option<u32>) -> Result<Vec<u32>> {
        if n as u64 > self.bitmap.free as u64 {
            return Err(Error::NoSpace);
        }
        let first = 2u32;
        let end = 2 + self.g.cluster_count;
        let start = match after {
            Some(a) if a + 1 < end && !self.is_used(a + 1) => a + 1,
            _ => self.bitmap.hint.clamp(first, end - 1),
        };
        let mut out = Vec::with_capacity(n);
        let mut c = start;
        for _ in 0..self.g.cluster_count {
            if out.len() == n {
                break;
            }
            if !self.is_used(c) {
                out.push(c);
            }
            c = if c + 1 >= end { first } else { c + 1 };
        }
        if out.len() < n {
            return Err(Error::NoSpace);
        }
        for &c in &out {
            self.mark(c, true)?;
        }
        self.bitmap.hint = c;
        Ok(out)
    }

    /// Zmeni alokaci souboru na `len` bajtu (clustery, prvni cluster,
    /// `NoFatChain`); delku v sade nemeni. Vraci nove clustery souboru.
    pub(super) fn resize(&mut self, f: &mut FileSet, len: u64) -> Result<Vec<u32>> {
        let need = len.div_ceil(self.g.cluster_size());
        if need > self.g.cluster_count as u64 {
            return Err(Error::NoSpace);
        }
        let need = need as usize;
        let mut cl = self.file_chain(f)?;
        self.chain_cache = None;
        f.flags |= sflag::ALLOC_POSSIBLE;
        if need > cl.len() {
            let new = self.alloc(need - cl.len(), cl.last().copied())?;
            let follows = cl.last().is_none_or(|&l| new[0] == l + 1);
            let contiguous = follows && new.windows(2).all(|w| w[1] == w[0] + 1);
            if contiguous && (cl.is_empty() || f.no_fat_chain()) {
                f.flags |= sflag::NO_FAT_CHAIN;
            } else {
                // Retez FAT: puvodne souvisly soubor nejdriv prevest.
                let mut all = cl.clone();
                let from = if f.no_fat_chain() { 0 } else { all.len().saturating_sub(1) };
                all.extend_from_slice(&new);
                for i in from..all.len().saturating_sub(1) {
                    self.fat_set(all[i], all[i + 1])?;
                }
                if let Some(&l) = all.last() {
                    self.fat_set(l, EOC)?;
                }
                f.flags &= !sflag::NO_FAT_CHAIN;
            }
            if cl.is_empty() {
                f.first_cluster = new[0];
            }
            cl.extend(new);
        } else if need < cl.len() {
            // U retezu FAT uvolnit vse az po znacku konce (i clustery za
            // delkou souboru, ktere tam nekdo nechal).
            let freed = if f.no_fat_chain() { cl.split_off(need) } else {
                let all = self.chain_end(f.first_cluster)?;
                cl.truncate(need);
                all.get(need..).map(<[u32]>::to_vec).unwrap_or_default()
            };
            for &c in &freed {
                self.mark(c, false)?;
                if !f.no_fat_chain() {
                    self.fat_set(c, 0)?;
                }
            }
            if !f.no_fat_chain()
                && let Some(&l) = cl.last()
            {
                self.fat_set(l, EOC)?;
            }
            if cl.is_empty() {
                f.first_cluster = 0;
                f.flags &= !sflag::NO_FAT_CHAIN;
            }
        }
        Ok(cl)
    }

    /// Uvolni vsechny clustery souboru (smazani).
    pub(super) fn free_all(&mut self, f: &FileSet) -> Result<()> {
        let mut f = f.clone();
        self.resize(&mut f, 0).map(|_| ())
    }

    /// Prodlouzi retez FAT (koren) o jeden cluster; vraci novy cluster.
    pub(super) fn extend_fat_chain(&mut self, cl: &mut Vec<u32>) -> Result<u32> {
        let last = *cl.last().ok_or(Error::Corrupted)?;
        let c = self.alloc(1, Some(last))?[0];
        self.fat_set(last, c)?;
        self.fat_set(c, EOC)?;
        cl.push(c);
        Ok(c)
    }

    /// Useky (pozice na mediu, delka) pro bajty `off..off+len` clusteru.
    fn spans(&self, cl: &[u32], off: u64, len: usize) -> Result<Vec<(u64, usize)>> {
        let cs = self.g.cluster_size();
        let mut out: Vec<(u64, usize)> = Vec::new();
        let mut p = off;
        let end = off + len as u64;
        while p < end {
            let c = *cl.get((p / cs) as usize).ok_or(Error::Corrupted)?;
            let within = p % cs;
            let n = (cs - within).min(end - p) as usize;
            let pos = self.g.cluster_pos(c) + within;
            match out.last_mut() {
                Some((lp, ln)) if *lp + *ln as u64 == pos => *ln += n,
                _ => out.push((pos, n)),
            }
            p += n as u64;
        }
        Ok(out)
    }

    pub(super) fn read_clusters(&mut self, cl: &[u32], off: u64, buf: &mut [u8]) -> Result<()> {
        let mut done = 0;
        for (pos, n) in self.spans(cl, off, buf.len())? {
            read_at(&mut self.dev, pos, &mut buf[done..done + n])?;
            done += n;
        }
        Ok(())
    }

    pub(super) fn write_clusters(&mut self, cl: &[u32], off: u64, data: &[u8]) -> Result<()> {
        let mut done = 0;
        for (pos, n) in self.spans(cl, off, data.len())? {
            write_at(&mut self.dev, pos, &data[done..done + n])?;
            done += n;
        }
        Ok(())
    }

    pub(super) fn zero_clusters(&mut self, cl: &[u32]) -> Result<()> {
        let zeros = vec![0u8; self.g.cluster_size() as usize];
        for &c in cl {
            let pos = self.g.cluster_pos(c);
            write_at(&mut self.dev, pos, &zeros)?;
        }
        Ok(())
    }
}
