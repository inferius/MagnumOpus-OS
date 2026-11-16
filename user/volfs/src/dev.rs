//! Zarizeni po sektorech jako `Read + Write + Seek` pro crates FS (fatfs,
//! ntfs), s vyrovnavaci pameti sektoru.
//!
//! Zapis jde do pameti (spinave sektory) a na medium az pri [`Cached::flush`]
//! - proces media ho vola na konci kazde menici operace, takze po odpovedi
//! fsd jsou data na mediu (vytazeni flashky po dokonceni kopie je bezpecne,
//! behem ne). Cteni po celych sektorech, sousedni chybejici sektory jednim
//! pozadavkem.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;
use std::io::{self, Read, Seek, SeekFrom, Write};

/// Medium po sektorech (implementuje kanal `blockproto`, v testech pamet).
pub trait Sectors {
    fn sector_size(&self) -> usize;
    fn sectors(&self) -> u64;
    /// Precte `buf.len() / sector_size` sektoru od `lba`.
    fn read(&mut self, lba: u64, buf: &mut [u8]) -> io::Result<()>;
    /// Zapise `data.len() / sector_size` sektoru od `lba`.
    fn write(&mut self, lba: u64, data: &[u8]) -> io::Result<()>;
    /// Vyprazdni vyrovnavaci pamet zarizeni.
    fn flush(&mut self) -> io::Result<()>;
}

/// Nejvic sektoru v pameti (ciste se pak zahodi od nejstarsiho).
const CACHE_SECTORS: usize = 4096;
/// Nejvic sektoru jednoho cteni z media.
const READ_AHEAD: u64 = 64;

struct Slot {
    data: Vec<u8>,
    dirty: bool,
    used: u64,
}

/// Spolecny stav: zarizeni a vyrovnavaci pamet sektoru.
struct Core<D: Sectors> {
    dev: D,
    cache: BTreeMap<u64, Slot>,
    clock: u64,
    read_only: bool,
}

/// Pohled na zarizeni s vlastni pozici. Klon sdili zarizeni i pamet (crate
/// FS si jeden nechava uvnitr, proces media druhy pro [`Cached::flush_dirty`]).
pub struct Cached<D: Sectors> {
    core: Rc<RefCell<Core<D>>>,
    pos: u64,
}

impl<D: Sectors> Clone for Cached<D> {
    fn clone(&self) -> Self {
        Self { core: self.core.clone(), pos: 0 }
    }
}

impl<D: Sectors> Core<D> {
    fn len(&self) -> u64 {
        self.dev.sectors() * self.dev.sector_size() as u64
    }

    /// Sektor `lba` v pameti (precte ho i nekolik dalsich naraz).
    fn load(&mut self, lba: u64) -> io::Result<&mut Slot> {
        self.clock += 1;
        if !self.cache.contains_key(&lba) {
            let ss = self.dev.sector_size();
            let mut n = 1;
            while n < READ_AHEAD && lba + n < self.dev.sectors() && !self.cache.contains_key(&(lba + n)) {
                n += 1;
            }
            let mut buf = vec![0u8; n as usize * ss];
            self.dev.read(lba, &mut buf)?;
            for (i, chunk) in buf.chunks(ss).enumerate() {
                self.cache.insert(lba + i as u64, Slot { data: chunk.to_vec(), dirty: false, used: self.clock });
            }
            self.evict();
        }
        let clock = self.clock;
        let s = self.cache.get_mut(&lba).ok_or_else(|| io::Error::other("cache"))?;
        s.used = clock;
        Ok(s)
    }

    /// Zahodi nejstarsi ciste sektory nad limitem.
    fn evict(&mut self) {
        if self.cache.len() <= CACHE_SECTORS {
            return;
        }
        let mut clean: Vec<(u64, u64)> = self.cache.iter().filter(|(_, s)| !s.dirty).map(|(&l, s)| (s.used, l)).collect();
        clean.sort_unstable();
        let drop = self.cache.len() - CACHE_SECTORS / 2;
        for (_, l) in clean.into_iter().take(drop) {
            self.cache.remove(&l);
        }
    }

    fn flush_dirty(&mut self) -> io::Result<()> {
        let dirty: Vec<u64> = self.cache.iter().filter(|(_, s)| s.dirty).map(|(&l, _)| l).collect();
        let mut i = 0;
        while i < dirty.len() {
            let mut j = i + 1;
            while j < dirty.len() && dirty[j] == dirty[j - 1] + 1 && j - i < 256 {
                j += 1;
            }
            let mut buf = Vec::new();
            for l in &dirty[i..j] {
                buf.extend_from_slice(&self.cache[l].data);
            }
            self.dev.write(dirty[i], &buf)?;
            for l in &dirty[i..j] {
                if let Some(s) = self.cache.get_mut(l) {
                    s.dirty = false;
                }
            }
            i = j;
        }
        self.dev.flush()
    }
}

