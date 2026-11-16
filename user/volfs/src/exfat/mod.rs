//! exFAT: vlastni implementace, cteni i zapis (docs/cs/media.md).
//!
//! Crates exFAT v Rustu nebyly zrale (bez zkraceni souboru, cteni za
//! `ValidDataLength` bez nul), proto vlastni kod ve stejnem izolovanem
//! procesu media jako FAT a NTFS. Specifikace je verejna (Microsoft,
//! 2019); k patentum viz `THIRD-PARTY.md`.
//!
//! Rozsah: jedna FAT (TexFAT se dvema FAT jen pro cteni), sady polozek
//! s kontrolnim souctem, tabulka velkych pismen z media, bitmapa alokace
//! v pameti (zmeny hned do sektoru), soubory s retezem FAT i souvisle
//! (`NoFatChain`; nove alokace souvisle, jinak prevod na retez). Pri
//! prvni zmene se nastavi priznak "spinavy" svazek, bezpecne odpojeni ho
//! smaze. Bajty mezi `ValidDataLength` a `DataLength` se ctou jako nuly
//! (zvetseni souboru je proto levne).
//!
//! Vse z media je nedoveryhodne: poskozena sada polozek se v seznamu
//! vynecha (misto zustane obsazene), poskozeny retez = chyba operace,
//! poskozena spousteci oblast = zalozni oblast jen pro cteni, nebo
//! svazek nejde pripojit.

mod alloc;
mod boot;
mod dir;
mod entry;
mod upcase;

use std::io::{Read, Seek, SeekFrom, Write};

use volproto::{Error, Info, Result, Stat, VERSION, kind};

use crate::dev::{Cached, Sectors};
use crate::{Volume, io_err};
use boot::Geometry;
use entry::{FileSet, attr, sflag};
use upcase::Upcase;

use dir::DirRef;

/// Bitmapa alokace v pameti a clustery, kde lezi na mediu.
struct Bitmap {
    bits: Vec<u8>,
    clusters: Vec<u32>,
    free: u32,
    /// Odkud hledat volne clustery.
    hint: u32,
}

pub struct ExFatVolume<D: Sectors> {
    dev: Cached<D>,
    g: Geometry,
    read_only: bool,
    up: Upcase,
    bitmap: Bitmap,
    label: String,
    /// Priznak "spinavy" je na mediu (od prvni zmeny do odpojeni).
    dirty: bool,
    /// Posledni retez souboru (prvni cluster, priznaky, delka) -> clustery.
    chain_cache: Option<((u32, bool, u64), Vec<u32>)>,
}

/// Cesta bez uvodniho `/` rozdelena na slozky.
fn parts(path: &str) -> impl Iterator<Item = &str> {
    path.split('/').filter(|p| !p.is_empty())
}

/// (rodic, jmeno) normalizovane cesty; koren = `None`.
fn split(path: &str) -> Option<(&str, &str)> {
    let p = path.trim_end_matches('/');
    let i = p.rfind('/')?;
    let name = &p[i + 1..];
    if name.is_empty() { None } else { Some((&p[..i], name)) }
}

fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64)
}

