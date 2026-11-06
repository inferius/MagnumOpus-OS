//! Zakladni scenare.

use alloc::vec;
use alloc::vec::Vec;

use super::{assert_clean, new_fs, remount};
use crate::{Error, Fs, Kind};

#[test]
fn empty_root() {
    let mut fs = new_fs(256);
    assert_eq!(fs.list("/").unwrap(), vec![]);
    let st = fs.stat("/").unwrap();
    assert_eq!(st.kind, Kind::Dir);
    assert_eq!(fs.info().label, "test");
}

#[test]
fn small_file_roundtrip_and_persistence() {
    let mut fs = new_fs(256);
    fs.create("/ahoj.txt", Kind::File).unwrap();
    fs.write("/ahoj.txt", 0, b"Prilis zlutoucky kun").unwrap();
    assert_eq!(
        fs.read("/ahoj.txt", 0, 100).unwrap(),
        b"Prilis zlutoucky kun"
    );
    fs.commit().unwrap();
    let mut fs = remount(fs);
    assert_eq!(fs.read("/ahoj.txt", 7, 5).unwrap(), b"zluto");
    assert_eq!(fs.stat("/ahoj.txt").unwrap().size, 20);
}

#[test]
fn uncommitted_changes_are_lost_on_remount() {
    let mut fs = new_fs(256);
    fs.create("/a", Kind::File).unwrap();
    fs.commit().unwrap();
    fs.create("/b", Kind::File).unwrap();
    let mut fs = remount(fs);
    assert!(fs.stat("/a").is_ok());
    assert_eq!(fs.stat("/b").err(), Some(Error::NotFound));
}

#[test]
fn large_file_and_conversions() {
    let mut fs = new_fs(1024);
    fs.create("/velky", Kind::File).unwrap();
    let data: Vec<u8> = (0..300_000u32).map(|i| (i * 31 % 251) as u8).collect();
    fs.write("/velky", 0, &data[..1000]).unwrap(); // inline
    fs.write("/velky", 1000, &data[1000..]).unwrap(); // prevod na bloky
    assert_eq!(fs.read("/velky", 0, data.len()).unwrap(), data);
    fs.commit().unwrap();
    let mut fs = remount(fs);
    assert_eq!(
        fs.read("/velky", 123_456, 70_000).unwrap(),
        &data[123_456..193_456]
    );
    // Zmenseni pod limit maleho souboru a zpet.
    fs.truncate("/velky", 500).unwrap();
    assert_eq!(fs.read("/velky", 0, 10_000).unwrap(), &data[..500]);
    fs.truncate("/velky", 9000).unwrap();
    let mut expect = data[..500].to_vec();
    expect.resize(9000, 0);
    assert_eq!(fs.read("/velky", 0, 10_000).unwrap(), expect);
}

#[test]
fn sparse_write_reads_zeros() {
    let mut fs = new_fs(512);
    fs.create("/ridky", Kind::File).unwrap();
    fs.write("/ridky", 200_000, b"konec").unwrap();
    let st = fs.stat("/ridky").unwrap();
    assert_eq!(st.size, 200_005);
    let mid = fs.read("/ridky", 100_000, 16).unwrap();
    assert_eq!(mid, vec![0u8; 16]);
    assert_eq!(fs.read("/ridky", 200_000, 5).unwrap(), b"konec");
}

#[test]
fn case_insensitive_names_preserve_case() {
    let mut fs = new_fs(256);
    fs.create("/Fotky", Kind::Dir).unwrap();
    fs.create("/fotky/Dovolená.JPG", Kind::File).unwrap();
    assert_eq!(
        fs.create("/FOTKY", Kind::Dir).err(),
        Some(Error::AlreadyExists)
    );
    assert!(fs.stat("/FOTKY/dovolená.jpg").is_ok());
    // Rozlozeny zapis (a + carka) najde stejny soubor.
    assert!(fs.stat("/fotky/dovolena\u{301}.jpg").is_ok());
    let names: Vec<_> = fs.list("/").unwrap().into_iter().map(|e| e.name).collect();
    assert_eq!(names, vec!["Fotky"]);
    // Zmena jen velikosti pismen prejmenovanim.
    fs.rename("/fotky", "/FOTKY").unwrap();
    assert_eq!(fs.list("/").unwrap()[0].name, "FOTKY");
}

#[test]
fn directory_summaries() {
    let mut fs = new_fs(1024);
    fs.create("/a", Kind::Dir).unwrap();
    fs.create("/a/b", Kind::Dir).unwrap();
    fs.create("/a/b/x", Kind::File).unwrap();
    fs.write("/a/b/x", 0, &vec![1u8; 10_000]).unwrap();
    fs.create("/a/y", Kind::File).unwrap();
    fs.write("/a/y", 0, b"12345").unwrap();
    let a = fs.stat("/a").unwrap();
    assert_eq!((a.tree_bytes, a.tree_files, a.tree_dirs), (10_005, 2, 1));
    let root = fs.stat("/").unwrap();
    assert_eq!(
        (root.tree_bytes, root.tree_files, root.tree_dirs),
        (10_005, 2, 2)
    );
    // Presun podstromu jinam.
    fs.create("/c", Kind::Dir).unwrap();
    fs.rename("/a/b", "/c/b").unwrap();
    let a = fs.stat("/a").unwrap();
    let c = fs.stat("/c").unwrap();
    assert_eq!((a.tree_bytes, a.tree_files, a.tree_dirs), (5, 1, 0));
    assert_eq!((c.tree_bytes, c.tree_files, c.tree_dirs), (10_000, 1, 1));
    fs.remove("/a/y").unwrap();
    assert_eq!(fs.stat("/a").unwrap().tree_files, 0);
    assert_eq!(fs.stat("/").unwrap().tree_bytes, 10_000);
}

