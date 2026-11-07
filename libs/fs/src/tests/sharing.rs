//! Testy faze 3: okamzite kopie (reflink), deduplikace, tabulka sdileni
//! a uctovani mista.

use alloc::vec;

use super::data::{noise, text};
use super::{assert_clean, new_fs, remount};
use crate::datarec::{DataSettings, DedupMode};
use crate::layout::{BLOCK_SIZE, Compress, REUSE_DELAY};
use crate::{Error, Fs, Kind, MemDevice};

/// Obsazene bloky vcetne cekajicich na odlozene uvolneni - po
/// `settle` presne odpovida pouzitemu mistu.
fn used(fs: &mut Fs<MemDevice>) -> u64 {
    let i = fs.info();
    i.block_count - i.free_blocks
}

/// Probehne dost commitu, aby odlozene uvolneni dobehlo.
fn settle(fs: &mut Fs<MemDevice>) {
    for _ in 0..REUSE_DELAY + 1 {
        fs.create("/.tik", Kind::File).unwrap();
        fs.commit().unwrap();
        fs.remove("/.tik").unwrap();
        fs.commit().unwrap();
    }
}

fn dedup(fs: &mut Fs<MemDevice>, mode: DedupMode) {
    let mut s = fs.data_settings_of("/").unwrap();
    s.dedup = mode;
    fs.set_data_settings("/", &s).unwrap();
}

#[test]
fn clone_file_shares_blocks() {
    let mut fs = new_fs(8192);
    dedup(&mut fs, DedupMode::Off);
    settle(&mut fs);
    let empty = used(&mut fs);
    let data = noise(2 << 20, 11);
    fs.create("/a", Kind::File).unwrap();
    fs.write("/a", 0, &data).unwrap();
    fs.commit().unwrap();
    let one = used(&mut fs);
    assert_eq!(fs.clone_path("/a", "/b").unwrap(), data.len() as u64);
    assert!(used(&mut fs) - one < 64, "kopie zabrala misto");
    assert_eq!(fs.read("/b", 0, data.len()).unwrap(), data);
    assert!(fs.test_extents("/b").iter().all(|(_, e)| e.is_shared()));
    assert_clean(&mut fs);
    // Zmena kopie nemeni original a naopak.
    fs.write("/b", 4096, b"zmena v kopii").unwrap();
    fs.write("/a", 1 << 20, b"zmena v originalu").unwrap();
    fs.commit().unwrap();
    assert_eq!(fs.read("/a", 4096, 13).unwrap(), &data[4096..4096 + 13]);
    assert_eq!(fs.read("/b", 1 << 20, 17).unwrap(), &data[1 << 20..(1 << 20) + 17]);
    // Smazani originalu: kopie zustane cela.
    fs.remove("/a").unwrap();
    fs.commit().unwrap();
    let mut fs = remount(fs);
    let mut want = data.clone();
    want[4096..4096 + 13].copy_from_slice(b"zmena v kopii");
    assert_eq!(fs.read("/b", 0, data.len()).unwrap(), want);
    assert_clean(&mut fs);
    // Smazani kopie: vsechno misto zpet.
    fs.remove("/b").unwrap();
    fs.commit().unwrap();
    settle(&mut fs);
    // Tolerance: space log a uzly stromu se nezmensi presne zpet.
    assert!(used(&mut fs) <= empty + 16, "sdilene bloky se neuvolnily ({} > {empty})", used(&mut fs));
    assert_clean(&mut fs);
}