impl<D: Sectors> ExFatVolume<D> {
    pub fn new(dev: D, read_only: bool) -> Result<Self> {
        let mut dev = Cached::new(dev, read_only);
        let dev_len = dev.len();
        let mut first = vec![0u8; 512];
        read_at(&mut dev, 0, &mut first)?;
        let (g, from_backup) = match Self::boot_region(&mut dev, 0, &first, dev_len) {
            Ok(g) => (g, false),
            Err(e) => {
                // Zalozni oblast (sektory 12..23) se stejnou velikosti sektoru.
                let ss = boot::sector_shift(&first).unwrap_or(9);
                let mut b = vec![0u8; 512];
                read_at(&mut dev, 12 << ss, &mut b)?;
                match Self::boot_region(&mut dev, 12 << ss, &b, dev_len) {
                    Ok(g) => {
                        eprintln!("volfs: exFAT: poskozena spousteci oblast, zaloha jen pro cteni");
                        (g, true)
                    }
                    Err(_) => return Err(e),
                }
            }
        };
        let mut read_only = read_only || from_backup;
        if g.fats != 1 {
            eprintln!("volfs: exFAT se dvema FAT (TexFAT) jen pro cteni");
            read_only = true;
        }
        if g.flags & boot::flag::MEDIA_FAILURE != 0 {
            eprintln!("volfs: exFAT hlasi chybu media, jen pro cteni");
            read_only = true;
        }
        if g.flags & boot::flag::DIRTY != 0 {
            eprintln!("volfs: exFAT nebyl bezpecne odpojen (doporucena kontrola ve Windows)");
        }
        let mut v = Self {
            dev,
            g,
            read_only,
            up: Upcase::generated(),
            bitmap: Bitmap { bits: Vec::new(), clusters: Vec::new(), free: 0, hint: 2 },
            label: String::new(),
            dirty: false,
            chain_cache: None,
        };
        v.read_root_meta()?;
        if v.read_only {
            v.dev.set_read_only();
        }
        Ok(v)
    }

    /// Spousteci oblast od `pos` (12 sektoru).
    fn boot_region(dev: &mut Cached<D>, pos: u64, first: &[u8], dev_len: u64) -> Result<Geometry> {
        let ss = boot::sector_shift(first).ok_or(Error::Corrupted)?;
        let mut region = vec![0u8; 12 << ss];
        read_at(dev, pos, &mut region)?;
        boot::parse(&region, dev_len)
    }

    /// Polozky korene: bitmapa, tabulka velkych pismen, jmenovka.
    fn read_root_meta(&mut self) -> Result<()> {
        let root = self.root()?;
        let data = self.read_dir_data(&root.clusters)?;
        let mut bitmap = None;
        let mut upcase = None;
        for e in data.chunks_exact(entry::ENTRY) {
            match e[0] {
                entry::ty::END => break,
                entry::ty::BITMAP => {
                    // U dvou FAT patri ke kazde vlastni bitmapa (bit 0).
                    if (e[1] & 1) == self.g.active_fat && bitmap.is_none() {
                        bitmap = Some((u32le(e, 20), u64le(e, 24)));
                    }
                }
                entry::ty::UPCASE => upcase = Some((u32le(e, 4), u32le(e, 20), u64le(e, 24))),
                entry::ty::LABEL => {
                    let n = (e[1] as usize).min(11);
                    let u: Vec<u16> = (0..n).map(|i| u16::from_le_bytes([e[2 + i * 2], e[3 + i * 2]])).collect();
                    self.label = String::from_utf16_lossy(&u);
                }
                _ => {}
            }
        }
        let (first, len) = bitmap.ok_or(Error::Corrupted)?;
        let need = (self.g.cluster_count as u64).div_ceil(8);
        if len < need || len > need + self.g.cluster_size() {
            return Err(Error::Corrupted);
        }
        let clusters = self.chain(first, false, len)?;
        let mut bits = vec![0u8; need as usize];
        self.read_clusters(&clusters, 0, &mut bits)?;
        let used: u32 = (0..self.g.cluster_count).filter(|&i| bits[i as usize / 8] & (1 << (i % 8)) != 0).count() as u32;
        self.bitmap = Bitmap { bits, clusters, free: self.g.cluster_count - used, hint: 2 };
        match upcase {
            Some((sum, first, len)) if len > 0 && len <= upcase::MAX_BYTES && len % 2 == 0 => {
                let clusters = self.chain(first, false, len)?;
                let mut raw = vec![0u8; len as usize];
                self.read_clusters(&clusters, 0, &mut raw)?;
                match Upcase::parse(&raw, sum) {
                    Some(t) => self.up = t,
                    None => {
                        eprintln!("volfs: exFAT: poskozena tabulka velkych pismen, jen pro cteni");
                        self.read_only = true;
                    }
                }
            }
            _ => {
                eprintln!("volfs: exFAT bez tabulky velkych pismen, jen pro cteni");
                self.read_only = true;
            }
        }
        Ok(())
    }

    fn writable(&self) -> Result<()> {
        if self.read_only { Err(Error::ReadOnly) } else { Ok(()) }
    }

