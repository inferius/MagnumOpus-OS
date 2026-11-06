//! Vlastnosti formatu v2: souvisla alokace, fsync, navrat o generaci,
//! mazani snapshotu, defragmentace, TRIM, alokacni skupiny, BLAKE3.

use alloc::vec;
use alloc::vec::Vec;

use super::{assert_clean, new_fs, new_fs_with, opts, remount};
use crate::{Algo, BLOCK_SIZE, Error, FormatOptions, Fs, Kind, MemDevice, REUSE_DELAY};

fn pattern(len: usize, seed: u8) -> Vec<u8> {
    (0..len)
        .map(|i| (i as u32).wrapping_mul(2_654_435_761).to_le_bytes()[1] ^ seed)
        .collect()
}

#[test]
fn sequential_write_is_one_extent() {
    let mut fs = new_fs(8192);
    fs.create("/video", Kind::File).unwrap();
    // Po kouscich (jako aplikace), presto jeden souvisly usek.
    let data = pattern(10 * 1024 * 1024, 1);
    for chunk in data.chunks(65_536).enumerate() {
        fs.write("/video", (chunk.0 * 65_536) as u64, chunk.1)
            .unwrap();
    }
    fs.commit().unwrap();
    assert_eq!(fs.fragments("/video").unwrap(), 1);
    assert_eq!(fs.read("/video", 0, data.len()).unwrap(), data);
    assert_clean(&mut fs);
}

#[test]
fn fsync_survives_crash_without_commit() {
    let mut fs = new_fs(2048);
    fs.create("/a", Kind::Dir).unwrap();
    fs.commit().unwrap();
    fs.create("/a/maly", Kind::File).unwrap();
    fs.write("/a/maly", 0, b"ahoj").unwrap();
    fs.create("/a/velky", Kind::File).unwrap();
    let big = pattern(300_000, 7);
    fs.write("/a/velky", 0, &big).unwrap();
    fs.fsync().unwrap();
    // Druha davka: prepis casti, prejmenovani, zkraceni.
    fs.write("/a/velky", 4096 * 3 + 5, b"PREPIS").unwrap();
    fs.rename("/a/maly", "/maly-presunuty").unwrap();
    fs.truncate("/a/velky", 200_000).unwrap();
    fs.fsync().unwrap();
    // Zmena bez fsync se ztrati.
    fs.create("/ztraceny", Kind::File).unwrap();
    let mut fs = remount(fs);
    assert!(fs.info().replayed > 0);
    assert_eq!(fs.read("/maly-presunuty", 0, 10).unwrap(), b"ahoj");
    let mut expect = big[..200_000].to_vec();
    expect[4096 * 3 + 5..4096 * 3 + 11].copy_from_slice(b"PREPIS");
    assert_eq!(fs.read("/a/velky", 0, 300_000).unwrap(), expect);
    assert_eq!(fs.stat("/ztraceny").err(), Some(Error::NotFound));
    assert_eq!(fs.stat("/a").unwrap().tree_bytes, 200_000);
    assert_eq!(fs.stat("/").unwrap().tree_bytes, 200_004);
    assert_clean(&mut fs);
    // Po prehrani je vse ulozene commitem; dalsi pripojeni uz nic neprehrava.
    let mut fs = remount(fs);
    assert_eq!(fs.info().replayed, 0);
    assert_eq!(fs.read("/maly-presunuty", 0, 10).unwrap(), b"ahoj");
}

#[test]
fn fsync_of_inline_and_removed_files() {
    let mut fs = new_fs(1024);
    fs.create("/x", Kind::File).unwrap();
    fs.write("/x", 0, &pattern(5000, 3)).unwrap();
    fs.create("/y", Kind::File).unwrap();
    fs.write("/y", 0, b"kratky").unwrap();
    fs.fsync().unwrap();
    fs.truncate("/x", 100).unwrap(); // bloky -> inline
    fs.remove("/y").unwrap();
    fs.fsync().unwrap();
    let mut fs = remount(fs);
    assert_eq!(
        fs.read("/x", 0, 1000).unwrap(),
        pattern(5000, 3)[..100].to_vec()
    );
    assert_eq!(fs.stat("/y").err(), Some(Error::NotFound));
    assert_clean(&mut fs);
}