#[test]
fn clone_range_and_errors() {
    let mut fs = new_fs(8192);
    let a = noise(400_000, 2);
    let b = text(300_000, 3);
    fs.create("/a", Kind::File).unwrap();
    fs.write("/a", 0, &a).unwrap();
    fs.create("/b", Kind::File).unwrap();
    fs.write("/b", 0, &b).unwrap();
    // Nezarovnany posun.
    assert_eq!(fs.clone_range("/a", 100, "/b", 0, 4096).err(), Some(Error::InvalidArgs));
    // Prekryv v tomtez souboru.
    assert_eq!(fs.clone_range("/a", 0, "/a", 4096, 16384).err(), Some(Error::InvalidArgs));
    fs.clone_range("/a", 8192, "/b", 40960, 65536).unwrap();
    let mut want = b.clone();
    want[40960..40960 + 65536].copy_from_slice(&a[8192..8192 + 65536]);
    assert_eq!(fs.read("/b", 0, want.len()).unwrap(), want);
    // Az do konce zdroje (delka mimo bloky) za konec cile: cil se zvetsi.
    let tail = a.len() as u64 - 4096 * 90;
    fs.clone_range("/a", 4096 * 90, "/b", 4096 * 100, 1 << 30).unwrap();
    assert_eq!(fs.stat("/b").unwrap().size, 4096 * 100 + tail);
    assert_eq!(fs.read("/b", 4096 * 100, tail as usize).unwrap(), &a[4096 * 90..]);
    let mut fs = remount(fs);
    assert_eq!(fs.read("/b", 40960, 65536).unwrap(), &a[8192..8192 + 65536]);
    assert_clean(&mut fs);
}

#[test]
fn clone_directory_recursively() {
    let mut fs = new_fs(8192);
    fs.create("/p", Kind::Dir).unwrap();
    fs.create("/p/sub", Kind::Dir).unwrap();
    fs.create("/p/sub/f", Kind::File).unwrap();
    fs.write("/p/sub/f", 0, &noise(100_000, 5)).unwrap();
    fs.create("/p/maly", Kind::File).unwrap();
    fs.write("/p/maly", 0, b"kratky").unwrap();
    fs.symlink("/p/odkaz", "sub/f").unwrap();
    assert_eq!(fs.clone_path("/p", "/p/uvnitr").err(), Some(Error::InvalidArgs));
    fs.clone_path("/p", "/q").unwrap();
    assert_eq!(fs.read("/q/sub/f", 0, 100_000).unwrap(), noise(100_000, 5));
    assert_eq!(fs.read("/q/maly", 0, 10).unwrap(), b"kratky");
    assert_eq!(fs.readlink("/q/odkaz").unwrap(), "sub/f");
    assert_clean(&mut fs);
}

#[test]
fn clone_survives_snapshot_deletion() {
    let mut fs = new_fs(8192);
    dedup(&mut fs, DedupMode::Off);
    settle(&mut fs);
    let empty = used(&mut fs);
    let data = noise(1 << 20, 8);
    fs.create("/a", Kind::File).unwrap();
    fs.write("/a", 0, &data).unwrap();
    fs.snapshot("s").unwrap();
    fs.remove("/a").unwrap();
    // Obnova ze snapshotu sdili bloky (okamzita, bez mista navic).
    let before = used(&mut fs);
    fs.restore_copy("/", "s", "/a", "/obnoveny").unwrap();
    fs.commit().unwrap();
    assert!(used(&mut fs) - before < 64);
    assert!(fs.test_extents("/obnoveny").iter().all(|(_, e)| e.is_shared()));
    fs.delete_snapshot("s").unwrap();
    fs.commit().unwrap();
    let mut fs = remount(fs);
    assert_eq!(fs.read("/obnoveny", 0, data.len()).unwrap(), data);
    assert_clean(&mut fs);
    fs.remove("/obnoveny").unwrap();
    fs.commit().unwrap();
    settle(&mut fs);
    assert!(used(&mut fs) <= empty + 16);
    assert_clean(&mut fs);
}

#[test]
fn clone_between_datasets_and_compressed() {
    let mut fs = new_fs(8192);
    fs.set_data_settings("/", &DataSettings { compress: Some(Compress::Lz4), copies: 1, dedup: DedupMode::Off }).unwrap();
    fs.create_dataset("/d").unwrap();
    let data = text(500_000, 6);
    fs.create("/a", Kind::File).unwrap();
    fs.write("/a", 0, &data).unwrap();
    fs.clone_path("/a", "/d/a").unwrap();
    assert!(fs.test_extents("/d/a").iter().all(|(_, e)| e.is_shared() && e.is_compressed()));
    // Cast komprimovaneho kusu se kopiruje (rozbali), zbytek sdili.
    fs.clone_range("/a", 4096, "/d/a", 0, 8192).unwrap();
    let mut want = data.clone();
    want.copy_within(4096..4096 + 8192, 0);
    assert_eq!(fs.read("/d/a", 0, want.len()).unwrap(), want);
    fs.remove("/a").unwrap();
    fs.delete_dataset("/d").unwrap();
    fs.commit().unwrap();
    assert_clean(&mut fs);
}

