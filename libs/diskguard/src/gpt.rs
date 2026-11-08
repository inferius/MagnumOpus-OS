//! Tabulka oddilu GPT (specifikace UEFI 2.10, kap. 5.3): cteni s kontrolou
//! kontrolnich souctu a zapis (testovaci disky v `xtask`).
//!
//! Rozlozeni: LBA 0 ochranny MBR, LBA 1 hlavicka, LBA 2.. tabulka oddilu
//! (typicky 128 polozek po 128 B = 32 sektoru), na konci disku zalozni
//! tabulka a zalozni hlavicka v poslednim sektoru.
//!
//! Puvod: napsano podle verejne specifikace, bez prevzateho kodu jinych
//! projektu (zadny kod z Linuxu ani BSD).

extern crate alloc;

use alloc::vec;
use alloc::vec::Vec;

use crate::{Guid, SECTOR, crc32};

const SIGNATURE: &[u8; 8] = b"EFI PART";
/// Nejvetsi tabulka, kterou cteme (spec. minimum je 16 KiB; vetsi tabulky
/// se v praxi nepouzivaji a nesmyslna hodnota nesmi vycerpat pamet).
const MAX_TABLE_BYTES: u64 = 256 * 1024;

/// Polozka tabulky oddilu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Entry {
    pub type_guid: Guid,
    pub guid: Guid,
    pub first: u64,
    /// Posledni sektor oddilu (vcetne).
    pub last: u64,
    pub attrs: u64,
    /// Jmeno v UTF-16LE, doplnene nulami.
    pub name: [u16; 36],
}

impl Entry {
    pub fn is_empty(&self) -> bool {
        self.type_guid == [0; 16]
    }

    /// Je jmeno oddilu presne `s`?
    pub fn name_is(&self, s: &str) -> bool {
        let mut units = s.encode_utf16();
        for &u in &self.name {
            match units.next() {
                Some(c) if c == u => {}
                Some(_) => return false,
                None => return u == 0,
            }
        }
        units.next().is_none()
    }

    fn parse(b: &[u8]) -> Self {
        let u64_at = |o: usize| u64::from_le_bytes(b[o..o + 8].try_into().unwrap());
        let mut name = [0u16; 36];
        for (i, n) in name.iter_mut().enumerate() {
            *n = u16::from_le_bytes([b[56 + i * 2], b[57 + i * 2]]);
        }
        Self {
            type_guid: b[0..16].try_into().unwrap(),
            guid: b[16..32].try_into().unwrap(),
            first: u64_at(32),
            last: u64_at(40),
            attrs: u64_at(48),
            name,
        }
    }
}

/// Prectena a overena tabulka.
#[derive(Debug, Clone)]
pub struct Table {
    pub first_usable: u64,
    pub last_usable: u64,
    pub disk_guid: Guid,
    entry_size: usize,
    raw: Vec<u8>,
    /// Tabulka je ze zalozni hlavicky (primarni je poskozena).
    pub from_backup: bool,
}

impl Table {
    pub fn entries(&self) -> impl Iterator<Item = Entry> + '_ {
        self.raw.chunks_exact(self.entry_size).map(Entry::parse)
    }
}

/// Vysledek cteni GPT.
#[derive(Debug, Clone)]
pub enum Read {
    Table(Table),
    /// Zadna hlavicka GPT (ani zalozni).
    None,
    /// Podpis GPT je, ale hlavicka nebo tabulka neprosla kontrolou.
    Damaged,
    /// Chyba cteni.
    Unreadable,
}

struct Header {
    my_lba: u64,
    first_usable: u64,
    last_usable: u64,
    disk_guid: Guid,
    entries_lba: u64,
    count: u32,
    entry_size: u32,
    entries_crc: u32,
}