    /// Pred prvni zmenou: priznak "spinavy" na medium (pred ostatnimi zapisy).
    fn begin(&mut self) -> Result<()> {
        self.writable()?;
        if !self.dirty {
            self.g.flags |= boot::flag::DIRTY;
            write_at(&mut self.dev, boot::VOLUME_FLAGS, &self.g.flags.to_le_bytes())?;
            self.dev.flush_dirty().map_err(io_err)?;
            self.dirty = true;
        }
        Ok(())
    }

    /// Po menici operaci: na medium; po chybe zahodit rozpracovane zmeny.
    fn commit<T>(&mut self, r: Result<T>) -> Result<T> {
        match r {
            Ok(v) => {
                self.dev.flush_dirty().map_err(io_err)?;
                Ok(v)
            }
            Err(e) => {
                self.dev.discard_dirty();
                self.chain_cache = None;
                // Bitmapa v pameti mohla predbehnout medium: znovu z media.
                let _ = self.reload_bitmap();
                Err(e)
            }
        }
    }

    fn stat_of(&self, f: &FileSet) -> Stat {
        Stat {
            kind: if f.is_dir() { kind::DIR } else { kind::FILE },
            size: if f.is_dir() { 0 } else { f.len },
            mtime: entry::from_stamp(f.mtime),
            btime: entry::from_stamp(f.ctime),
            read_only: self.read_only || f.attrs & attr::READ_ONLY != 0,
        }
    }

    fn write_inner(&mut self, path: &str, off: u64, data: &[u8]) -> Result<usize> {
        let mut node = self.lookup(path)?;
        if node.set.is_dir() {
            return Err(Error::IsADirectory);
        }
        if node.set.extra {
            return Err(Error::NotSupported);
        }
        let end = off.checked_add(data.len() as u64).ok_or(Error::InvalidArgs)?;
        let new_len = end.max(node.set.len);
        let clusters = self.resize(&mut node.set, new_len)?;
        // Mezi platnymi daty a zacatkem zapisu nuly (to, co se cetlo jako nuly).
        if off > node.set.valid_len {
            let zeros = vec![0u8; 64 * 1024];
            let mut p = node.set.valid_len;
            while p < off {
                let n = (off - p).min(zeros.len() as u64) as usize;
                self.write_clusters(&clusters, p, &zeros[..n])?;
                p += n as u64;
            }
        }
        self.write_clusters(&clusters, off, data)?;
        node.set.len = node.set.len.max(end);
        node.set.valid_len = node.set.valid_len.max(end);
        node.set.mtime = entry::to_stamp(now());
        node.set.attrs |= attr::ARCHIVE;
        self.write_set(&node)?;
        Ok(data.len())
    }

    fn set_len_inner(&mut self, path: &str, len: u64) -> Result<()> {
        let mut node = self.lookup(path)?;
        if node.set.is_dir() {
            return Err(Error::IsADirectory);
        }
        if node.set.extra {
            return Err(Error::NotSupported);
        }
        if len == node.set.len {
            return Ok(());
        }
        self.resize(&mut node.set, len)?;
        node.set.len = len;
        node.set.valid_len = node.set.valid_len.min(len);
        node.set.mtime = entry::to_stamp(now());
        node.set.attrs |= attr::ARCHIVE;
        self.write_set(&node)
    }

    fn create_inner(&mut self, path: &str, k: u8, exclusive: bool) -> Result<()> {
        let (parent, name) = split(path).ok_or(Error::AlreadyExists)?;
        let mut dir = self.open_dir(parent)?;
        if let Some(n) = self.find(&dir, name)? {
            return if exclusive || (k == kind::DIR) != n.set.is_dir() { Err(Error::AlreadyExists) } else { Ok(()) };
        }
        let name = entry::check_name(name)?;
        let t = entry::to_stamp(now());
        let mut set = FileSet {
            attrs: if k == kind::DIR { attr::DIR } else { attr::ARCHIVE },
            ctime: t,
            mtime: t,
            atime: t,
            flags: sflag::ALLOC_POSSIBLE,
            name,
            valid_len: 0,
            first_cluster: 0,
            len: 0,
            extra: false,
        };
        if k == kind::DIR {
            // Slozka ma aspon jeden cluster (nuly = konec polozek).
            let cs = self.g.cluster_size();
            let clusters = self.resize(&mut set, cs)?;
            self.zero_clusters(&clusters)?;
            set.len = cs;
            set.valid_len = cs;
        }
        self.insert_set(&mut dir, &set)?;
        Ok(())
    }