#[test]
fn background_dedup_merges_identical_files() {
    let mut fs = new_fs(16384);
    settle(&mut fs);
    let empty = used(&mut fs);
    let data = noise(1 << 20, 21);
    for p in ["/x", "/y", "/z"] {
        fs.create(p, Kind::File).unwrap();
        fs.write(p, 0, &data).unwrap();
        fs.commit().unwrap();
    }
    let three = used(&mut fs);
    let mut merged = 0;
    loop {
        let r = fs.dedup_step(16).unwrap();
        merged += r.merged;
        if r.done {
            break;
        }
    }
    assert_eq!(merged, 2 * (1 << 20) / (32 * BLOCK_SIZE as u64), "kusy se nesloucily");
    fs.commit().unwrap();
    settle(&mut fs);
    let after = used(&mut fs);
    // 2 x 256 bloku dat, minus zaznamy tabulky sdileni a indexu.
    assert!(three - after >= 450, "deduplikace neusetrila misto ({three} -> {after})");
    assert_clean(&mut fs);
    // Dalsi pruchod nic noveho nenajde (jen nova data).
    let r = fs.dedup_step(1000).unwrap();
    assert_eq!(r.merged, 0);
    // Zmena jedne kopie nemeni ostatni.
    fs.write("/y", 1000, b"jina data").unwrap();
    fs.commit().unwrap();
    assert_eq!(fs.read("/x", 0, data.len()).unwrap(), data);
    assert_eq!(fs.read("/z", 1000, 9).unwrap(), &data[1000..1009]);
    let mut fs = remount(fs);
    assert_clean(&mut fs);
    for p in ["/x", "/y", "/z"] {
        fs.remove(p).unwrap();
    }
    fs.commit().unwrap();
    settle(&mut fs);
    assert!(used(&mut fs) <= empty + 16, "po smazani zustalo misto ({} > {empty})", used(&mut fs));
    assert_clean(&mut fs);
}

#[test]
fn inline_dedup_at_write() {
    let mut fs = new_fs(16384);
    dedup(&mut fs, DedupMode::Inline);
    let data = noise(512 * 1024, 4);
    fs.create("/a", Kind::File).unwrap();
    fs.write("/a", 0, &data).unwrap();
    fs.commit().unwrap();
    let one = used(&mut fs);
    fs.create("/b", Kind::File).unwrap();
    fs.write("/b", 0, &data).unwrap();
    // fsync se sdilenym extentem z teto transakce udela commit.
    fs.fsync().unwrap();
    assert!(used(&mut fs) - one < 32, "druha kopie se ulozila");
    assert!(fs.test_extents("/b").iter().all(|(_, e)| e.is_shared()));
    let mut fs = remount(fs);
    assert_eq!(fs.read("/b", 0, data.len()).unwrap(), data);
    // Prepis v teze transakci, kdy kus vznikl a zaindexoval se.
    fs.create("/c", Kind::File).unwrap();
    fs.write("/c", 0, &noise(256 * 1024, 99)).unwrap();
    fs.flush_for_test();
    fs.write("/c", 0, &vec![7u8; 256 * 1024]).unwrap();
    fs.commit().unwrap();
    assert_eq!(fs.read("/c", 0, 10).unwrap(), vec![7u8; 10]);
    assert_clean(&mut fs);
}

#[test]
fn dedup_across_datasets() {
    let mut fs = new_fs(16384);
    fs.create_dataset("/d").unwrap();
    let data = noise(256 * 1024, 31);
    fs.create("/a", Kind::File).unwrap();
    fs.write("/a", 0, &data).unwrap();
    fs.create("/d/a", Kind::File).unwrap();
    fs.write("/d/a", 0, &data).unwrap();
    fs.commit().unwrap();
    while !fs.dedup_step(64).unwrap().done {}
    fs.commit().unwrap();
    assert!(fs.test_extents("/d/a").iter().chain(fs.test_extents("/a").iter()).any(|(_, e)| e.is_shared()));
    fs.delete_dataset("/d").unwrap();
    fs.commit().unwrap();
    assert_eq!(fs.read("/a", 0, data.len()).unwrap(), data);
    assert_clean(&mut fs);
}