/// Rozebere hlavicku v sektoru `s`, ktera ma lezet na `expected_lba`.
/// `None` = bez podpisu, `Some(Err)` = podpis je, ale hlavicka je spatne.
fn parse_header(s: &[u8], expected_lba: u64, sectors: u64) -> Option<Result<Header, ()>> {
    if &s[0..8] != SIGNATURE {
        return None;
    }
    let u32_at = |o: usize| u32::from_le_bytes(s[o..o + 4].try_into().unwrap());
    let u64_at = |o: usize| u64::from_le_bytes(s[o..o + 8].try_into().unwrap());
    let size = u32_at(12) as usize;
    if !(92..=SECTOR).contains(&size) {
        return Some(Err(()));
    }
    let mut copy = [0u8; SECTOR];
    copy[..size].copy_from_slice(&s[..size]);
    copy[16..20].fill(0);
    if crc32::hash(&copy[..size]) != u32_at(16) {
        return Some(Err(()));
    }
    let h = Header {
        my_lba: u64_at(24),
        first_usable: u64_at(40),
        last_usable: u64_at(48),
        disk_guid: s[56..72].try_into().unwrap(),
        entries_lba: u64_at(72),
        count: u32_at(80),
        entry_size: u32_at(84),
        entries_crc: u32_at(88),
    };
    let table_bytes = h.count as u64 * h.entry_size as u64;
    let table_sectors = table_bytes.div_ceil(SECTOR as u64);
    let sane = h.my_lba == expected_lba
        && h.first_usable <= h.last_usable
        && h.last_usable < sectors
        && h.entry_size >= 128
        && h.entry_size.is_power_of_two()
        && h.count > 0
        && table_bytes <= MAX_TABLE_BYTES
        && h.entries_lba >= 1
        && h.entries_lba.checked_add(table_sectors).is_some_and(|e| e <= sectors);
    Some(if sane { Ok(h) } else { Err(()) })
}

/// Precte tabulku podle hlavicky; `None` = chyba cteni, `Some(None)` =
/// spatny kontrolni soucet.
fn read_entries(
    h: &Header,
    read: &mut impl FnMut(u64, &mut [u8]) -> bool,
) -> Option<Option<Vec<u8>>> {
    let bytes = h.count as usize * h.entry_size as usize;
    let mut raw = vec![0u8; bytes.next_multiple_of(SECTOR)];
    if !read(h.entries_lba, &mut raw) {
        return None;
    }
    raw.truncate(bytes);
    Some((crc32::hash(&raw) == h.entries_crc).then_some(raw))
}

/// Precte GPT disku o `sectors` sektorech: primarni hlavicku, pri jeji
/// poskozeni zalozni.
pub fn read_table(sectors: u64, read: &mut impl FnMut(u64, &mut [u8]) -> bool) -> Read {
    let mut saw_signature = false;
    for lba in [1, sectors - 1] {
        let mut s = [0u8; SECTOR];
        if !read(lba, &mut s) {
            return Read::Unreadable;
        }
        let h = match parse_header(&s, lba, sectors) {
            None => continue,
            Some(Err(())) => {
                saw_signature = true;
                continue;
            }
            Some(Ok(h)) => h,
        };
        saw_signature = true;
        match read_entries(&h, read) {
            None => return Read::Unreadable,
            Some(None) => continue,
            Some(Some(raw)) => {
                return Read::Table(Table {
                    first_usable: h.first_usable,
                    last_usable: h.last_usable,
                    disk_guid: h.disk_guid,
                    entry_size: h.entry_size as usize,
                    raw,
                    from_backup: lba != 1,
                });
            }
        }
    }
    if saw_signature { Read::Damaged } else { Read::None }
}

/// Oddil pro [`write`].
#[derive(Debug, Clone, Copy)]
pub struct NewPartition<'a> {
    pub type_guid: Guid,
    pub guid: Guid,
    pub first: u64,
    pub last: u64,
    pub attrs: u64,
    pub name: &'a str,
}

/// Pocet polozek a sektoru tabulky, kterou zapisuje [`write`].
pub const WRITE_ENTRIES: usize = 128;
pub const WRITE_ENTRY_SIZE: usize = 128;
pub const WRITE_TABLE_SECTORS: u64 = (WRITE_ENTRIES * WRITE_ENTRY_SIZE / SECTOR) as u64;
/// Prvni pouzitelny sektor po [`write`] (MBR + hlavicka + tabulka).
pub const FIRST_USABLE: u64 = 2 + WRITE_TABLE_SECTORS;

/// Posledni pouzitelny sektor disku o `sectors` sektorech po [`write`].
pub const fn last_usable(sectors: u64) -> u64 {
    sectors - 1 - WRITE_TABLE_SECTORS - 1
}

