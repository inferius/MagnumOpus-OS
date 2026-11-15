//! Minimalni a bezpecny zapis do FAT32: vytvoreni a prepsani souboru
//! (logy a zaznamy padu na flashce, ktere Windows precte bez dalsich
//! nastroju; ESP pri instalaci), cteni souboru, formatovani (`format`).
//!
//! Co umi a co ne:
//! - jen FAT32 se sektorem 512 B (oddil `MO-LOGY` vytvari xtask),
//! - jmena 8.3 velkymi pismeny (bez dlouhych jmen),
//! - slozky (i vnorene, `ensure_dir`): najde je, nebo vytvori,
//! - data cte a zapisuje po souvislych bezich clusteru a FAT po celych
//!   sektorech (60 MiB initfs pri instalaci = stovky operaci, ne desitky
//!   tisic),
//! - soubor v ni: vytvori, nebo prepise (stary retez clusteru uvolni),
//! - nikdy nemaze ani nemeni nic mimo svou slozku a soubory, ktere zapisuje;
//!   cizi soubory zustanou netknute,
//! - poradi zapisu: data, FAT (vsechny kopie), polozka adresare - preruseny
//!   zapis nanejvys ztrati misto (clustery bez souboru, opravi chkdsk), ale
//!   neposkodi jiny soubor.
//!
//! Testy proti crate `fatfs` (cteni po zapisu): `cargo test -p fatw`.

#![cfg_attr(not(test), no_std)]

extern crate alloc;

use alloc::vec;
use alloc::vec::Vec;

/// Sektor (jiny FAT32 neumime).
pub const SECTOR: usize = 512;

/// Chyby.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    /// Zarizeni vratilo chybu.
    Io,
    /// Neni to FAT32 se sektorem 512 B.
    NotFat32,
    /// Neplatne jmeno (8.3).
    BadName,
    /// Malo mista.
    Full,
    /// Poskozena struktura (retez mimo svazek, smycka).
    Corrupted,
}

pub type Result<T> = core::result::Result<T, Error>;

/// Blokove zarizeni po sektorech 512 B (oddil, sektor 0 = zacatek oddilu).
pub trait Device {
    fn read(&mut self, lba: u64, buf: &mut [u8]) -> Result<()>;
    fn write(&mut self, lba: u64, data: &[u8]) -> Result<()>;
    fn flush(&mut self) -> Result<()>;
}

const EOC: u32 = 0x0FFF_FFFF;
const ATTR_DIR: u8 = 0x10;
const ATTR_ARCHIVE: u8 = 0x20;
const ATTR_VOLUME: u8 = 0x08;
const ATTR_LFN: u8 = 0x0F;

/// Datum a cas FAT (mistni cas; 0 = 1980-01-01).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Stamp {
    pub date: u16,
    pub time: u16,
}

impl Stamp {
    pub fn new(year: u16, month: u8, day: u8, hour: u8, min: u8, sec: u8) -> Self {
        Stamp {
            date: ((year.saturating_sub(1980)) << 9) | ((month as u16) << 5) | day as u16,
            time: ((hour as u16) << 11) | ((min as u16) << 5) | (sec as u16 / 2),
        }
    }
}

pub struct Volume<D: Device> {
    dev: D,
    spc: u32,
    fat_start: u64,
    fat_size: u64,
    fats: u32,
    data_start: u64,
    clusters: u32,
    root: u32,
    fsinfo: u64,
    /// Kde hledat volny cluster.
    hint: u32,
    /// Posledni prectany sektor prvni FAT (cislo sektoru v FAT, obsah):
    /// cteni retezu a hledani volnych clusteru pak nectou sektor FAT pro
    /// kazdy cluster zvlast (128 polozek na sektor).
    fat_cache: Option<(u64, [u8; SECTOR])>,
}

/// Nejvic bajtu v jednom cteni nebo zapisu souvisleho behu clusteru (1 MiB:
/// blokovy klient posle cteni najednou a ovladac USB je spoji).
const RUN_BYTES: usize = 1024 * 1024;