#[test]
fn errors() {
    let mut fs = new_fs(256);
    fs.create("/d", Kind::Dir).unwrap();
    fs.create("/d/f", Kind::File).unwrap();
    assert_eq!(fs.remove("/d").err(), Some(Error::NotEmpty));
    assert_eq!(fs.list("/d/f").err(), Some(Error::NotADirectory));
    assert_eq!(fs.read("/d", 0, 1).err(), Some(Error::IsADirectory));
    assert_eq!(
        fs.create("/d/f/g", Kind::File).err(),
        Some(Error::NotADirectory)
    );
    assert_eq!(fs.stat("/nic").err(), Some(Error::NotFound));
    assert_eq!(
        fs.create("/a\u{0}b", Kind::File).err(),
        Some(Error::InvalidArgs)
    );
    assert_eq!(fs.rename("/d", "/d/f2").err(), Some(Error::InvalidArgs));
    assert_eq!(fs.stat("relativni").err(), Some(Error::InvalidArgs));
}

#[test]
fn many_entries_split_tree() {
    let mut fs = new_fs(4096);
    for i in 0..2000 {
        fs.create(&alloc::format!("/soubor-{i}"), Kind::File)
            .unwrap();
    }
    fs.commit().unwrap();
    let mut fs = remount(fs);
    assert_eq!(fs.list("/").unwrap().len(), 2000);
    for i in (0..2000).step_by(2) {
        fs.remove(&alloc::format!("/soubor-{i}")).unwrap();
    }
    fs.commit().unwrap();
    let mut fs = remount(fs);
    assert_eq!(fs.list("/").unwrap().len(), 1000);
    assert_eq!(fs.stat("/").unwrap().tree_files, 1000);
    assert_clean(&mut fs);
}

#[test]
fn space_is_reused_after_commit() {
    let mut fs = new_fs(2048);
    let free0 = fs.info().free_blocks;
    for round in 0..20 {
        fs.create("/x", Kind::File).unwrap();
        fs.write("/x", 0, &vec![round as u8; 400_000]).unwrap();
        fs.commit().unwrap();
        fs.remove("/x").unwrap();
        fs.commit().unwrap();
    }
    // Po smazani a commitu se vse uvolni (bez snapshotu nic neunika).
    let free = fs.info().free_blocks;
    assert!(free + 8 >= free0, "unik bloku: {free0} -> {free}");
    assert_clean(&mut fs);
}

#[test]
fn out_of_space() {
    let mut fs = new_fs(128);
    fs.create("/plny", Kind::File).unwrap();
    // Bez commitu: odlozena alokace musi odmitnout zapis hned, ne az pri commitu.
    let r = fs.write("/plny", 0, &vec![7u8; 1024 * 1024]);
    assert_eq!(r.err(), Some(Error::NoSpace));
}

#[test]
fn snapshots_keep_old_state() {
    let mut fs = new_fs(1024);
    fs.create("/dok", Kind::File).unwrap();
    fs.write("/dok", 0, &vec![b'A'; 50_000]).unwrap();
    fs.snapshot("pred-upravou").unwrap();
    fs.write("/dok", 0, &vec![b'B'; 50_000]).unwrap();
    fs.create("/novy", Kind::File).unwrap();
    fs.commit().unwrap();
    let mut fs = remount(fs);
    assert_eq!(fs.read("/dok", 0, 3).unwrap(), b"BBB");
    assert_eq!(
        fs.snapshot_read("pred-upravou", "/dok", 0, 3).unwrap(),
        b"AAA"
    );
    assert_eq!(
        fs.snapshot_stat("pred-upravou", "/novy").err(),
        Some(Error::NotFound)
    );
    assert_eq!(fs.snapshots().unwrap().len(), 1);
    assert_eq!(
        fs.snapshot("PRED-upravou").err(),
        Some(Error::AlreadyExists)
    );
    let names: Vec<_> = fs
        .snapshot_list("pred-upravou", "/")
        .unwrap()
        .into_iter()
        .map(|e| e.name)
        .collect();
    assert_eq!(names, vec!["dok"]);
    assert_clean(&mut fs);
}

#[test]
fn corrupted_block_is_detected() {
    let mut fs = new_fs(256);
    fs.create("/f", Kind::File).unwrap();
    fs.write("/f", 0, &vec![9u8; 20_000]).unwrap();
    fs.commit().unwrap();
    let mut dev = fs.into_device();
    // Prepsat jeden bajt v kazdem datovem bloku (bloky plne devitek).
    let bs = crate::BLOCK_SIZE;
    let mut hit = 0;
    for b in 0..256usize {
        let block = &mut dev.data[b * bs..(b + 1) * bs];
        if block.iter().all(|&x| x == 9) {
            block[100] = 8;
            hit += 1;
        }
    }
    assert!(hit >= 4, "datove bloky nenalezeny");
    let mut fs = Fs::mount(dev).unwrap();
    assert_eq!(fs.read("/f", 0, 20_000).err(), Some(Error::Corrupted));
    let r = fs.check(true).unwrap();
    assert!(!r.problems.is_empty(), "scrub musi poskozeni najit");
}