#[test]
fn fallback_to_previous_generation() {
    let mut fs = new_fs(1024);
    fs.create("/stary", Kind::File).unwrap();
    fs.commit().unwrap();
    let good_gen = fs.info().generation;
    fs.create("/novy", Kind::File).unwrap();
    fs.commit().unwrap();
    // Nejnovejsi koren "ztracen" (disk lhal o flush): prepsat blok korene.
    let mut dev = fs.into_device();
    let sb_block = crate::layout::SB_FIRST + (good_gen + 1) % crate::layout::SB_COUNT;
    let off = sb_block as usize * BLOCK_SIZE;
    let sb = crate::layout::Superblock::decode(&dev.data[off..off + BLOCK_SIZE]).unwrap();
    let root = sb.root.block as usize * BLOCK_SIZE;
    dev.data[root + 100] ^= 0xFF;
    let mut fs = Fs::mount(dev).unwrap();
    assert_eq!(fs.info().fallback, 1);
    assert!(fs.stat("/stary").is_ok());
    assert_eq!(fs.stat("/novy").err(), Some(Error::NotFound));
    assert_clean(&mut fs);
}

#[test]
fn deleting_snapshots_frees_space() {
    let mut fs = new_fs(4096);
    let base = fs.info().free_blocks;
    fs.create("/f", Kind::File).unwrap();
    fs.write("/f", 0, &pattern(1_000_000, 1)).unwrap();
    fs.snapshot("s1").unwrap();
    fs.write("/f", 0, &pattern(1_000_000, 2)).unwrap();
    fs.snapshot("s2").unwrap();
    fs.write("/f", 0, &pattern(1_000_000, 3)).unwrap();
    fs.commit().unwrap();
    assert_eq!(
        fs.snapshot_read("s1", "/f", 0, 1_000_000).unwrap(),
        pattern(1_000_000, 1)
    );
    assert_eq!(
        fs.snapshot_read("s2", "/f", 0, 1_000_000).unwrap(),
        pattern(1_000_000, 2)
    );
    assert_clean(&mut fs);
    // Smazat prostredni: s1 a zivy stav musi zustat cele.
    fs.delete_snapshot("s2").unwrap();
    fs.commit().unwrap();
    assert_eq!(
        fs.snapshot_read("s1", "/f", 0, 1_000_000).unwrap(),
        pattern(1_000_000, 1)
    );
    assert_eq!(fs.read("/f", 0, 1_000_000).unwrap(), pattern(1_000_000, 3));
    assert_clean(&mut fs);
    fs.delete_snapshot("S1").unwrap();
    fs.remove("/f").unwrap();
    for _ in 0..=REUSE_DELAY {
        fs.create("/t", Kind::File).unwrap();
        fs.remove("/t").unwrap();
        fs.commit().unwrap();
    }
    let free = fs.info().free_blocks;
    assert!(
        free + 16 >= base,
        "po smazani snapshotu se nevratilo misto: {base} -> {free}"
    );
    // Odlozene jsou jen bloky metadat poslednich commitu.
    assert!(fs.info().deferred_blocks < 64);
    assert_clean(&mut fs);
    assert_eq!(fs.delete_snapshot("s1").err(), Some(Error::NotFound));
}

#[test]
fn defrag_makes_file_contiguous() {
    let mut fs = new_fs(8192);
    fs.create("/a", Kind::File).unwrap();
    fs.create("/b", Kind::File).unwrap();
    // Stridave zapisy dvou souboru s commity = roztrhane soubory.
    for i in 0..40u64 {
        fs.write("/a", i * 16_384, &pattern(16_384, i as u8))
            .unwrap();
        fs.write("/b", i * 16_384, &pattern(16_384, 100 + i as u8))
            .unwrap();
        fs.commit().unwrap();
    }
    let before = fs.fragments("/a").unwrap();
    assert!(before > 10, "soubor mel byt roztrhany ({before})");
    let n = fs.defrag_file("/a").unwrap();
    assert!(n > 0);
    fs.commit().unwrap();
    assert_eq!(fs.fragments("/a").unwrap(), 1);
    for i in 0..40u64 {
        assert_eq!(
            fs.read("/a", i * 16_384, 16_384).unwrap(),
            pattern(16_384, i as u8)
        );
    }
    // Krok na pozadi dorovna i druhy soubor.
    let mut total = 0;
    loop {
        let r = fs.defrag_step(1 << 20).unwrap();
        total += r.blocks;
        if r.done {
            break;
        }
    }
    assert!(total > 0);
    fs.commit().unwrap();
    assert_eq!(fs.fragments("/b").unwrap(), 1);
    assert_clean(&mut fs);
}

#[test]
fn defrag_skips_snapshot_blocks() {
    let mut fs = new_fs(8192);
    fs.create("/a", Kind::File).unwrap();
    fs.create("/b", Kind::File).unwrap();
    for i in 0..20u64 {
        fs.write("/a", i * 16_384, &pattern(16_384, i as u8))
            .unwrap();
        fs.write("/b", i * 16_384, &pattern(16_384, 1)).unwrap();
        fs.commit().unwrap();
    }
    fs.snapshot("s").unwrap();
    let free = fs.info().free_blocks;
    assert_eq!(fs.defrag_file("/a").unwrap(), 0, "bloky drzi snapshot");
    fs.commit().unwrap();
    assert!(fs.info().free_blocks + 8 >= free);
    assert_clean(&mut fs);
}

