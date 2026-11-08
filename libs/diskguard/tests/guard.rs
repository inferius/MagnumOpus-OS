//! Testy ochrany cizich disku nad obrazy v pameti.

use diskguard::gpt::{self, NewPartition};
use diskguard::{Decision, Kind, OUR_LABEL, OUR_TYPE, SECTOR, decide, guid};

/// Typ "Linux filesystem" (cizi oddil).
const LINUX: diskguard::Guid = guid(
    0x0FC6_3DAF,
    0x8483,
    0x4772,
    [0x8E, 0x79, 0x3D, 0x69, 0xD8, 0x47, 0x7D, 0xE4],
);
/// Typ "Microsoft basic data".
const BASIC: diskguard::Guid = guid(
    0xEBD0_A0A2,
    0xB9E5,
    0x4433,
    [0x87, 0xC0, 0x68, 0xB6, 0xB7, 0x26, 0x99, 0xC7],
);

const DISK_SECTORS: u64 = 8192;

fn part(type_guid: diskguard::Guid, first: u64, last: u64, name: &str) -> NewPartition<'_> {
    NewPartition {
        type_guid,
        guid: [first as u8; 16],
        first,
        last,
        attrs: 0,
        name,
    }
}

fn disk_with(parts: &[NewPartition]) -> Vec<u8> {
    let mut d = vec![0u8; DISK_SECTORS as usize * SECTOR];
    gpt::write(&mut d, [7; 16], parts).unwrap();
    d
}

fn run(disk: &[u8]) -> Decision {
    let sectors = (disk.len() / SECTOR) as u64;
    decide(sectors, |lba, buf| {
        let start = lba as usize * SECTOR;
        match disk.get(start..start + buf.len()) {
            Some(src) => {
                buf.copy_from_slice(src);
                true
            }
            None => false,
        }
    })
}

#[test]
fn our_partition_by_type() {
    let d = disk_with(&[part(LINUX, 34, 2047, "linux"), part(OUR_TYPE, 2048, 6143, "data")]);
    let r = run(&d);
    assert_eq!(r.kind, Kind::OurPartition { index: 1 });
    assert_eq!((r.start, r.sectors, r.writable), (2048, 4096, true));
    assert_eq!(r.translate(0, 8), Some(2048));
    assert_eq!(r.translate(4095, 1), Some(6143));
    assert_eq!(r.translate(4095, 2), None, "za koncem oddilu");
    assert_eq!(r.translate(u64::MAX, 2), None, "preteceni");
}

#[test]
fn our_partition_by_label() {
    let d = disk_with(&[part(BASIC, 2048, 4095, OUR_LABEL)]);
    let r = run(&d);
    assert_eq!(r.kind, Kind::OurPartition { index: 0 });
    assert!(r.writable);
}

#[test]
fn label_must_match_exactly() {
    let d = disk_with(&[part(BASIC, 2048, 4095, "MagnumOpus2")]);
    assert_eq!(run(&d).kind, Kind::ForeignGpt);
    let d = disk_with(&[part(BASIC, 2048, 4095, "Magnum")]);
    assert_eq!(run(&d).kind, Kind::ForeignGpt);
}

#[test]
fn foreign_gpt_is_read_only() {
    let d = disk_with(&[part(LINUX, 2048, 8000, "root")]);
    let r = run(&d);
    assert_eq!(r.kind, Kind::ForeignGpt);
    assert_eq!((r.start, r.sectors, r.writable), (0, DISK_SECTORS, false));
}

#[test]
fn read_only_attribute_respected() {
    let mut p = part(OUR_TYPE, 2048, 4095, "data");
    p.attrs = diskguard::ATTR_READ_ONLY;
    let r = run(&disk_with(&[p]));
    assert_eq!(r.kind, Kind::OurPartition { index: 0 });
    assert!(!r.writable);
}

#[test]
fn overlapping_partition_rejected() {
    // Nas oddil prekryva cizi: zapis by znicil cizi data.
    let d = disk_with(&[part(LINUX, 2048, 4095, "root"), part(OUR_TYPE, 4000, 6000, "data")]);
    let r = run(&d);
    assert_eq!(r.kind, Kind::InvalidPartition);
    assert!(!r.writable);
}

