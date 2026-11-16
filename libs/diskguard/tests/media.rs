//! Testy pohledu na vymenna media: co se ukaze a kam smi zapis.

use diskguard::gpt::{self, NewPartition};
use diskguard::media::WHOLE;
use diskguard::{BASIC_DATA_TYPE, Decision, ESP_TYPE, Kind, MediaFs, OUR_TYPE, SECTOR, guid, media_partitions};

const SECTORS: u64 = 8192;

/// Typ "Linux filesystem".
const LINUX: diskguard::Guid = guid(0x0FC6_3DAF, 0x8483, 0x4772, [0x8E, 0x79, 0x3D, 0x69, 0xD8, 0x47, 0x7D, 0xE4]);

fn boot(fs: MediaFs) -> [u8; 512] {
    let mut b = [0u8; 512];
    b[0] = 0xEB;
    match fs {
        MediaFs::ExFat => b[3..11].copy_from_slice(b"EXFAT   "),
        MediaFs::Ntfs => b[3..11].copy_from_slice(b"NTFS    "),
        MediaFs::Fat => {
            b[3..11].copy_from_slice(b"MSWIN4.1");
            b[11..13].copy_from_slice(&512u16.to_le_bytes());
            b[13] = 8;
            b[14..16].copy_from_slice(&32u16.to_le_bytes());
            b[16] = 2;
            b[82..90].copy_from_slice(b"FAT32   ");
        }
    }
    b[510] = 0x55;
    b[511] = 0xAA;
    b
}

fn put(d: &mut [u8], lba: u64, data: &[u8]) {
    let at = lba as usize * SECTOR;
    d[at..at + data.len()].copy_from_slice(data);
}

/// Disk s MBR: (typ, prvni sektor, pocet, obsah).
fn mbr(parts: &[(u8, u32, u32, Option<MediaFs>)]) -> Vec<u8> {
    let mut d = vec![0u8; SECTORS as usize * SECTOR];
    for (i, &(t, first, count, fs)) in parts.iter().enumerate() {
        let e = 446 + i * 16;
        d[e + 4] = t;
        d[e + 8..e + 12].copy_from_slice(&first.to_le_bytes());
        d[e + 12..e + 16].copy_from_slice(&count.to_le_bytes());
        if let Some(fs) = fs {
            put(&mut d, first as u64, &boot(fs));
        }
    }
    d[510] = 0x55;
    d[511] = 0xAA;
    d
}

fn gpt_disk(parts: &[(diskguard::Guid, u64, u64, &str, u64, Option<MediaFs>)]) -> Vec<u8> {
    let mut d = vec![0u8; SECTORS as usize * SECTOR];
    let np: Vec<NewPartition> = parts
        .iter()
        .map(|&(t, first, last, name, attrs, _)| NewPartition { type_guid: t, guid: [first as u8; 16], first, last, attrs, name })
        .collect();
    gpt::write(&mut d, SECTOR, [7; 16], &np).unwrap();
    for &(_, first, _, _, _, fs) in parts {
        if let Some(fs) = fs {
            put(&mut d, first, &boot(fs));
        }
    }
    d
}

fn run(d: &[u8], removable: bool) -> Vec<Decision> {
    media_partitions((d.len() / SECTOR) as u64, SECTOR, removable, |lba, buf| {
        let s = lba as usize * SECTOR;
        match d.get(s..s + buf.len()) {
            Some(src) => {
                buf.copy_from_slice(src);
                true
            }
            None => false,
        }
    })
}

fn kinds(v: &[Decision]) -> Vec<(u32, MediaFs, bool)> {
    v.iter()
        .map(|d| match d.kind {
            Kind::Media { index, fs } => (index, fs, d.writable),
            k => panic!("necekany druh {k:?}"),
        })
        .collect()
}

#[test]
fn superfloppy_writable_only_when_removable() {
    let mut d = vec![0u8; SECTORS as usize * SECTOR];
    put(&mut d, 0, &boot(MediaFs::Fat));
    let v = run(&d, true);
    assert_eq!(kinds(&v), [(WHOLE, MediaFs::Fat, true)]);
    assert_eq!((v[0].start, v[0].sectors), (0, SECTORS));
    assert_eq!(kinds(&run(&d, false)), [(WHOLE, MediaFs::Fat, false)], "vnitrni disk jen pro cteni");
}

