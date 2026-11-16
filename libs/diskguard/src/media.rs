//! Vymenna media (flash disky): oddily s cizim souborovym systemem, ktere
//! system pripoji do `/media/<jmeno>` (docs/cs/media.md).
//!
//! Pro kazdy oddil FAT, exFAT nebo NTFS vznikne samostatny pohled
//! ([`Decision`] s druhem [`Kind::Media`]): klient vidi jen ten oddil.
//! **Zapis** se povoli jen kdyz plati vse:
//!
//! - zarizeni je vymenne (USB mass storage s bitem RMB v INQUIRY) a neni
//!   chranene proti zapisu - vnitrni disky (NVMe, AHCI, virtio) nikdy,
//! - souborovy system je FAT nebo exFAT (podle obsahu prvniho sektoru
//!   oddilu, ne jen podle typu v tabulce),
//! - typ oddilu tomu odpovida (MBR 0x01/0x04/0x06/0x0B/0x0C/0x0E pro FAT,
//!   0x07 pro exFAT; GPT "Microsoft basic data"); disk bez tabulky oddilu
//!   s FAT/exFAT primo v sektoru 0 ("superfloppy") taky.
//!
//! NTFS je vzdy jen pro cteni. Oddil EFI (GPT typ ESP, MBR 0xEF), rozsirene
//! oddily MBR (logicke oddily zatim ne), oddil mimo disk nebo prekryvajici
//! jiny se vynechaji. Disk s nasim oddilem (zive USB s oddilem pro logy)
//! zadne medium nema: oddil logu zapisuje jen `fatw` a dva pisatele jednoho
//! FAT by ho poskodili.

use alloc::vec::Vec;

use crate::{BASIC_DATA_TYPE, Decision, ESP_TYPE, Kind, gpt, valid_sector_size};

/// Souborovy system media podle prvniho sektoru oddilu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaFs {
    Fat,
    ExFat,
    Ntfs,
}

impl MediaFs {
    /// Jmeno pro zpravy a log (`fat`, `exfat`, `ntfs`).
    pub fn name(self) -> &'static str {
        match self {
            MediaFs::Fat => "fat",
            MediaFs::ExFat => "exfat",
            MediaFs::Ntfs => "ntfs",
        }
    }

    pub fn from_name(s: &str) -> Option<Self> {
        match s {
            "fat" => Some(MediaFs::Fat),
            "exfat" => Some(MediaFs::ExFat),
            "ntfs" => Some(MediaFs::Ntfs),
            _ => None,
        }
    }

    /// Smi se na tento system vubec zapisovat (podporujeme zapis)?
    pub fn writable_fs(self) -> bool {
        matches!(self, MediaFs::Fat | MediaFs::ExFat)
    }
}

/// Oznaceni pohledu bez tabulky oddilu (cele zarizeni).
pub const WHOLE: u32 = u32::MAX;

/// Rozpozna souborovy system z prvniho sektoru (boot sektoru) oddilu.
pub fn detect(boot: &[u8]) -> Option<MediaFs> {
    if boot.len() < 512 {
        return None;
    }
    match &boot[3..11] {
        b"EXFAT   " => return Some(MediaFs::ExFat),
        b"NTFS    " => return Some(MediaFs::Ntfs),
        _ => {}
    }
    if boot[510] != 0x55 || boot[511] != 0xAA {
        return None;
    }
    // BPB FAT: rozumne hodnoty a znacka typu (FAT12/16 na 54, FAT32 na 82).
    let bps = u16::from_le_bytes([boot[11], boot[12]]) as usize;
    let spc = boot[13];
    let reserved = u16::from_le_bytes([boot[14], boot[15]]);
    let fats = boot[16];
    let sane = valid_sector_size(bps) && bps <= 4096 && spc != 0 && spc.is_power_of_two() && reserved != 0 && (1..=2).contains(&fats);
    let tagged = boot[82..87] == *b"FAT32" || boot[54..57] == *b"FAT";
    (sane && tagged).then_some(MediaFs::Fat)
}

/// Typ oddilu MBR odpovida souborovemu systemu (pro povoleni zapisu)?
fn mbr_type_matches(t: u8, fs: MediaFs) -> bool {
    match fs {
        MediaFs::Fat => matches!(t, 0x01 | 0x04 | 0x06 | 0x0B | 0x0C | 0x0E),
        MediaFs::ExFat => t == 0x07,
        MediaFs::Ntfs => false,
    }
}

