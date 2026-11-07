//! Testy faze 3 (data): komprese, diry, predalokace, kopie dat
//! a samoleceni, odmitnuti stareho formatu.

use alloc::vec;
use alloc::vec::Vec;

use super::{assert_clean, new_fs, remount};
use crate::datarec::{DataSettings, DedupMode};
use crate::layout::{BLOCK_SIZE, COMP_CHUNK, Compress, SB_COUNT, SB_FIRST, Superblock};
use crate::{Error, Fs, Kind, MemDevice};

/// Dobre komprimovatelna data (opakujici se text s cisly).
pub(super) fn text(len: usize, seed: u32) -> Vec<u8> {
    let mut out = Vec::with_capacity(len + 64);
    let mut i = seed;
    while out.len() < len {
        out.extend_from_slice(alloc::format!("radek {i:08} souboroveho systemu; ").as_bytes());
        i = i.wrapping_add(1);
    }
    out.truncate(len);
    out
}

/// Nekomprimovatelna data (xorshift).
pub(super) fn noise(len: usize, seed: u64) -> Vec<u8> {
    let mut x = seed | 1;
    (0..len)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x as u8
        })
        .collect()
}

fn set(fs: &mut Fs<MemDevice>, compress: Option<Compress>, copies: u8) {
    let s = DataSettings {
        compress,
        copies,
        dedup: DedupMode::Off,
    };
    fs.set_data_settings("/", &s).unwrap();
}

/// Obsazene bloky (bez cekajicich na odlozene uvolneni).
fn used(fs: &mut Fs<MemDevice>) -> u64 {
    let i = fs.info();
    i.block_count - i.free_blocks
}

#[test]
fn compression_saves_space_and_roundtrips() {
    for algo in [Compress::Lz4, Compress::Zstd] {
        let mut fs = new_fs(8192);
        set(&mut fs, Some(algo), 1);
        let data = text(1 << 20, 7);
        let before = used(&mut fs);
        fs.create("/t", Kind::File).unwrap();
        fs.write("/t", 0, &data).unwrap();
        fs.commit().unwrap();
        let grew = used(&mut fs) - before;
        assert!(grew < 128, "{algo:?}: 1 MiB textu zabral {grew} bloku");
        let ext = fs.test_extents("/t");
        assert!(ext.iter().all(|(_, e)| e.is_compressed() && e.len() <= COMP_CHUNK));
        assert_eq!(fs.read("/t", 0, data.len()).unwrap(), data);
        let mut fs = remount(fs);
        assert_eq!(fs.read("/t", 12_345, 70_000).unwrap(), data[12_345..82_345]);
        assert_clean(&mut fs);
    }
}

#[test]
fn incompressible_data_stays_plain() {
    let mut fs = new_fs(4096);
    set(&mut fs, Some(Compress::Lz4), 1);
    let data = noise(256 * 1024, 3);
    fs.create("/n", Kind::File).unwrap();
    fs.write("/n", 0, &data).unwrap();
    fs.commit().unwrap();
    assert!(fs.test_extents("/n").iter().all(|(_, e)| !e.is_compressed()));
    assert_eq!(fs.read("/n", 0, data.len()).unwrap(), data);
    assert_clean(&mut fs);
}

#[test]
fn compressed_chunk_partial_changes() {
    let mut fs = new_fs(8192);
    set(&mut fs, Some(Compress::Lz4), 1);
    let mut data = text(600_000, 1);
    fs.create("/t", Kind::File).unwrap();
    fs.write("/t", 0, &data).unwrap();
    fs.commit().unwrap();
    // Prepis uprostred kusu, pres hranici kusu a zkraceni uvnitr kusu.
    for (off, len) in [(5000usize, 100usize), (COMP_CHUNK as usize * BLOCK_SIZE - 50, 300), (300_000, 1)] {
        let patch = vec![0xA5; len];
        fs.write("/t", off as u64, &patch).unwrap();
        data[off..off + len].copy_from_slice(&patch);
    }
    fs.commit().unwrap();
    assert_eq!(fs.read("/t", 0, data.len()).unwrap(), data);
    fs.truncate("/t", 200_001).unwrap();
    data.truncate(200_001);
    fs.commit().unwrap();
    let mut fs = remount(fs);
    assert_eq!(fs.read("/t", 0, 1 << 20).unwrap(), data);
    // Zvetseni po zkraceni cte nuly.
    fs.truncate("/t", 250_000).unwrap();
    let tail = fs.read("/t", 200_001, 49_999).unwrap();
    assert!(tail.iter().all(|&b| b == 0));
    assert_clean(&mut fs);
}

#[test]
fn compression_off_by_default() {
    let mut fs = new_fs(4096);
    let s = fs.data_settings_of("/").unwrap();
    assert_eq!(s.compress, None);
    assert_eq!(s.copies, 1);
    assert_eq!(s.dedup, DedupMode::Background);
    fs.create("/t", Kind::File).unwrap();
    fs.write("/t", 0, &text(200_000, 0)).unwrap();
    fs.commit().unwrap();
    assert!(fs.test_extents("/t").iter().all(|(_, e)| !e.is_compressed()));
}