/// Jmeno 8.3 do 11 bajtu adresare (`LOG.TXT` -> `LOG     TXT`).
pub fn short_name(name: &str) -> Result<[u8; 11]> {
    let (base, ext) = name.split_once('.').unwrap_or((name, ""));
    if base.is_empty() || base.len() > 8 || ext.len() > 3 {
        return Err(Error::BadName);
    }
    let mut out = [b' '; 11];
    for (i, c) in base.bytes().enumerate() {
        out[i] = check_char(c)?;
    }
    for (i, c) in ext.bytes().enumerate() {
        out[8 + i] = check_char(c)?;
    }
    Ok(out)
}

fn check_char(c: u8) -> Result<u8> {
    let c = c.to_ascii_uppercase();
    if c.is_ascii_uppercase() || c.is_ascii_digit() || b"_-~!#$%&".contains(&c) {
        Ok(c)
    } else {
        Err(Error::BadName)
    }
}

fn u16_at(b: &[u8], o: usize) -> u16 {
    u16::from_le_bytes([b[o], b[o + 1]])
}

fn u32_at(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}

impl<D: Device> Volume<D> {
    /// Otevre svazek (sektor 0 oddilu = boot sektor FAT32).
    pub fn open(mut dev: D) -> Result<Self> {
        let mut bs = [0u8; SECTOR];
        dev.read(0, &mut bs)?;
        let bps = u16_at(&bs, 11) as usize;
        let spc = bs[13] as u32;
        let reserved = u16_at(&bs, 14) as u64;
        let fats = bs[16] as u32;
        let root_entries = u16_at(&bs, 17);
        let total16 = u16_at(&bs, 19) as u64;
        let fat16 = u16_at(&bs, 22);
        let total32 = u32_at(&bs, 32) as u64;
        let fat_size = u32_at(&bs, 36) as u64;
        let root = u32_at(&bs, 44);
        let fsinfo = u16_at(&bs, 48) as u64;
        if bps != SECTOR
            || u16_at(&bs, 510) != 0xAA55
            || spc == 0
            || !spc.is_power_of_two()
            || fats == 0
            || root_entries != 0
            || fat16 != 0
            || fat_size == 0
            || &bs[82..87] != b"FAT32"
        {
            return Err(Error::NotFat32);
        }
        let total = if total16 != 0 { total16 } else { total32 };
        let data_start = reserved + fats as u64 * fat_size;
        let clusters = ((total.saturating_sub(data_start)) / spc as u64) as u32;
        // FAT musi pojmout vsechny clustery (+2 rezervovane).
        if clusters < 65525 || (clusters as u64 + 2) * 4 > fat_size * SECTOR as u64 || root < 2 || root >= clusters + 2 {
            return Err(Error::NotFat32);
        }
        Ok(Self {
            dev,
            spc,
            fat_start: reserved,
            fat_size,
            fats,
            data_start,
            clusters,
            root,
            fsinfo,
            hint: 2,
            fat_cache: None,
        })
    }

    pub fn into_device(self) -> D {
        self.dev
    }

    fn cluster_bytes(&self) -> usize {
        self.spc as usize * SECTOR
    }

    fn cluster_lba(&self, c: u32) -> u64 {
        self.data_start + (c as u64 - 2) * self.spc as u64
    }

    fn fat_get(&mut self, c: u32) -> Result<u32> {
        let off = c as u64 * 4;
        let sec = off / SECTOR as u64;
        let o = (off % SECTOR as u64) as usize;
        if let Some((n, s)) = &self.fat_cache
            && *n == sec
        {
            return Ok(u32_at(s, o) & 0x0FFF_FFFF);
        }
        let mut s = [0u8; SECTOR];
        self.dev.read(self.fat_start + sec, &mut s)?;
        let v = u32_at(&s, o) & 0x0FFF_FFFF;
        self.fat_cache = Some((sec, s));
        Ok(v)
    }

    /// Zapise vic polozek FAT najednou: kazdy dotceny sektor se v kazde
    /// kopii FAT precte a zapise jednou (`items` serazene podle clusteru
    /// nejsou nutne, souvisle useky jsou ale nejrychlejsi).
    fn fat_set_many(&mut self, items: &[(u32, u32)]) -> Result<()> {
        let mut i = 0;
        while i < items.len() {
            let sec = items[i].0 as u64 * 4 / SECTOR as u64;
            let mut j = i;
            while j < items.len() && items[j].0 as u64 * 4 / SECTOR as u64 == sec {
                j += 1;
            }
            for f in 0..self.fats as u64 {
                let lba = self.fat_start + f * self.fat_size + sec;
                let mut s = [0u8; SECTOR];
                self.dev.read(lba, &mut s)?;
                for &(c, v) in &items[i..j] {
                    let o = (c as u64 * 4 % SECTOR as u64) as usize;
                    let new = (u32_at(&s, o) & 0xF000_0000) | (v & 0x0FFF_FFFF);
                    s[o..o + 4].copy_from_slice(&new.to_le_bytes());
                }
                self.dev.write(lba, &s)?;
                if f == 0 {
                    self.fat_cache = Some((sec, s));
                }
            }
            i = j;
        }
        Ok(())
    }