/// Oddily media na zarizeni o `sectors` sektorech po `ss` bajtech.
/// `removable` = vymenne zarizeni, ktere neni chranene proti zapisu (jen pak
/// muze byt nektery pohled zapisovatelny). Poradi: podle tabulky oddilu.
pub fn media_partitions(sectors: u64, ss: usize, removable: bool, mut read: impl FnMut(u64, &mut [u8]) -> bool) -> Vec<Decision> {
    let mut out = Vec::new();
    if !valid_sector_size(ss) || sectors < 2 {
        return out;
    }
    let mut s0 = alloc::vec![0u8; ss];
    if !read(0, &mut s0) {
        return out;
    }
    let boot_of = |first: u64, read: &mut dyn FnMut(u64, &mut [u8]) -> bool| -> Option<MediaFs> {
        let mut b = alloc::vec![0u8; ss];
        if read(first, &mut b) { detect(&b) } else { None }
    };
    match gpt::read_table(sectors, ss, &mut read) {
        gpt::Read::Table(t) => {
            // Disk s nasim oddilem (system, zive USB): zadna media.
            if t.entries().any(|e| !e.is_empty() && crate::is_our_entry(&e)) {
                return out;
            }
            for (i, e) in t.entries().enumerate() {
                if e.is_empty() || e.type_guid == ESP_TYPE {
                    continue;
                }
                let in_bounds = e.first <= e.last && e.first >= t.first_usable && e.last <= t.last_usable && e.last < sectors;
                let overlaps = t.entries().enumerate().any(|(j, o)| j != i && !o.is_empty() && o.first <= e.last && e.first <= o.last)
                    || t.reserved.iter().any(|&(a, b)| a <= e.last && e.first <= b);
                if !in_bounds || overlaps {
                    continue;
                }
                let Some(fs) = boot_of(e.first, &mut read) else { continue };
                let typed = e.type_guid == BASIC_DATA_TYPE;
                let ro_attr = e.attrs & crate::ATTR_READ_ONLY != 0;
                out.push(view(i as u32, fs, e.first, e.last - e.first + 1, ss, removable && typed && !ro_attr && fs.writable_fs()));
            }
            return out;
        }
        // Poskozena GPT nebo chyba cteni: nic (opatrne, zadny zapis).
        gpt::Read::Damaged | gpt::Read::Unreadable => return out,
        gpt::Read::None => {}
    }
    // Superfloppy: souborovy system primo v sektoru 0 (bez tabulky).
    if let Some(fs) = detect(&s0) {
        out.push(view(WHOLE, fs, 0, sectors, ss, removable && fs.writable_fs()));
        return out;
    }
    if s0[510] != 0x55 || s0[511] != 0xAA {
        return out;
    }
    // MBR: jen primarni oddily.
    let mut parts: Vec<(u32, u8, u64, u64)> = Vec::new();
    for i in 0..4 {
        let e = &s0[446 + i * 16..446 + i * 16 + 16];
        let t = e[4];
        let first = u32::from_le_bytes([e[8], e[9], e[10], e[11]]) as u64;
        let count = u32::from_le_bytes([e[12], e[13], e[14], e[15]]) as u64;
        // Prazdny, rozsireny (logicke oddily zatim ne), ochranny GPT, ESP.
        if matches!(t, 0x00 | 0x05 | 0x0F | 0x85 | 0xEE | 0xEF) || count == 0 {
            continue;
        }
        parts.push((i as u32, t, first, count));
    }
    for &(i, t, first, count) in &parts {
        let Some(end) = first.checked_add(count) else { continue };
        let in_bounds = first >= 1 && end <= sectors;
        let overlaps = parts.iter().any(|&(j, _, f, c)| j != i && f < end && first < f.saturating_add(c));
        if !in_bounds || overlaps {
            continue;
        }
        let Some(fs) = boot_of(first, &mut read) else { continue };
        out.push(view(i, fs, first, count, ss, removable && mbr_type_matches(t, fs) && fs.writable_fs()));
    }
    out
}

fn view(index: u32, fs: MediaFs, start: u64, sectors: u64, ss: usize, writable: bool) -> Decision {
    Decision { kind: Kind::Media { index, fs }, start, sectors, sector_size: ss, writable }
}