/// Zapise do obrazu disku `disk` ochranny MBR, primarni i zalozni GPT
/// s oddily `parts`. Obsah oddilu nemeni. Chyba = oddil mimo pouzitelnou
/// oblast nebo prilis mnoho oddilu.
pub fn write(disk: &mut [u8], disk_guid: Guid, parts: &[NewPartition]) -> Result<(), &'static str> {
    if !disk.len().is_multiple_of(SECTOR) || parts.len() > WRITE_ENTRIES {
        return Err("spatna velikost disku nebo prilis mnoho oddilu");
    }
    let sectors = (disk.len() / SECTOR) as u64;
    if sectors < FIRST_USABLE + WRITE_TABLE_SECTORS + 2 {
        return Err("disk je prilis maly pro GPT");
    }
    let last_lba = sectors - 1;
    let last = last_usable(sectors);
    let mut entries = vec![0u8; WRITE_ENTRIES * WRITE_ENTRY_SIZE];
    for (p, e) in parts.iter().zip(entries.as_chunks_mut::<WRITE_ENTRY_SIZE>().0.iter_mut()) {
        if p.first < FIRST_USABLE || p.last > last || p.first > p.last {
            return Err("oddil mimo pouzitelnou oblast");
        }
        e[0..16].copy_from_slice(&p.type_guid);
        e[16..32].copy_from_slice(&p.guid);
        e[32..40].copy_from_slice(&p.first.to_le_bytes());
        e[40..48].copy_from_slice(&p.last.to_le_bytes());
        e[48..56].copy_from_slice(&p.attrs.to_le_bytes());
        for (i, unit) in p.name.encode_utf16().take(36).enumerate() {
            e[56 + i * 2..58 + i * 2].copy_from_slice(&unit.to_le_bytes());
        }
    }
    let entries_crc = crc32::hash(&entries);

    // Ochranny MBR: jeden oddil typu 0xEE pres cely disk.
    {
        let mbr = &mut disk[..SECTOR];
        mbr[446..510].fill(0);
        let entry = &mut mbr[446..462];
        entry[1..4].copy_from_slice(&[0x00, 0x02, 0x00]);
        entry[4] = 0xEE;
        entry[5..8].copy_from_slice(&[0xFF, 0xFF, 0xFF]);
        entry[8..12].copy_from_slice(&1u32.to_le_bytes());
        let size = (sectors - 1).min(u32::MAX as u64) as u32;
        entry[12..16].copy_from_slice(&size.to_le_bytes());
        mbr[510] = 0x55;
        mbr[511] = 0xAA;
    }

    let header = |my_lba: u64, alternate: u64, entries_lba: u64| {
        let mut h = [0u8; SECTOR];
        h[0..8].copy_from_slice(SIGNATURE);
        h[8..12].copy_from_slice(&0x0001_0000u32.to_le_bytes());
        h[12..16].copy_from_slice(&92u32.to_le_bytes());
        h[24..32].copy_from_slice(&my_lba.to_le_bytes());
        h[32..40].copy_from_slice(&alternate.to_le_bytes());
        h[40..48].copy_from_slice(&FIRST_USABLE.to_le_bytes());
        h[48..56].copy_from_slice(&last.to_le_bytes());
        h[56..72].copy_from_slice(&disk_guid);
        h[72..80].copy_from_slice(&entries_lba.to_le_bytes());
        h[80..84].copy_from_slice(&(WRITE_ENTRIES as u32).to_le_bytes());
        h[84..88].copy_from_slice(&(WRITE_ENTRY_SIZE as u32).to_le_bytes());
        h[88..92].copy_from_slice(&entries_crc.to_le_bytes());
        let crc = crc32::hash(&h[..92]);
        h[16..20].copy_from_slice(&crc.to_le_bytes());
        h
    };
    let at = |lba: u64| lba as usize * SECTOR;
    let backup_entries = last_lba - WRITE_TABLE_SECTORS;
    disk[at(1)..at(2)].copy_from_slice(&header(1, last_lba, 2));
    disk[at(2)..at(2 + WRITE_TABLE_SECTORS)].copy_from_slice(&entries);
    disk[at(backup_entries)..at(last_lba)].copy_from_slice(&entries);
    disk[at(last_lba)..].copy_from_slice(&header(last_lba, 1, backup_entries));
    Ok(())
}