    /// Zapise polozku FAT do vsech kopii (horni 4 bity zachova).
    fn fat_set(&mut self, c: u32, v: u32) -> Result<()> {
        let off = c as u64 * 4;
        let o = (off % SECTOR as u64) as usize;
        for f in 0..self.fats as u64 {
            let lba = self.fat_start + f * self.fat_size + off / SECTOR as u64;
            let mut s = [0u8; SECTOR];
            self.dev.read(lba, &mut s)?;
            let old = u32_at(&s, o);
            let new = (old & 0xF000_0000) | (v & 0x0FFF_FFFF);
            s[o..o + 4].copy_from_slice(&new.to_le_bytes());
            self.dev.write(lba, &s)?;
            if f == 0 {
                self.fat_cache = Some((off / SECTOR as u64, s));
            }
        }
        Ok(())
    }

    fn valid(&self, c: u32) -> bool {
        (2..self.clusters + 2).contains(&c)
    }

    /// Retez clusteru od `first`.
    fn chain(&mut self, first: u32) -> Result<Vec<u32>> {
        let mut out = Vec::new();
        let mut c = first;
        while self.valid(c) {
            if out.len() > self.clusters as usize {
                return Err(Error::Corrupted);
            }
            out.push(c);
            c = self.fat_get(c)?;
        }
        if c < 0x0FFF_FFF8 && c != 0 {
            return Err(Error::Corrupted);
        }
        Ok(out)
    }

    /// Najde `n` volnych clusteru (od napovedy, pak od zacatku).
    fn find_free(&mut self, n: usize) -> Result<Vec<u32>> {
        let mut out = Vec::with_capacity(n);
        let total = self.clusters;
        let mut c = self.hint;
        for _ in 0..total {
            if out.len() == n {
                break;
            }
            if !self.valid(c) {
                c = 2;
            }
            if self.fat_get(c)? == 0 {
                out.push(c);
            }
            c += 1;
        }
        if out.len() < n {
            return Err(Error::Full);
        }
        self.hint = c;
        Ok(out)
    }

    /// Spoji clustery do retezce (posledni = konec).
    fn link(&mut self, cs: &[u32]) -> Result<()> {
        let items: Vec<(u32, u32)> =
            cs.iter().enumerate().map(|(i, &c)| (c, cs.get(i + 1).copied().unwrap_or(EOC))).collect();
        self.fat_set_many(&items)
    }

    fn free_chain(&mut self, cs: &[u32]) -> Result<()> {
        let items: Vec<(u32, u32)> = cs.iter().map(|&c| (c, 0)).collect();
        self.fat_set_many(&items)
    }

    /// Souvisle behy clusteru (prvni cluster, pocet), kazdy nejvys
    /// [`RUN_BYTES`].
    fn runs(&self, cs: &[u32]) -> Vec<(u32, usize)> {
        let max = (RUN_BYTES / self.cluster_bytes()).max(1);
        let mut out: Vec<(u32, usize)> = Vec::new();
        for &c in cs {
            match out.last_mut() {
                Some((first, n)) if *first + *n as u32 == c && *n < max => *n += 1,
                _ => out.push((c, 1)),
            }
        }
        out
    }

    fn read_cluster(&mut self, c: u32) -> Result<Vec<u8>> {
        let mut b = vec![0u8; self.cluster_bytes()];
        self.dev.read(self.cluster_lba(c), &mut b)?;
        Ok(b)
    }

    fn write_cluster(&mut self, c: u32, data: &[u8]) -> Result<()> {
        let mut b = vec![0u8; self.cluster_bytes()];
        b[..data.len()].copy_from_slice(data);
        self.dev.write(self.cluster_lba(c), &b)
    }

