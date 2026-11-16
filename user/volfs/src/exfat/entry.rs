//! Polozky slozky exFAT (32 B): sada souboru = primarni `File` (0x85),
//! `Stream Extension` (0xC0) a 1..17 `File Name` (0xC1), s kontrolnim
//! souctem sady a hashem jmena. Casova razitka.

use volproto::{Error, Result};

use super::upcase::Upcase;
use crate::time::{civil_from_days, days_from_civil};

pub const ENTRY: usize = 32;
pub type Raw = [u8; ENTRY];

/// Typy polozek (bit 7 = pouzita).
pub mod ty {
    pub const END: u8 = 0x00;
    pub const BITMAP: u8 = 0x81;
    pub const UPCASE: u8 = 0x82;
    pub const LABEL: u8 = 0x83;
    pub const FILE: u8 = 0x85;
    pub const STREAM: u8 = 0xC0;
    pub const NAME: u8 = 0xC1;
    pub const IN_USE: u8 = 0x80;
    /// Sekundarni polozka (bit 6) a nedulezita = lze ignorovat (bit 5).
    pub const SECONDARY: u8 = 0x40;
    pub const BENIGN: u8 = 0x20;
}

/// Atributy souboru.
pub mod attr {
    pub const READ_ONLY: u16 = 0x01;
    pub const DIR: u16 = 0x10;
    pub const ARCHIVE: u16 = 0x20;
}

/// Priznaky `Stream Extension`.
pub mod sflag {
    pub const ALLOC_POSSIBLE: u8 = 1;
    pub const NO_FAT_CHAIN: u8 = 2;
}

/// Znaku jmena v jedne polozce `File Name`.
const NAME_CHARS: usize = 15;
/// Nejdelsi jmeno (znaky UTF-16).
pub const MAX_NAME: usize = 255;

/// Sada polozek souboru nebo slozky.
#[derive(Clone, Debug)]
pub struct FileSet {
    pub attrs: u16,
    /// Vytvoreni, zmena, pristup: (razitko, 10 ms, pasmo).
    pub ctime: (u32, u8, u8),
    pub mtime: (u32, u8, u8),
    pub atime: (u32, u8, u8),
    pub flags: u8,
    pub name: Vec<u16>,
    pub valid_len: u64,
    pub first_cluster: u32,
    pub len: u64,
    /// Sada ma dalsi (nedulezite) sekundarni polozky: zmena by je ztratila,
    /// proto jen pro cteni.
    pub extra: bool,
}

impl FileSet {
    pub fn is_dir(&self) -> bool {
        self.attrs & attr::DIR != 0
    }

    pub fn no_fat_chain(&self) -> bool {
        self.flags & sflag::NO_FAT_CHAIN != 0
    }

    /// Pocet polozek sady (vcetne primarni).
    pub fn entries(&self) -> usize {
        2 + self.name.len().div_ceil(NAME_CHARS)
    }
}

fn u16_at(b: &[u8], o: usize) -> u16 {
    u16::from_le_bytes([b[o], b[o + 1]])
}

fn u32_at(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}

fn u64_at(b: &[u8], o: usize) -> u64 {
    let mut a = [0u8; 8];
    a.copy_from_slice(&b[o..o + 8]);
    u64::from_le_bytes(a)
}

/// Kontrolni soucet sady (bez bajtu 2 a 3 primarni polozky).
pub fn set_checksum(set: &[Raw]) -> u16 {
    let mut s: u16 = 0;
    for (n, e) in set.iter().enumerate() {
        for (i, &b) in e.iter().enumerate() {
            if n == 0 && (i == 2 || i == 3) {
                continue;
            }
            s = s.rotate_right(1).wrapping_add(b as u16);
        }
    }
    s
}

/// Hash jmena (z velkych pismen, bajty UTF-16LE).
pub fn name_hash(up: &Upcase, name: &[u16]) -> u16 {
    let mut h: u16 = 0;
    for &c in name {
        for b in up.up(c).to_le_bytes() {
            h = h.rotate_right(1).wrapping_add(b as u16);
        }
    }
    h
}

/// Rozebere sadu; `set[0]` je primarni `File`, delka = `SecondaryCount + 1`.
/// Chyba = sada je poskozena (ignoruje se, misto zustava obsazene).
pub fn parse(set: &[Raw], up: &Upcase) -> Result<FileSet> {
    let p = &set[0];
    let count = p[1] as usize;
    if p[0] != ty::FILE || count < 2 || count + 1 != set.len() {
        return Err(Error::Corrupted);
    }
    if u16_at(p, 2) != set_checksum(set) {
        return Err(Error::Corrupted);
    }
    let s = &set[1];
    if s[0] != ty::STREAM {
        return Err(Error::Corrupted);
    }
    let name_len = s[3] as usize;
    let names = name_len.div_ceil(NAME_CHARS);
    if name_len == 0 || 1 + names > count {
        return Err(Error::Corrupted);
    }
    let mut name = Vec::with_capacity(name_len);
    for e in &set[2..2 + names] {
        if e[0] != ty::NAME {
            return Err(Error::Corrupted);
        }
        for i in 0..NAME_CHARS {
            if name.len() < name_len {
                name.push(u16_at(e, 2 + i * 2));
            }
        }
    }
    // Dalsi sekundarni polozky: nedulezite se preskoci, dulezite neznamy
    // = sada neplatna (specifikace).
    let mut extra = false;
    for e in &set[2 + names..] {
        if e[0] & (ty::IN_USE | ty::SECONDARY) != ty::IN_USE | ty::SECONDARY || e[0] & ty::BENIGN == 0 {
            return Err(Error::Corrupted);
        }
        extra = true;
    }
    if name_hash(up, &name) != u16_at(s, 4) {
        return Err(Error::Corrupted);
    }
    let flags = s[1];
    let valid_len = u64_at(s, 8);
    let first_cluster = u32_at(s, 20);
    let len = u64_at(s, 24);
    if valid_len > len || (first_cluster == 0 && len != 0) || (flags & sflag::ALLOC_POSSIBLE == 0 && first_cluster != 0) {
        return Err(Error::Corrupted);
    }
    Ok(FileSet {
        attrs: u16_at(p, 4),
        ctime: (u32_at(p, 8), p[20], p[22]),
        mtime: (u32_at(p, 12), p[21], p[23]),
        atime: (u32_at(p, 16), 0, p[24]),
        flags,
        name,
        valid_len,
        first_cluster,
        len,
        extra,
    })
}