#[test]
fn second_valid_partition_used_when_first_invalid() {
    let d = disk_with(&[
        part(LINUX, 2048, 4095, "root"),
        part(OUR_TYPE, 4000, 4500, "spatny"),
        part(OUR_TYPE, 6000, 7000, "dobry"),
    ]);
    assert_eq!(run(&d).kind, Kind::OurPartition { index: 2 });
}

#[test]
fn corrupted_primary_uses_backup() {
    let mut d = disk_with(&[part(OUR_TYPE, 2048, 4095, "data")]);
    d[SECTOR + 40] ^= 1; // primarni hlavicka: spatny CRC
    let r = run(&d);
    assert_eq!(r.kind, Kind::OurPartition { index: 0 });
}

#[test]
fn corrupted_table_both_copies_is_damaged() {
    let mut d = disk_with(&[part(OUR_TYPE, 2048, 4095, "data")]);
    d[2 * SECTOR] ^= 1; // primarni tabulka
    let backup = (DISK_SECTORS - 1 - gpt::WRITE_TABLE_SECTORS) as usize * SECTOR;
    d[backup] ^= 1; // zalozni tabulka
    let r = run(&d);
    assert_eq!(r.kind, Kind::DamagedGpt);
    assert!(!r.writable);
}

#[test]
fn protective_mbr_without_headers_is_damaged() {
    let mut d = disk_with(&[part(OUR_TYPE, 2048, 4095, "data")]);
    d[SECTOR..2 * SECTOR].fill(0);
    let last = (DISK_SECTORS - 1) as usize * SECTOR;
    d[last..].fill(0);
    assert_eq!(run(&d).kind, Kind::DamagedGpt);
}

#[test]
fn mbr_disk_is_foreign() {
    let mut d = vec![0u8; DISK_SECTORS as usize * SECTOR];
    d[446 + 4] = 0x07; // NTFS
    d[510] = 0x55;
    d[511] = 0xAA;
    let r = run(&d);
    assert_eq!(r.kind, Kind::Mbr);
    assert!(!r.writable);
}

#[test]
fn empty_disk_is_read_only() {
    let d = vec![0u8; DISK_SECTORS as usize * SECTOR];
    let r = run(&d);
    assert_eq!(r.kind, Kind::Unknown);
    assert!(!r.writable);
}

#[test]
fn whole_disk_fs_is_ours() {
    let mut d = vec![0u8; DISK_SECTORS as usize * SECTOR];
    // Superblok v bloku 2 (sektor 16).
    d[16 * SECTOR..16 * SECTOR + 8].copy_from_slice(b"OSFS\0\0v3");
    let r = run(&d);
    assert_eq!(r.kind, Kind::OurWholeDisk);
    assert_eq!((r.start, r.sectors, r.writable), (0, DISK_SECTORS, true));
}

#[test]
fn fs_magic_inside_mbr_disk_is_not_enough() {
    let mut d = vec![0u8; DISK_SECTORS as usize * SECTOR];
    d[510] = 0x55;
    d[511] = 0xAA;
    d[8 * SECTOR..8 * SECTOR + 8].copy_from_slice(b"OSFS\0\0v3");
    assert_eq!(run(&d).kind, Kind::Mbr);
}

#[test]
fn read_error_is_read_only() {
    let r = decide(DISK_SECTORS, |_, _| false);
    assert_eq!(r.kind, Kind::Unreadable);
    assert!(!r.writable);
}

#[test]
fn partition_past_disk_end_rejected() {
    // Tabulka zapsana pro vetsi disk a pak disk zkraceny: hlavicka je mimo.
    let big = disk_with(&[part(OUR_TYPE, 2048, 8000, "data")]);
    let small = &big[..4096 * SECTOR];
    let r = run(small);
    assert!(!r.kind.is_ours());
    assert!(!r.writable);
}

#[test]
fn writer_rejects_bad_ranges() {
    let mut d = vec![0u8; DISK_SECTORS as usize * SECTOR];
    assert!(gpt::write(&mut d, [0; 16], &[part(OUR_TYPE, 10, 100, "x")]).is_err());
    let end = gpt::last_usable(DISK_SECTORS) + 1;
    assert!(gpt::write(&mut d, [0; 16], &[part(OUR_TYPE, 2048, end, "x")]).is_err());
}

#[test]
fn crc_known_value() {
    assert_eq!(diskguard::crc32::hash(b"123456789"), 0xCBF4_3926);
}