    /// Najde polozku `name` v adresari od clusteru `dir`: (cluster,
    /// index polozky, polozka). `None` = neni.
    fn find_entry(&mut self, dir: u32, name: &[u8; 11]) -> Result<Option<(u32, usize, [u8; 32])>> {
        for c in self.chain(dir)? {
            let b = self.read_cluster(c)?;
            for (i, e) in b.chunks(32).enumerate() {
                if e[0] == 0 {
                    return Ok(None);
                }
                if e[0] == 0xE5 || e[11] == ATTR_LFN || e[11] & ATTR_VOLUME != 0 {
                    continue;
                }
                if &e[..11] == name {
                    return Ok(Some((c, i, e.try_into().unwrap())));
                }
            }
        }
        Ok(None)
    }

    /// Volne misto pro polozku v adresari (pripadne prodlouzi adresar).
    fn free_slot(&mut self, dir: u32) -> Result<(u32, usize)> {
        let chain = self.chain(dir)?;
        for &c in &chain {
            let b = self.read_cluster(c)?;
            if let Some(i) = b.chunks(32).position(|e| e[0] == 0 || e[0] == 0xE5) {
                return Ok((c, i));
            }
        }
        // Adresar je plny: dalsi cluster (vynulovany = konec adresare).
        let new = self.find_free(1)?[0];
        self.write_cluster(new, &[])?;
        self.fat_set(new, EOC)?;
        let last = *chain.last().ok_or(Error::Corrupted)?;
        self.fat_set(last, new)?;
        Ok((new, 0))
    }

    fn put_entry(&mut self, c: u32, i: usize, e: &[u8; 32]) -> Result<()> {
        let mut b = self.read_cluster(c)?;
        b[i * 32..i * 32 + 32].copy_from_slice(e);
        self.dev.write(self.cluster_lba(c), &b)
    }

    fn entry(name: &[u8; 11], attr: u8, first: u32, size: u32, t: Stamp) -> [u8; 32] {
        let mut e = [0u8; 32];
        e[..11].copy_from_slice(name);
        e[11] = attr;
        e[14..16].copy_from_slice(&t.time.to_le_bytes());
        e[16..18].copy_from_slice(&t.date.to_le_bytes());
        e[18..20].copy_from_slice(&t.date.to_le_bytes());
        e[20..22].copy_from_slice(&((first >> 16) as u16).to_le_bytes());
        e[22..24].copy_from_slice(&t.time.to_le_bytes());
        e[24..26].copy_from_slice(&t.date.to_le_bytes());
        e[26..28].copy_from_slice(&(first as u16).to_le_bytes());
        e[28..32].copy_from_slice(&size.to_le_bytes());
        e
    }

    fn first_cluster(e: &[u8]) -> u32 {
        (u16_at(e, 20) as u32) << 16 | u16_at(e, 26) as u32
    }

    /// Slozka `dir` (cesta od korene, napr. `EFI/BOOT`): najde ji, nebo
    /// vytvori (i nadrazene). Vraci jeji cluster.
    pub fn ensure_dir(&mut self, dir: &str, t: Stamp) -> Result<u32> {
        let mut parent = self.root;
        for part in dir.split('/').filter(|p| !p.is_empty()) {
            parent = self.ensure_child_dir(parent, part, t)?;
        }
        Ok(parent)
    }

    fn ensure_child_dir(&mut self, parent: u32, dir: &str, t: Stamp) -> Result<u32> {
        let name = short_name(dir)?;
        if let Some((_, _, e)) = self.find_entry(parent, &name)? {
            if e[11] & ATTR_DIR == 0 {
                return Err(Error::Corrupted);
            }
            return Ok(Self::first_cluster(&e));
        }
        let c = self.find_free(1)?[0];
        // Obsah: "." a ".." (rodic koren = 0).
        let up = if parent == self.root { 0 } else { parent };
        let mut b = vec![0u8; 64];
        b[..32].copy_from_slice(&Self::entry(b".          ", ATTR_DIR, c, 0, t));
        b[32..].copy_from_slice(&Self::entry(b"..         ", ATTR_DIR, up, 0, t));
        self.write_cluster(c, &b)?;
        self.fat_set(c, EOC)?;
        let (sc, si) = self.free_slot(parent)?;
        self.put_entry(sc, si, &Self::entry(&name, ATTR_DIR, c, 0, t))?;
        self.invalidate_fsinfo()?;
        self.dev.flush()?;
        Ok(c)
    }