#[test]
fn new_dataset_inherits_data_settings() {
    let mut fs = new_fs(4096);
    set(&mut fs, Some(Compress::Zstd), 2);
    fs.create_dataset("/d").unwrap();
    let s = fs.data_settings_of("/d").unwrap();
    assert_eq!((s.compress, s.copies), (Some(Compress::Zstd), 2));
    let s = remount(fs).data_settings_of("/d").unwrap();
    assert_eq!(s.compress, Some(Compress::Zstd));
}

#[test]
fn zero_blocks_become_holes() {
    let mut fs = new_fs(4096);
    let before = used(&mut fs);
    let mut data = vec![0u8; 1 << 20];
    data[(1 << 20) - 10..].copy_from_slice(b"konec dat!");
    fs.create("/z", Kind::File).unwrap();
    fs.write("/z", 0, &data).unwrap();
    fs.commit().unwrap();
    assert!(used(&mut fs) - before < 16, "nuly se ulozily");
    assert_eq!(fs.read("/z", 0, data.len()).unwrap(), data);
    // Prepis dat nulami uvolni bloky.
    fs.create("/w", Kind::File).unwrap();
    fs.write("/w", 0, &text(400_000, 2)).unwrap();
    fs.commit().unwrap();
    let full = used(&mut fs);
    fs.write("/w", 0, &vec![0u8; 400_000]).unwrap();
    fs.commit().unwrap();
    assert!(fs.test_extents("/w").is_empty());
    assert!(used(&mut fs) < full);
    assert!(fs.read("/w", 0, 400_000).unwrap().iter().all(|&b| b == 0));
    assert_clean(&mut fs);
}

#[test]
fn prealloc_reads_zeros_and_writes_in_place() {
    let mut fs = new_fs(8192);
    fs.create("/db", Kind::File).unwrap();
    fs.fallocate("/db", 0, 2 << 20, false).unwrap();
    assert_eq!(fs.stat("/db").unwrap().size, 2 << 20);
    let ext = fs.test_extents("/db");
    assert!(!ext.is_empty() && ext.iter().all(|(_, e)| e.is_unwritten()));
    let phys: Vec<u64> = ext.iter().map(|(_, e)| e.phys).collect();
    assert!(fs.read("/db", 0, 1 << 20).unwrap().iter().all(|&b| b == 0));
    // Zapis doprostred jde na misto: fyzicke bloky se nezmeni.
    let page = text(8192, 5);
    fs.write("/db", 1 << 20, &page).unwrap();
    fs.commit().unwrap();
    let after: Vec<u64> = fs.test_extents("/db").iter().map(|(ks, e)| e.phys - ks).collect();
    assert!(after.iter().all(|p| *p == phys[0]), "zapis nesel na misto");
    assert_eq!(fs.fragments("/db").unwrap(), 1);
    let mut fs = remount(fs);
    assert_eq!(fs.read("/db", 1 << 20, 8192).unwrap(), page);
    assert!(fs.read("/db", 0, 4096).unwrap().iter().all(|&b| b == 0));
    assert_clean(&mut fs);
}

#[test]
fn prealloc_keep_size_and_snapshot_cow() {
    let mut fs = new_fs(8192);
    fs.create("/f", Kind::File).unwrap();
    fs.write("/f", 0, &text(10_000, 1)).unwrap();
    fs.fallocate("/f", 0, 1 << 20, true).unwrap();
    assert_eq!(fs.stat("/f").unwrap().size, 10_000);
    // Snapshot drzi predalokovane bloky: zapis jde CoW, snapshot cte nuly.
    fs.snapshot("s").unwrap();
    fs.write("/f", 500_000, b"novy obsah").unwrap();
    fs.commit().unwrap();
    assert_eq!(fs.read("/f", 500_000, 10).unwrap(), b"novy obsah");
    assert_eq!(fs.snapshot_read("s", "/f", 0, 1 << 20).unwrap().len(), 10_000);
    assert_clean(&mut fs);
}

#[test]
fn prealloc_write_survives_fsync_replay() {
    let mut fs = new_fs(8192);
    fs.create("/log", Kind::File).unwrap();
    fs.fallocate("/log", 0, 1 << 20, false).unwrap();
    let rec = text(20_000, 9);
    fs.write("/log", 4096, &rec).unwrap();
    fs.fsync().unwrap();
    // Vypadek bez commitu: prehrani logu.
    let mut fs = remount(fs);
    assert_eq!(fs.read("/log", 4096, rec.len()).unwrap(), rec);
    assert!(fs.test_extents("/log").iter().any(|(_, e)| e.is_unwritten()));
    assert_clean(&mut fs);
}