    fn remove_inner(&mut self, path: &str) -> Result<()> {
        let node = self.lookup(path)?;
        if node.set.extra {
            return Err(Error::NotSupported);
        }
        if node.set.is_dir() {
            let d = DirRef { clusters: self.file_chain(&node.set)?, node: Some(node.clone()) };
            if self.any_entry(&d)? {
                return Err(Error::NotEmpty);
            }
        }
        self.mark_unused(&node)?;
        self.free_all(&node.set)?;
        Ok(())
    }

    fn rename_inner(&mut self, from: &str, to: &str, replace: bool) -> Result<()> {
        let src = self.lookup(from)?;
        if src.set.extra {
            return Err(Error::NotSupported);
        }
        let (tparent, tname) = split(to).ok_or(Error::InvalidArgs)?;
        // Slozka do sve vlastni podslozky.
        let fl = from.to_lowercase();
        if src.set.is_dir() && to.to_lowercase().starts_with(&format!("{fl}/")) {
            return Err(Error::InvalidArgs);
        }
        let name = entry::check_name(tname)?;
        let mut dir = self.open_dir(tparent)?;
        if let Some(dst) = self.find(&dir, tname)? {
            let same = dst.slot == src.slot && dst.dir_first == src.dir_first;
            if !same {
                if !replace {
                    return Err(Error::AlreadyExists);
                }
                if dst.set.is_dir() {
                    return Err(Error::IsADirectory);
                }
                if dst.set.extra {
                    return Err(Error::NotSupported);
                }
                self.mark_unused(&dst)?;
                self.free_all(&dst.set)?;
            }
        }
        let mut set = src.set.clone();
        set.name = name;
        let same_dir = dir.clusters.first() == src.dir_clusters.first();
        if same_dir && set.entries() <= src.set.entries() {
            // Na miste: nova sada v puvodnich slotech, zbytek uvolnit.
            let mut n = src.clone();
            n.set = set;
            self.write_set(&n)?;
            self.mark_slots_unused(&src.dir_clusters, src.slot + n.set.entries(), src.set.entries() - n.set.entries())?;
        } else {
            // Nejdriv nova sada (pad = dvojite jmeno, ne ztraceny soubor).
            self.insert_set(&mut dir, &set)?;
            // Rozsireni slozky nemeni pozice existujicich slotu.
            self.mark_unused(&src)?;
        }
        Ok(())
    }

    /// Bitmapa znovu z media (po chybe uprostred operace).
    fn reload_bitmap(&mut self) -> Result<()> {
        let clusters = self.bitmap.clusters.clone();
        let mut bits = vec![0u8; self.bitmap.bits.len()];
        self.read_clusters(&clusters, 0, &mut bits)?;
        let used: u32 = (0..self.g.cluster_count).filter(|&i| bits[i as usize / 8] & (1 << (i % 8)) != 0).count() as u32;
        self.bitmap.bits = bits;
        self.bitmap.free = self.g.cluster_count - used;
        Ok(())
    }
}

impl<D: Sectors> Volume for ExFatVolume<D> {
    fn info(&mut self) -> Result<Info> {
        let cs = self.g.cluster_size();
        Ok(Info {
            version: VERSION,
            fs: "exfat".into(),
            label: self.label.trim().to_string(),
            total_bytes: self.g.cluster_count as u64 * cs,
            free_bytes: self.bitmap.free as u64 * cs,
            read_only: self.read_only,
        })
    }

    fn stat(&mut self, path: &str) -> Result<Stat> {
        if split(path).is_none() {
            return Ok(Stat { kind: kind::DIR, read_only: self.read_only, ..Stat::default() });
        }
        let n = self.lookup(path)?;
        Ok(self.stat_of(&n.set))
    }