#[test]
fn discard_is_sent_after_delay() {
    let mut fs = new_fs(2048);
    fs.set_discard(true);
    fs.create("/x", Kind::File).unwrap();
    fs.write("/x", 0, &pattern(400_000, 9)).unwrap();
    fs.commit().unwrap();
    fs.remove("/x").unwrap();
    fs.commit().unwrap();
    for _ in 0..REUSE_DELAY {
        fs.create("/t", Kind::File).unwrap();
        fs.commit().unwrap();
        fs.remove("/t").unwrap();
    }
    fs.commit().unwrap();
    let dev = fs.into_device();
    assert!(
        dev.discarded >= 97,
        "TRIM nedorazil ({} bloku)",
        dev.discarded
    );
    let mut fs = Fs::mount(dev).unwrap();
    assert!(fs.trim_all().unwrap() > 1000);
    assert_clean(&mut fs);
}

#[test]
fn many_allocation_groups_and_blake3() {
    let o = FormatOptions {
        label: "ag".into(),
        algo: Algo::Blake3,
        ag_blocks: Some(64),
        uuid: None,
    };
    let mut fs = new_fs_with(4096, &o);
    assert_eq!(fs.info().algo, Algo::Blake3);
    for i in 0..30 {
        let p = alloc::format!("/f{i}");
        fs.create(&p, Kind::File).unwrap();
        fs.write(&p, 0, &pattern(50_000 + i * 3000, i as u8))
            .unwrap();
        if i % 7 == 0 {
            fs.commit().unwrap();
        }
    }
    fs.commit().unwrap();
    let mut fs = remount(fs);
    for i in 0..30 {
        assert_eq!(
            fs.read(&alloc::format!("/f{i}"), 0, 200_000).unwrap(),
            pattern(50_000 + i * 3000, i as u8)
        );
    }
    assert_clean(&mut fs);
}

#[test]
fn space_log_condenses() {
    let mut fs = new_fs(4096);
    fs.create("/churn", Kind::File).unwrap();
    for round in 0..300u32 {
        fs.write(
            "/churn",
            (round as u64 % 7) * 8192,
            &pattern(8192, round as u8),
        )
        .unwrap();
        fs.commit().unwrap();
    }
    let blocks = fs.space_log_blocks(0);
    assert!(blocks < 40, "space log se nezhustil ({blocks} bloku)");
    let mut fs = remount(fs);
    assert_clean(&mut fs);
}

#[test]
fn uncommitted_write_without_room_fails_early() {
    let mut fs = new_fs(256);
    fs.create("/f", Kind::File).unwrap();
    assert_eq!(
        fs.write("/f", 0, &vec![1u8; 2 << 20]).err(),
        Some(Error::NoSpace)
    );
    // System zustava pouzitelny.
    fs.write("/f", 0, &vec![1u8; 100_000]).unwrap();
    fs.commit().unwrap();
    assert_clean(&mut fs);
}

#[test]
fn format_rejects_tiny_device_and_mount_empty() {
    assert_eq!(
        Fs::format(MemDevice::new(16), &opts("x")).err().map(|_| ()),
        Some(())
    );
    assert_eq!(Fs::mount(MemDevice::new(256)).err(), Some(Error::NotFound));
}

#[test]
fn full_volume_can_free_space_by_deleting_snapshot() {
    // Scenar z review: data drzi uz jen snapshot, svazek je plny bezny
    // zapisem - smazani snapshotu musi projit a misto se musi vratit.
    let mut fs = new_fs(2048);
    fs.create("/stare", Kind::File).unwrap();
    fs.write("/stare", 0, &alloc::vec![1u8; 2 << 20]).unwrap();
    fs.snapshot("historie").unwrap();
    fs.remove("/stare").unwrap();
    fs.commit().unwrap();
    fs.create("/plnic", Kind::File).unwrap();
    let mut off = 0u64;
    loop {
        match fs.write("/plnic", off, &[2u8; 65536]) {
            Ok(()) => off += 65536,
            Err(Error::NoSpace) => break,
            Err(e) => panic!("{e:?}"),
        }
    }
    let full_at = off;
    fs.delete_snapshot("historie").unwrap();
    // Uvolnene bloky jsou pouzitelne po ochranne lhute (REUSE_DELAY commitu).
    for _ in 0..crate::REUSE_DELAY + 1 {
        fs.commit().unwrap();
    }
    fs.write("/plnic", off, &alloc::vec![3u8; 1 << 20]).unwrap();
    fs.commit().unwrap();
    assert!(fs.stat("/plnic").unwrap().size > full_at);
    assert_clean(&mut fs);
}