    /// Precte soubor `path` (napr. `os/kernel.elf`); `None` = neni.
    pub fn read_file(&mut self, path: &str) -> Result<Option<Vec<u8>>> {
        let mut parts: Vec<&str> = path.split('/').filter(|p| !p.is_empty()).collect();
        let Some(file) = parts.pop() else { return Ok(None) };
        let mut dir = self.root;
        for p in parts {
            match self.find_entry(dir, &short_name(p)?)? {
                Some((_, _, e)) if e[11] & ATTR_DIR != 0 => dir = Self::first_cluster(&e),
                _ => return Ok(None),
            }
        }
        let Some((_, _, e)) = self.find_entry(dir, &short_name(file)?)? else { return Ok(None) };
        if e[11] & ATTR_DIR != 0 {
            return Ok(None);
        }
        let size = u32_at(&e, 28) as usize;
        let first = Self::first_cluster(&e);
        let mut out = Vec::with_capacity(size);
        if size > 0 {
            let chain = self.chain(first)?;
            for (c, n) in self.runs(&chain) {
                let mut b = vec![0u8; n * self.cluster_bytes()];
                self.dev.read(self.cluster_lba(c), &mut b)?;
                let take = (size - out.len()).min(b.len());
                out.extend_from_slice(&b[..take]);
                if out.len() == size {
                    break;
                }
            }
        }
        if out.len() != size {
            return Err(Error::Corrupted);
        }
        Ok(Some(out))
    }

    /// Vytvori nebo prepise soubor `name` ve slozce `dir` (cesta od korene).
    pub fn write_file(&mut self, dir: &str, name: &str, data: &[u8], t: Stamp) -> Result<()> {
        if data.len() > u32::MAX as usize {
            return Err(Error::Full);
        }
        let dir_c = self.ensure_dir(dir, t)?;
        let fname = short_name(name)?;
        let old = self.find_entry(dir_c, &fname)?;
        if let Some((_, _, e)) = &old
            && e[11] & ATTR_DIR != 0
        {
            return Err(Error::Corrupted);
        }
        // Nova data do novych clusteru (stara zustanou, dokud se neprepise
        // polozka - preruseni nenecha soubor s cizimi daty).
        let n = data.len().div_ceil(self.cluster_bytes());
        let cs = if n > 0 { self.find_free(n)? } else { Vec::new() };
        let mut at = 0;
        for (c, n) in self.runs(&cs) {
            let len = n * self.cluster_bytes();
            let end = (at + len).min(data.len());
            if end - at == len {
                self.dev.write(self.cluster_lba(c), &data[at..end])?;
            } else {
                // Posledni beh: doplnit nulami na cele clustery.
                let mut b = vec![0u8; len];
                b[..end - at].copy_from_slice(&data[at..end]);
                self.dev.write(self.cluster_lba(c), &b)?;
            }
            at = end;
        }
        self.link(&cs)?;
        let first = cs.first().copied().unwrap_or(0);
        let e = Self::entry(&fname, ATTR_ARCHIVE, first, data.len() as u32, t);
        match old {
            Some((c, i, olde)) => {
                self.put_entry(c, i, &e)?;
                let old_first = Self::first_cluster(&olde);
                if self.valid(old_first) {
                    let chain = self.chain(old_first)?;
                    self.free_chain(&chain)?;
                }
            }
            None => {
                let (c, i) = self.free_slot(dir_c)?;
                self.put_entry(c, i, &e)?;
            }
        }
        self.invalidate_fsinfo()?;
        self.dev.flush()
    }

    /// FSInfo: pocet volnych clusteru "neznamy" (Windows si ho spocita).
    fn invalidate_fsinfo(&mut self) -> Result<()> {
        if self.fsinfo == 0 || self.fsinfo == 0xFFFF {
            return Ok(());
        }
        let mut s = [0u8; SECTOR];
        self.dev.read(self.fsinfo, &mut s)?;
        if u32_at(&s, 0) != 0x4161_5252 || u32_at(&s, 484) != 0x6141_7272 {
            return Ok(());
        }
        s[488..492].copy_from_slice(&u32::MAX.to_le_bytes());
        s[492..496].copy_from_slice(&self.hint.to_le_bytes());
        self.dev.write(self.fsinfo, &s)
    }
}

pub mod format;

#[cfg(test)]
mod tests;