    fn list(&mut self, path: &str) -> Result<Vec<(String, Stat)>> {
        let dir = self.open_dir(path)?;
        let mut out = Vec::new();
        for n in self.entries(&dir)? {
            let name = String::from_utf16_lossy(&n.set.name);
            out.push((name, self.stat_of(&n.set)));
        }
        Ok(out)
    }

    fn read(&mut self, path: &str, off: u64, len: usize) -> Result<Vec<u8>> {
        let n = self.lookup(path)?;
        if n.set.is_dir() {
            return Err(Error::IsADirectory);
        }
        if off >= n.set.len {
            return Ok(Vec::new());
        }
        let want = (len as u64).min(n.set.len - off) as usize;
        let mut buf = vec![0u8; want];
        // Do `ValidDataLength` z media, dal nuly.
        let valid = n.set.valid_len.saturating_sub(off).min(want as u64) as usize;
        if valid > 0 {
            let clusters = self.file_chain(&n.set)?;
            self.read_clusters(&clusters, off, &mut buf[..valid])?;
        }
        Ok(buf)
    }

    fn write(&mut self, path: &str, off: u64, data: &[u8]) -> Result<usize> {
        self.begin()?;
        let r = self.write_inner(path, off, data);
        self.commit(r)
    }

    fn create(&mut self, path: &str, k: u8, exclusive: bool) -> Result<()> {
        self.begin()?;
        let r = self.create_inner(path, k, exclusive);
        self.commit(r)
    }

    fn remove(&mut self, path: &str) -> Result<()> {
        if split(path).is_none() {
            return Err(Error::AccessDenied);
        }
        self.begin()?;
        let r = self.remove_inner(path);
        self.commit(r)
    }

    fn rename(&mut self, from: &str, to: &str, replace: bool) -> Result<()> {
        if split(from).is_none() || split(to).is_none() {
            return Err(Error::InvalidArgs);
        }
        self.begin()?;
        let r = self.rename_inner(from, to, replace);
        self.commit(r)
    }

    fn set_len(&mut self, path: &str, len: u64) -> Result<()> {
        self.begin()?;
        let r = self.set_len_inner(path, len);
        self.commit(r)
    }

    fn sync(&mut self) -> Result<()> {
        self.dev.flush_dirty().map_err(io_err)
    }

    fn unmount(&mut self) -> Result<()> {
        if self.dirty {
            // Procento vyuziti a cisty svazek (oboje mimo kontrolni soucet).
            let used = self.g.cluster_count - self.bitmap.free;
            let pct = (used as u64 * 100 / self.g.cluster_count as u64) as u8;
            write_at(&mut self.dev, boot::PERCENT_IN_USE, &[pct])?;
            self.g.flags &= !boot::flag::DIRTY;
            self.dev.flush_dirty().map_err(io_err)?;
            write_at(&mut self.dev, boot::VOLUME_FLAGS, &self.g.flags.to_le_bytes())?;
            self.dirty = false;
        }
        self.dev.flush_dirty().map_err(io_err)
    }
}

impl<D: Sectors> ExFatVolume<D> {
    /// Medium (testy: obsah obrazu po operacich).
    pub fn device(&self) -> &Cached<D> {
        &self.dev
    }
}

#[cfg(test)]
mod tests;

fn u32le(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}

fn u64le(b: &[u8], o: usize) -> u64 {
    let mut a = [0u8; 8];
    a.copy_from_slice(&b[o..o + 8]);
    u64::from_le_bytes(a)
}

fn read_at<D: Sectors>(dev: &mut Cached<D>, pos: u64, buf: &mut [u8]) -> Result<()> {
    dev.seek(SeekFrom::Start(pos)).map_err(io_err)?;
    dev.read_exact(buf).map_err(io_err)
}

fn write_at<D: Sectors>(dev: &mut Cached<D>, pos: u64, data: &[u8]) -> Result<()> {
    dev.seek(SeekFrom::Start(pos)).map_err(io_err)?;
    dev.write_all(data).map_err(io_err)
}