#[test]
fn mbr_fat_writable_ntfs_read_only() {
    let d = mbr(&[(0x0C, 2048, 2048, Some(MediaFs::Fat)), (0x07, 4096, 2048, Some(MediaFs::Ntfs))]);
    let v = run(&d, true);
    assert_eq!(kinds(&v), [(0, MediaFs::Fat, true), (1, MediaFs::Ntfs, false)]);
    assert_eq!((v[0].start, v[0].sectors), (2048, 2048));
    assert_eq!(v[0].translate(2047, 1), Some(4095), "posledni sektor oddilu");
    assert_eq!(v[0].translate(2048, 1), None, "za oddilem nic");
}

#[test]
fn mbr_exfat_and_type_mismatch() {
    let d = mbr(&[(0x07, 2048, 1024, Some(MediaFs::ExFat)), (0x07, 4096, 1024, Some(MediaFs::Fat)), (0x83, 6000, 1024, Some(MediaFs::Fat))]);
    assert_eq!(kinds(&run(&d, true)), [(0, MediaFs::ExFat, true), (1, MediaFs::Fat, false), (2, MediaFs::Fat, false)]);
}

#[test]
fn mbr_esp_extended_overlap_and_out_of_disk_skipped() {
    let d = mbr(&[(0xEF, 2048, 1024, Some(MediaFs::Fat)), (0x0F, 3072, 1024, Some(MediaFs::Fat))]);
    assert!(run(&d, true).is_empty(), "ESP a rozsireny oddil se nevystavi");
    let d = mbr(&[(0x0C, 2048, 2048, Some(MediaFs::Fat)), (0x0C, 3000, 2048, Some(MediaFs::Fat))]);
    assert!(run(&d, true).is_empty(), "prekryv = ani jeden");
    let d = mbr(&[(0x0C, 8000, 1000, None)]);
    assert!(run(&d, true).is_empty(), "mimo disk");
    let d = mbr(&[(0x0C, 0, 100, None)]);
    assert!(run(&d, true).is_empty(), "pres sektor 0");
}

#[test]
fn gpt_basic_data_esp_and_foreign_types() {
    let d = gpt_disk(&[
        (ESP_TYPE, 2048, 3071, "EFI", 0, Some(MediaFs::Fat)),
        (BASIC_DATA_TYPE, 3072, 5119, "DATA", 0, Some(MediaFs::Fat)),
        (BASIC_DATA_TYPE, 5120, 6143, "RO", diskguard::ATTR_READ_ONLY, Some(MediaFs::ExFat)),
        (LINUX, 6144, 7167, "LINUX", 0, Some(MediaFs::Fat)),
    ]);
    assert_eq!(kinds(&run(&d, true)), [(1, MediaFs::Fat, true), (2, MediaFs::ExFat, false), (3, MediaFs::Fat, false)]);
    assert_eq!(kinds(&run(&d, false)), [(1, MediaFs::Fat, false), (2, MediaFs::ExFat, false), (3, MediaFs::Fat, false)], "vnitrni disk");
}

#[test]
fn disk_with_our_partition_has_no_media() {
    let d = gpt_disk(&[(OUR_TYPE, 2048, 5119, "MagnumOpus", 0, None), (BASIC_DATA_TYPE, 5120, 7167, "MO-LOGY", 0, Some(MediaFs::Fat))]);
    assert!(run(&d, true).is_empty());
}

#[test]
fn unknown_empty_and_garbage() {
    assert!(run(&vec![0u8; SECTORS as usize * SECTOR], true).is_empty());
    let mut d = vec![0x5Au8; SECTORS as usize * SECTOR];
    d[510] = 0x55;
    d[511] = 0xAA;
    // Nesmyslne polozky MBR (mimo disk) se nevystavi.
    assert!(run(&d, true).is_empty());
    // Chyba cteni = nic.
    assert!(media_partitions(SECTORS, SECTOR, true, |_, _| false).is_empty());
}
