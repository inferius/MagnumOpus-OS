//! Testy podpory instalatoru: pridani oddilu do cizi GPT, volne useky,
//! pravidlo zapisu raw kanalu.

use diskguard::gpt::{self, NewPartition, Read};
use diskguard::install::{OUR_ESP_LABEL, add_partitions, free_ranges, raw_write_allowed};
use diskguard::{ESP_TYPE, OUR_LABEL, OUR_TYPE, SECTOR, decide, guid};

const BASIC: diskguard::Guid = guid(0xEBD0_A0A2, 0xB9E5, 0x4433, [0x87, 0xC0, 0x68, 0xB6, 0xB7, 0x26, 0x99, 0xC7]);
const SECTORS: u64 = 1 << 20; // 512 MiB

fn part(t: diskguard::Guid, first: u64, last: u64, name: &str) -> NewPartition<'_> {
    NewPartition { type_guid: t, guid: [first as u8 | 1; 16], first, last, attrs: 0, name }
}

/// Disk s "Windows": ESP a datovy oddil se znackou, zbytek volny.
fn windows_disk() -> Vec<u8> {
    let mut d = vec![0u8; SECTORS as usize * SECTOR];
    gpt::write(
        &mut d,
        SECTOR,
        [3; 16],
        &[part(ESP_TYPE, 2048, 206_847, "EFI system partition"), part(BASIC, 206_848, 409_599, "Basic data partition")],
    )
    .unwrap();
    d[206_848 * SECTOR..206_848 * SECTOR + 7].copy_from_slice(b"WINDOWS");
    d
}

fn table(d: &[u8]) -> gpt::Table {
    let mut read = |lba: u64, buf: &mut [u8]| {
        let o = lba as usize * SECTOR;
        buf.copy_from_slice(&d[o..o + buf.len()]);
        true
    };
    match gpt::read_table(SECTORS, SECTOR, &mut read) {
        Read::Table(t) => t,
        _ => panic!("tabulka"),
    }
}

#[test]
fn free_space_after_windows() {
    let d = windows_disk();
    let t = table(&d);
    let free = free_ranges(&t, 2048, 2048);
    assert_eq!(free.len(), 1);
    let (a, b) = free[0];
    assert_eq!(a, 409_600);
    assert_eq!((b + 1) % 2048, 0);
    assert!(b <= t.last_usable);
}

#[test]
fn add_keeps_foreign_entries_and_data() {
    let mut d = windows_disk();
    let before = d.clone();
    let t = table(&d);
    let (a, b) = free_ranges(&t, 2048, 2048)[0];
    let esp_end = a + 1_048_576 / 512 * 64 - 1;
    let parts = [part(ESP_TYPE, a, esp_end, OUR_ESP_LABEL), part(OUR_TYPE, esp_end + 1, b, OUR_LABEL)];
    let ends = add_partitions(&t, SECTORS, SECTOR, &parts).unwrap();
    assert_eq!(ends.head_lba, 1);
    // Zapis: zalozni konec, pak zacatek (od sektoru 1, MBR beze zmeny).
    let tail_at = d.len() - ends.tail.len();
    d[tail_at..].copy_from_slice(&ends.tail);
    d[SECTOR..SECTOR + ends.head.len()].copy_from_slice(&ends.head);
    assert_eq!(&d[..SECTOR], &before[..SECTOR], "MBR");
    let t2 = table(&d);
    assert!(!t2.from_backup);
    assert_eq!(t2.disk_guid, t.disk_guid);
    let old: Vec<_> = t.entries().filter(|e| !e.is_empty()).collect();
    let new: Vec<_> = t2.entries().filter(|e| !e.is_empty()).collect();
    assert_eq!(new.len(), 4);
    for e in &old {
        assert!(new.iter().any(|n| n.first == e.first && n.last == e.last && n.type_guid == e.type_guid));
    }
    // Ochrana disku ted vidi nas oddil.
    let dd = d.clone();
    let dec = decide(SECTORS, SECTOR, |lba, buf| {
        let o = lba as usize * SECTOR;
        buf.copy_from_slice(&dd[o..o + buf.len()]);
        true
    });
    assert_eq!(dec.start, esp_end + 1);
    assert!(dec.writable);
    // Data Windows nedotcena.
    assert_eq!(&d[206_848 * SECTOR..206_848 * SECTOR + 7], b"WINDOWS");
}

#[test]
fn add_rejects_overlap() {
    let d = windows_disk();
    let t = table(&d);
    assert!(add_partitions(&t, SECTORS, SECTOR, &[part(OUR_TYPE, 300_000, 500_000, OUR_LABEL)]).is_err());
    assert!(add_partitions(&t, SECTORS, SECTOR, &[part(OUR_TYPE, 10, 500, OUR_LABEL)]).is_err());
}

#[test]
fn raw_rule() {
    let mut d = windows_disk();
    let t = table(&d);
    // GPT na zacatku a konci: ano. ESP a data Windows: ne.
    assert!(raw_write_allowed(Some(&t), SECTORS, SECTOR, 0, 34));
    assert!(raw_write_allowed(Some(&t), SECTORS, SECTOR, SECTORS - 33, 33));
    assert!(!raw_write_allowed(Some(&t), SECTORS, SECTOR, 2048, 8));
    assert!(!raw_write_allowed(Some(&t), SECTORS, SECTOR, 206_848, 1));
    // Volne misto bez oddilu: ne.
    assert!(!raw_write_allowed(Some(&t), SECTORS, SECTOR, 500_000, 1));
    // Pres hranici GPT do oddilu: ne. Mimo disk: ne.
    assert!(!raw_write_allowed(Some(&t), SECTORS, SECTOR, 30, 4000));
    assert!(!raw_write_allowed(Some(&t), SECTORS, SECTOR, SECTORS - 1, 2));
    // Po pridani nasich oddilu: do nich ano.
    let (a, b) = free_ranges(&t, 2048, 2048)[0];
    let ends = add_partitions(&t, SECTORS, SECTOR, &[part(ESP_TYPE, a, a + 4095, OUR_ESP_LABEL), part(OUR_TYPE, a + 4096, b, OUR_LABEL)]).unwrap();
    let tail_at = d.len() - ends.tail.len();
    d[tail_at..].copy_from_slice(&ends.tail);
    d[SECTOR..SECTOR + ends.head.len()].copy_from_slice(&ends.head);
    let t2 = table(&d);
    assert!(raw_write_allowed(Some(&t2), SECTORS, SECTOR, a, 16));
    assert!(raw_write_allowed(Some(&t2), SECTORS, SECTOR, a + 4096, 64));
    assert!(!raw_write_allowed(Some(&t2), SECTORS, SECTOR, a + 4090, 16), "pres hranici dvou oddilu");
    assert!(!raw_write_allowed(Some(&t2), SECTORS, SECTOR, 2048, 1), "cizi ESP");
    // Disk bez tabulky: jen oblast GPT.
    assert!(raw_write_allowed(None, SECTORS, SECTOR, 1, 33));
    assert!(!raw_write_allowed(None, SECTORS, SECTOR, 4096, 1));
}
