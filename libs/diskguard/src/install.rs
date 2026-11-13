//! Podpora instalatoru: pridani nasich oddilu do existujici GPT (cizi
//! polozky zustanou beze zmeny) a pravidlo zapisu pres "raw" kanal disku.
//!
//! Raw kanal dostane jen instalator (pres init, po vyslovnem potvrzeni
//! uzivatele). Ovladac pri kazdem zapisu overi [`raw_write_allowed`]:
//! zapsat jde jen
//! - struktury GPT (ochranny MBR, hlavicky a tabulky - na zacatku a konci
//!   disku), a
//! - obsah oddilu, ktere jsou nase: typ [`OUR_TYPE`] se jmenem
//!   [`OUR_LABEL`], nebo typ ESP se jmenem [`OUR_ESP_LABEL`].
//!
//! Obsah cizich oddilu (Windows, Linux, cizi ESP) se tak nepreplese ani
//! chybou instalatoru. Prepsani tabulky (rezim "cely disk") je zamerny
//! krok, ktery potvrzuje uzivatel opsanim nazvu disku v instalatoru.

use alloc::vec;
use alloc::vec::Vec;

use crate::gpt::{self, Ends, NewPartition, Table};
use crate::{ESP_TYPE, Guid, OUR_LABEL, OUR_TYPE, crc32};

/// Jmeno nasi ESP (oddil se zavadecem MagnumOpus vedle jinych systemu).
pub const OUR_ESP_LABEL: &str = "MagnumOpus EFI";

/// Je polozka nas oddil (data nebo vlastni ESP)?
pub fn is_installer_owned(e: &gpt::Entry) -> bool {
    (e.type_guid == OUR_TYPE && e.name_is(OUR_LABEL)) || (e.type_guid == ESP_TYPE && e.name_is(OUR_ESP_LABEL))
}

/// Smi raw kanal zapsat sektory `lba..lba+count` disku o `sectors`
/// sektorech? `table` = aktualni GPT disku (`None` = bez tabulky: jen
/// oblasti struktur GPT).
pub fn raw_write_allowed(table: Option<&Table>, sectors: u64, ss: usize, lba: u64, count: u64) -> bool {
    let Some(end) = lba.checked_add(count).filter(|&e| e <= sectors && count > 0) else {
        return false;
    };
    let last = end - 1;
    // Struktury GPT ve standardnim rozlozeni (nova tabulka) i podle
    // existujici hlavicky.
    let tsec = gpt::table_sectors(ss);
    let mut areas = vec![(0, gpt::first_usable(ss) - 1), (sectors.saturating_sub(1 + tsec), sectors - 1)];
    if let Some(t) = table {
        areas.extend(t.reserved.iter().copied());
    }
    if areas.iter().any(|&(a, b)| a <= lba && last <= b) {
        return true;
    }
    table.is_some_and(|t| t.entries().any(|e| !e.is_empty() && is_installer_owned(&e) && e.first <= lba && last <= e.last))
}

/// Volne useky disku (mimo oddily a struktury GPT), zarovnane na `align`
/// sektoru: (prvni, posledni) vcetne, jen useky aspon `min` sektoru.
pub fn free_ranges(table: &Table, align: u64, min: u64) -> Vec<(u64, u64)> {
    let mut used: Vec<(u64, u64)> = table.entries().filter(|e| !e.is_empty()).map(|e| (e.first, e.last)).collect();
    used.sort();
    let mut out = Vec::new();
    let mut at = table.first_usable;
    for (a, b) in used.iter().copied().chain(core::iter::once((table.last_usable + 1, table.last_usable + 1))) {
        if a > at {
            let first = at.next_multiple_of(align);
            // Konec zarovnany dolu (posledni sektor pred hranici).
            let end = a.min(table.last_usable + 1) / align * align;
            if end > first && end - first >= min {
                out.push((first, end - 1));
            }
        }
        at = at.max(b + 1);
    }
    out
}