/// Polozky sady (s kontrolnim souctem a hashem jmena).
pub fn build(f: &FileSet, up: &Upcase) -> Vec<Raw> {
    let n = f.entries();
    let mut set = vec![[0u8; ENTRY]; n];
    let p = &mut set[0];
    p[0] = ty::FILE;
    p[1] = (n - 1) as u8;
    p[4..6].copy_from_slice(&f.attrs.to_le_bytes());
    p[8..12].copy_from_slice(&f.ctime.0.to_le_bytes());
    p[12..16].copy_from_slice(&f.mtime.0.to_le_bytes());
    p[16..20].copy_from_slice(&f.atime.0.to_le_bytes());
    p[20] = f.ctime.1;
    p[21] = f.mtime.1;
    p[22] = f.ctime.2;
    p[23] = f.mtime.2;
    p[24] = f.atime.2;
    let s = &mut set[1];
    s[0] = ty::STREAM;
    s[1] = f.flags;
    s[3] = f.name.len() as u8;
    s[4..6].copy_from_slice(&name_hash(up, &f.name).to_le_bytes());
    s[8..16].copy_from_slice(&f.valid_len.to_le_bytes());
    s[20..24].copy_from_slice(&f.first_cluster.to_le_bytes());
    s[24..32].copy_from_slice(&f.len.to_le_bytes());
    for (k, chunk) in f.name.chunks(NAME_CHARS).enumerate() {
        let e = &mut set[2 + k];
        e[0] = ty::NAME;
        for (i, &c) in chunk.iter().enumerate() {
            e[2 + i * 2..4 + i * 2].copy_from_slice(&c.to_le_bytes());
        }
    }
    let sum = set_checksum(&set);
    set[0][2..4].copy_from_slice(&sum.to_le_bytes());
    set
}

/// Jmeno pro novou polozku: UTF-16, bez znaku zakazanych ve Windows
/// a bez tecky ci mezery na konci (Pruzkumnik je neotevre).
pub fn check_name(name: &str) -> Result<Vec<u16>> {
    if name.is_empty() || name == "." || name == ".." || name.ends_with('.') || name.ends_with(' ') {
        return Err(Error::InvalidArgs);
    }
    if name.chars().any(|c| (c as u32) < 0x20 || "\"*/:<>?\\|".contains(c)) {
        return Err(Error::InvalidArgs);
    }
    let u: Vec<u16> = name.encode_utf16().collect();
    if u.len() > MAX_NAME {
        return Err(Error::InvalidArgs);
    }
    Ok(u)
}

/// Sekundy od 1970 (UTC) -> (razitko, 10 ms, pasmo = UTC platne).
pub fn to_stamp(secs: i64) -> (u32, u8, u8) {
    let min = days_from_civil(1980, 1, 1) * 86400;
    let max = days_from_civil(2107, 12, 31) * 86400 + 86399;
    let s = secs.clamp(min, max);
    let (y, m, d) = civil_from_days(s.div_euclid(86400));
    let r = s.rem_euclid(86400);
    let (h, mi, sec) = (r / 3600, r % 3600 / 60, r % 60);
    let stamp = ((y - 1980) as u32) << 25 | m << 21 | d << 16 | (h as u32) << 11 | (mi as u32) << 5 | (sec as u32 / 2);
    (stamp, ((sec % 2) * 100) as u8, 0x80)
}

/// Razitko -> sekundy od 1970 (UTC; bez platneho pasma bere mistni cas
/// jako UTC, stejne jako FAT).
pub fn from_stamp(t: (u32, u8, u8)) -> i64 {
    let (st, ms10, tz) = t;
    let y = 1980 + (st >> 25) as i64;
    let m = ((st >> 21) & 0xF).clamp(1, 12);
    let d = ((st >> 16) & 0x1F).max(1);
    let secs = days_from_civil(y, m, d) * 86400 + ((st >> 11) & 0x1F) as i64 * 3600 + ((st >> 5) & 0x3F) as i64 * 60 + (st & 0x1F) as i64 * 2 + (ms10 as i64 / 100).min(1);
    if tz & 0x80 != 0 {
        // Posun v 15 minutach se znamenkem (7 bitu).
        let off = ((tz << 1) as i8 >> 1) as i64;
        secs - off * 15 * 60
    } else {
        secs
    }
}
