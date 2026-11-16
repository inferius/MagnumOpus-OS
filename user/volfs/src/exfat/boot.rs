//! Spousteci oblast exFAT: boot sektor, kontrolni soucet oblasti
//! (sektory 0..=10, sektor 11 = soucet), zalozni oblast od sektoru 12.
//!
//! Vse z media je nedoveryhodne: kazde pole se overi proti velikosti
//! zarizeni a proti ostatnim polim, aritmetika je kontrolovana.

use volproto::{Error, Result};

/// Priznaky svazku (`VolumeFlags`, bajt 106; nejsou v kontrolnim souctu).
pub mod flag {
    pub const ACTIVE_FAT: u16 = 1;
    pub const DIRTY: u16 = 2;
    pub const MEDIA_FAILURE: u16 = 4;
}

/// Pozice poli v boot sektoru, ktere se meni za behu.
pub const VOLUME_FLAGS: u64 = 106;
pub const PERCENT_IN_USE: u64 = 112;

/// Geometrie svazku (vse v bajtech, krome clusteru).
#[derive(Clone, Debug)]
pub struct Geometry {
    /// log2 velikosti clusteru v bajtech.
    pub cluster_shift: u32,
    /// Zacatek aktivni FAT.
    pub fat: u64,
    pub heap: u64,
    pub cluster_count: u32,
    pub root: u32,
    pub flags: u16,
    pub fats: u8,
    pub active_fat: u8,
}

impl Geometry {
    pub fn cluster_size(&self) -> u64 {
        1 << self.cluster_shift
    }

    /// Cislo clusteru je v datove oblasti.
    pub fn valid(&self, c: u32) -> bool {
        c >= 2 && c - 2 < self.cluster_count
    }

    /// Bajtova pozice clusteru (`c` musi byt platny).
    pub fn cluster_pos(&self, c: u32) -> u64 {
        self.heap + (((c - 2) as u64) << self.cluster_shift)
    }

    /// Pozice polozky FAT clusteru `c` v aktivni FAT.
    pub fn fat_pos(&self, c: u32) -> u64 {
        self.fat + c as u64 * 4
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

/// Kontrolni soucet spousteci oblasti: sektory 0..=10 bez bajtu 106, 107
/// (priznaky) a 112 (procento vyuziti).
pub fn checksum(region: &[u8]) -> u32 {
    let mut s: u32 = 0;
    for (i, &b) in region.iter().enumerate() {
        if i == 106 || i == 107 || i == 112 {
            continue;
        }
        s = s.rotate_right(1).wrapping_add(b as u32);
    }
    s
}

/// log2 velikosti sektoru z boot sektoru (nebo `None`, kdyz to neni exFAT).
pub fn sector_shift(boot: &[u8]) -> Option<u32> {
    if boot.len() < 512 || &boot[3..11] != b"EXFAT   " {
        return None;
    }
    let s = boot[108] as u32;
    (9..=12).contains(&s).then_some(s)
}

/// Rozebere a overi spousteci oblast (12 sektoru od boot sektoru).
/// `dev_len` = velikost oddilu v bajtech.
pub fn parse(region: &[u8], dev_len: u64) -> Result<Geometry> {
    let ss = sector_shift(region).ok_or(Error::Corrupted)?;
    let bps = 1usize << ss;
    if region.len() < 12 * bps {
        return Err(Error::Corrupted);
    }
    let b = &region[..bps];
    if b[510] != 0x55 || b[511] != 0xAA || b[0] != 0xEB || b[11..64].iter().any(|&x| x != 0) {
        return Err(Error::Corrupted);
    }
    // Soucet v sektoru 11 (cely sektor = opakovany soucet).
    let sum = checksum(&region[..11 * bps]);
    if region[11 * bps..12 * bps].chunks(4).any(|c| u32_at(c, 0) != sum) {
        return Err(Error::Corrupted);
    }
    // Hlavni verze 1 (1.00).
    if b[105] != 1 {
        return Err(Error::NotSupported);
    }
    let cs_shift = b[109] as u32;
    if ss + cs_shift > 25 {
        return Err(Error::Corrupted);
    }
    let fats = b[110];
    if fats != 1 && fats != 2 {
        return Err(Error::Corrupted);
    }
    let sec = |n: u64| n.checked_mul(bps as u64).ok_or(Error::Corrupted);
    let vol_len = sec(u64_at(b, 72))?;
    let fat_off = u32_at(b, 80) as u64;
    let fat_len = sec(u32_at(b, 84) as u64)?;
    let heap_off = u32_at(b, 88) as u64;
    let count = u32_at(b, 92);
    let root = u32_at(b, 96);
    let flags = u16_at(b, 106);
    if vol_len > dev_len || fat_off < 24 || count == 0 || count > 0xFFFF_FFF5 {
        return Err(Error::Corrupted);
    }
    let fat = sec(fat_off)?;
    // FAT musi pojmout polozky 0..count+2.
    if fat_len < (count as u64 + 2) * 4 {
        return Err(Error::Corrupted);
    }
    let fats_end = fat.checked_add(fat_len.checked_mul(fats as u64).ok_or(Error::Corrupted)?).ok_or(Error::Corrupted)?;
    let heap = sec(heap_off)?;
    let heap_end = heap.checked_add((count as u64) << (ss + cs_shift)).ok_or(Error::Corrupted)?;
    if heap < fats_end || heap_end > vol_len {
        return Err(Error::Corrupted);
    }
    let active_fat = (flags & flag::ACTIVE_FAT) as u8;
    if active_fat >= fats {
        return Err(Error::Corrupted);
    }
    let g = Geometry {
        cluster_shift: ss + cs_shift,
        fat: fat + active_fat as u64 * fat_len,
        heap,
        cluster_count: count,
        root,
        flags,
        fats,
        active_fat,
    };
    if !g.valid(root) {
        return Err(Error::Corrupted);
    }
    Ok(g)
}