/// Nove zacatky a konce disku: existujici tabulka `table` s pridanymi
/// oddily `parts` ve volnych slotech. Disk GUID, pouzitelna oblast a vsechny
/// existujici polozky zustanou; ochranny MBR (sektor 0) se nemeni -
/// [`Ends::head`] zacina sektorem 1 ([`Ends::head_lba`]).
///
/// Chyba = oddil mimo pouzitelnou oblast, prekryv s existujicim oddilem,
/// zadny volny slot, nebo neobvykle rozlozeni (primarni tabulka jinde nez
/// v sektoru 2).
pub fn add_partitions(table: &Table, sectors: u64, ss: usize, parts: &[NewPartition]) -> Result<Ends, &'static str> {
    if table.from_backup {
        return Err("primarni GPT je poskozena - nejdriv ji oprav (instalator ji neprepisuje)");
    }
    let (count, entry_size, raw) = table.raw_parts();
    let tsec = ((count * entry_size) as u64).div_ceil(ss as u64);
    if !table.reserved.get(2).is_some_and(|&(a, _)| a == 2) {
        return Err("neobvykle rozlozeni GPT (tabulka neni v sektoru 2)");
    }
    let mut entries = raw.to_vec();
    for (i, p) in parts.iter().enumerate() {
        if p.first < table.first_usable || p.last > table.last_usable || p.first > p.last {
            return Err("oddil mimo pouzitelnou oblast");
        }
        let clash = table.entries().any(|e| !e.is_empty() && e.first <= p.last && p.first <= e.last)
            || parts.iter().enumerate().any(|(j, o)| j != i && o.first <= p.last && p.first <= o.last);
        if clash {
            return Err("oddil se prekryva s existujicim");
        }
        let slot = entries
            .chunks(entry_size)
            .position(|e| e[..16].iter().all(|&b| b == 0))
            .ok_or("v tabulce GPT neni volny slot")?;
        let e = &mut entries[slot * entry_size..(slot + 1) * entry_size];
        e.fill(0);
        e[0..16].copy_from_slice(&p.type_guid);
        e[16..32].copy_from_slice(&p.guid);
        e[32..40].copy_from_slice(&p.first.to_le_bytes());
        e[40..48].copy_from_slice(&p.last.to_le_bytes());
        e[48..56].copy_from_slice(&p.attrs.to_le_bytes());
        for (k, unit) in p.name.encode_utf16().take(36).enumerate() {
            e[56 + k * 2..58 + k * 2].copy_from_slice(&unit.to_le_bytes());
        }
    }
    let crc = crc32::hash(&entries);
    let last_lba = sectors - 1;
    let backup_entries = last_lba - tsec;
    let header = |my: u64, alt: u64, at: u64| header(ss, my, alt, at, table, count, entry_size, crc);
    let mut padded = entries.clone();
    padded.resize(tsec as usize * ss, 0);
    let mut head = header(1, last_lba, 2);
    head.extend_from_slice(&padded);
    let mut tail = padded;
    tail.extend_from_slice(&header(last_lba, 1, backup_entries));
    Ok(Ends { head, tail, head_lba: 1 })
}

#[allow(clippy::too_many_arguments)]
fn header(ss: usize, my: u64, alt: u64, entries_lba: u64, t: &Table, count: usize, entry_size: usize, crc: u32) -> Vec<u8> {
    let mut h = vec![0u8; ss];
    h[0..8].copy_from_slice(b"EFI PART");
    h[8..12].copy_from_slice(&0x0001_0000u32.to_le_bytes());
    h[12..16].copy_from_slice(&92u32.to_le_bytes());
    h[24..32].copy_from_slice(&my.to_le_bytes());
    h[32..40].copy_from_slice(&alt.to_le_bytes());
    h[40..48].copy_from_slice(&t.first_usable.to_le_bytes());
    h[48..56].copy_from_slice(&t.last_usable.to_le_bytes());
    h[56..72].copy_from_slice(&t.disk_guid);
    h[72..80].copy_from_slice(&entries_lba.to_le_bytes());
    h[80..84].copy_from_slice(&(count as u32).to_le_bytes());
    h[84..88].copy_from_slice(&(entry_size as u32).to_le_bytes());
    h[88..92].copy_from_slice(&crc.to_le_bytes());
    let c = crc32::hash(&h[..92]);
    h[16..20].copy_from_slice(&c.to_le_bytes());
    h
}

/// Nahodne-ish GUID z 16 bajtu entropie (verze 4).
pub fn guid_from(mut b: [u8; 16]) -> Guid {
    b[7] = (b[7] & 0x0F) | 0x40;
    b[8] = (b[8] & 0x3F) | 0x80;
    b
}