impl<D: Sectors> Cached<D> {
    pub fn new(dev: D, read_only: bool) -> Self {
        Self { core: Rc::new(RefCell::new(Core { dev, cache: BTreeMap::new(), clock: 0, read_only })), pos: 0 }
    }

    pub fn len(&self) -> u64 {
        self.core.borrow().len()
    }

    /// Spinave sektory na medium (souvisle useky jednim zapisem).
    pub fn flush_dirty(&self) -> io::Result<()> {
        self.core.borrow_mut().flush_dirty()
    }

    /// Zahodi neulozene zmeny (chyba uprostred operace).
    pub fn discard_dirty(&self) {
        self.core.borrow_mut().cache.retain(|_, s| !s.dirty);
    }

    /// Prace se zarizenim (testy: obsah obrazu v pameti).
    pub fn with_dev<R>(&self, f: impl FnOnce(&mut D) -> R) -> R {
        f(&mut self.core.borrow_mut().dev)
    }
}

impl<D: Sectors> Read for Cached<D> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let mut c = self.core.borrow_mut();
        let ss = c.dev.sector_size() as u64;
        let len = c.len();
        if self.pos >= len || buf.is_empty() {
            return Ok(0);
        }
        let lba = self.pos / ss;
        let off = (self.pos % ss) as usize;
        let n = buf.len().min(ss as usize - off).min((len - self.pos) as usize);
        let s = c.load(lba)?;
        buf[..n].copy_from_slice(&s.data[off..off + n]);
        self.pos += n as u64;
        Ok(n)
    }
}

impl<D: Sectors> Write for Cached<D> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let mut c = self.core.borrow_mut();
        if c.read_only {
            return Err(io::Error::new(io::ErrorKind::PermissionDenied, "medium jen pro cteni"));
        }
        let ss = c.dev.sector_size() as u64;
        let len = c.len();
        if self.pos >= len {
            return Err(io::Error::new(io::ErrorKind::WriteZero, "za koncem media"));
        }
        let lba = self.pos / ss;
        let off = (self.pos % ss) as usize;
        let n = buf.len().min(ss as usize - off).min((len - self.pos) as usize);
        let s = c.load(lba)?;
        s.data[off..off + n].copy_from_slice(&buf[..n]);
        s.dirty = true;
        self.pos += n as u64;
        Ok(n)
    }

    /// Zapis na medium az na konci operace ([`Cached::flush_dirty`]); crates
    /// FS volaji `flush` casto a uprostred operace.
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl<D: Sectors> Seek for Cached<D> {
    fn seek(&mut self, to: SeekFrom) -> io::Result<u64> {
        let len = self.len() as i128;
        let new = match to {
            SeekFrom::Start(p) => p as i128,
            SeekFrom::End(d) => len + d as i128,
            SeekFrom::Current(d) => self.pos as i128 + d as i128,
        };
        if new < 0 {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "pred zacatkem"));
        }
        self.pos = new as u64;
        Ok(self.pos)
    }
}

/// Medium v pameti (testy na hostiteli, obrazy v xtask).
pub struct MemDisk {
    pub data: Vec<u8>,
    pub ss: usize,
    pub writes: u64,
}

impl Sectors for MemDisk {
    fn sector_size(&self) -> usize {
        self.ss
    }
    fn sectors(&self) -> u64 {
        (self.data.len() / self.ss) as u64
    }
    fn read(&mut self, lba: u64, buf: &mut [u8]) -> io::Result<()> {
        let s = lba as usize * self.ss;
        let src = self.data.get(s..s + buf.len()).ok_or_else(|| io::Error::new(io::ErrorKind::UnexpectedEof, "mimo disk"))?;
        buf.copy_from_slice(src);
        Ok(())
    }
    fn write(&mut self, lba: u64, data: &[u8]) -> io::Result<()> {
        let s = lba as usize * self.ss;
        let dst = self.data.get_mut(s..s + data.len()).ok_or_else(|| io::Error::new(io::ErrorKind::UnexpectedEof, "mimo disk"))?;
        dst.copy_from_slice(data);
        self.writes += 1;
        Ok(())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