#[test]
fn data_copies_heal_damaged_block() {
    let mut fs = new_fs(8192);
    set(&mut fs, None, 2);
    let data = text(300_000, 4);
    fs.create("/k", Kind::File).unwrap();
    fs.write("/k", 0, &data).unwrap();
    fs.commit().unwrap();
    let (_, e) = fs.test_extents("/k")[0].clone();
    assert_ne!(e.copy, 0);
    let mut dev = fs.into_device();
    dev.data[(e.phys + 1) as usize * BLOCK_SIZE + 7] ^= 0x55;
    let mut fs = Fs::mount(dev).unwrap();
    assert_eq!(fs.read("/k", 0, data.len()).unwrap(), data);
    assert!(fs.test_healed() >= 1);
    // Oprava se zapsala na disk: dalsi pripojeni ji uz nepotrebuje.
    let mut fs = remount(fs);
    assert_eq!(fs.read("/k", 0, data.len()).unwrap(), data);
    assert_eq!(fs.test_healed(), 0);
    assert_clean(&mut fs);
}

#[test]
fn metadata_copy_heals_node() {
    let mut fs = new_fs(4096);
    for i in 0..50 {
        let p = alloc::format!("/soubor-{i}");
        fs.create(&p, Kind::File).unwrap();
        fs.write(&p, 0, b"obsah").unwrap();
    }
    fs.commit().unwrap();
    let generation = fs.info().generation;
    let mut dev = fs.into_device();
    let o = (SB_FIRST + generation % SB_COUNT) as usize * BLOCK_SIZE;
    let sb = Superblock::decode(&dev.data[o..o + BLOCK_SIZE]).unwrap();
    assert_ne!(sb.root.copy, 0, "koren bez kopie");
    dev.data[sb.root.block as usize * BLOCK_SIZE + 200] ^= 0xFF;
    let mut fs = Fs::mount(dev).unwrap();
    assert_eq!(fs.info().fallback, 0);
    assert_eq!(fs.read("/soubor-7", 0, 5).unwrap(), b"obsah");
    assert!(fs.test_healed() >= 1);
    assert_clean(&mut fs);
}

#[test]
fn old_pointer_format_is_rejected() {
    let fs = new_fs(1024);
    let generation = fs.info().generation;
    let mut dev = fs.into_device();
    let o = (SB_FIRST + generation % SB_COUNT) as usize * BLOCK_SIZE;
    // Svazek bez priznaku ukazatele s kopii (stary tvar).
    let mut sb = Superblock::decode(&dev.data[o..o + BLOCK_SIZE]).unwrap();
    sb.features_incompat &= !crate::layout::feature::INCOMPAT_PTR_COPY;
    dev.data[o..o + BLOCK_SIZE].copy_from_slice(&sb.encode());
    for g in 0..SB_COUNT {
        if g != generation % SB_COUNT {
            let x = (SB_FIRST + g) as usize * BLOCK_SIZE;
            dev.data[x..x + BLOCK_SIZE].fill(0);
        }
    }
    assert_eq!(Fs::mount(dev).err(), Some(Error::NotSupported));
}

#[test]
fn deadlist_keeps_node_copies_apart() {
    // Hodne uzlu se snapshoty v mnoha kolech: kopie uzlu se alokuji po
    // castech od konce disku a sousedni zaznamy deadlistu nemaji souvisle
    // kopie. Smazani snapshotu nesmi uvolnit cizi bloky.
    let mut fs = new_fs(16384);
    for i in 0..400 {
        let p = alloc::format!("/f{i:03}");
        fs.create(&p, Kind::File).unwrap();
        fs.write(&p, 0, &text(3000, i)).unwrap();
    }
    fs.commit().unwrap();
    for round in 0..12u32 {
        fs.snapshot(&alloc::format!("s{round}")).unwrap();
        for i in (round as usize % 7..400).step_by(3 + round as usize % 4) {
            let p = alloc::format!("/f{i:03}");
            fs.write(&p, 0, &text(2500, round * 1000 + i as u32)).unwrap();
        }
        fs.commit().unwrap();
        if round >= 2 {
            fs.delete_snapshot(&alloc::format!("s{}", round - 2)).unwrap();
            fs.commit().unwrap();
        }
        assert_clean(&mut fs);
    }
}

#[test]
fn fsync_of_zeroed_range_survives() {
    let mut fs = new_fs(4096);
    fs.create("/z", Kind::File).unwrap();
    fs.write("/z", 0, &text(200_000, 1)).unwrap();
    fs.commit().unwrap();
    // Nuly = dira; log diru neumi, fsync proto commitne.
    fs.write("/z", 40_960, &vec![0u8; 81_920]).unwrap();
    fs.fsync().unwrap();
    let mut fs = remount(fs);
    assert!(fs.read("/z", 40_960, 81_920).unwrap().iter().all(|&b| b == 0));
    assert_clean(&mut fs);
}
