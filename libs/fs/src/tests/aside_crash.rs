//! "Bokem" s vnorenym datasetem: vypadek proudu uprostred `aside_begin`,
//! `aside_apply` a `aside_discard` (po pripojeni klon cely, nebo zadny;
//! hranice dal plati) a kontrola konzistence se dvema soubeznymi klony
//! korene nad vetsim stromem (sdilene uzly klonu a snapshotu).

use abi::Error;
use proptest::prelude::*;

use super::crash::CrashDevice;
use super::{assert_clean, new_fs, opts, remount};
use crate::{BlockDevice, Fs, Kind, MemDevice};

fn put<D: BlockDevice>(fs: &mut Fs<D>, p: &str, d: &[u8]) {
    if fs.stat(p).is_err() {
        fs.create(p, Kind::File).unwrap();
    }
    fs.truncate(p, 0).unwrap();
    fs.write(p, 0, d).unwrap();
}

fn read<D: BlockDevice>(fs: &mut Fs<D>, p: &str) -> Vec<u8> {
    fs.read(p, 0, 1 << 20).unwrap()
}

/// Koren s vnorenym datasetem /repo, souborem /f.txt, slozkou /tmp a tolika
/// soubory, aby mel strom vic listu (klon pak sdili jen cast uzlu).
fn setup<D: BlockDevice>(fs: &mut Fs<D>, files: usize) {
    fs.create_dataset("/repo").unwrap();
    put(fs, "/repo/a.txt", b"repo\n");
    put(fs, "/f.txt", b"original\n");
    fs.create("/tmp", Kind::Dir).unwrap();
    fs.create("/d", Kind::Dir).unwrap();
    for i in 0..files {
        let p = alloc::format!("/d/soubor-{i}");
        fs.create(&p, Kind::File).unwrap();
        if i % 4 == 0 {
            fs.write(&p, 0, &alloc::vec![i as u8; 100 + i * 7]).unwrap();
        }
    }
}

/// Hranice /repo v klonu `c`: videt, dovnitr ani zapisem.
fn assert_boundary<D: BlockDevice>(fs: &mut Fs<D>, c: &str) {
    let p = alloc::format!("{c}/repo");
    assert_eq!(fs.stat(&p).unwrap().kind, Kind::Dataset);
    assert_eq!(fs.list(&p).unwrap_err(), Error::NotSupported);
    assert_eq!(fs.write(&alloc::format!("{p}/a.txt"), 0, b"x").unwrap_err(), Error::NotSupported);
    assert_eq!(fs.create(&alloc::format!("{p}/x"), Kind::File).unwrap_err(), Error::NotSupported);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Op {
    Begin,
    Apply,
    Discard,
}

const CLONE: &str = "/tmp/w";

fn crash_scenario(op: Op, fail_after: usize, keep: Vec<bool>) {
    let dev = Fs::format(MemDevice::new(8192), &opts("aside")).unwrap();
    let dev = CrashDevice { durable: dev, pending: Vec::new(), fail_after: None, writes: 0 };
    let mut fs = Fs::mount(dev).unwrap();
    setup(&mut fs, 300);
    if op != Op::Begin {
        fs.aside_begin("/", CLONE).unwrap();
        put(&mut fs, "/tmp/w/f.txt", b"zmeneno\n");
        put(&mut fs, "/tmp/w/novy.txt", b"novy\n");
    }
    fs.commit().unwrap();

    let mut dev = fs.into_device();
    dev.writes = 0;
    dev.fail_after = Some(fail_after);
    let mut fs = Fs::mount(dev).unwrap();
    let res = match op {
        Op::Begin => fs.aside_begin("/", CLONE).map(|_| ()),
        Op::Apply => fs.aside_apply(CLONE, None).map(|_| ()),
        Op::Discard => fs.aside_discard(CLONE),
    };

    // Vypadek: z neflushnutych zapisu prezije jen nahodna cast.
    let mut dev = fs.into_device();
    for (i, (b, d)) in core::mem::take(&mut dev.pending).into_iter().enumerate() {
        if keep.get(i).copied().unwrap_or(false) {
            dev.durable.write_block(b, &d).unwrap();
        }
    }
    let mut fs = Fs::mount(dev.durable).expect("system po vypadku nejde pripojit");
    let ctx = alloc::format!("{op:?}, fail_after = {fail_after}, vysledek {res:?}");
    assert_clean(&mut fs);
    // Klon cely (dataset, s hranici a svym obsahem), nebo zadny - a pak
    // ani jeho snapshot.
    let entry = fs.list("/tmp").unwrap().into_iter().find(|e| e.name == "w");
    let exists = match entry {
        Some(e) => {
            assert_eq!(e.kind, Kind::Dataset, "{ctx}");
            assert_boundary(&mut fs, CLONE);
            assert!(fs.stat("/tmp/w/d/soubor-299").is_ok(), "{ctx}");
            true
        }
        None => {
            assert_eq!(fs.stat(CLONE).unwrap_err(), Error::NotFound, "{ctx}");
            false
        }
    };
    assert_eq!(fs.snapshots_of("/").unwrap().len(), usize::from(exists), "{ctx}");
    assert_eq!(fs.datasets().unwrap().len(), 1 + usize::from(exists), "{ctx}");
    assert_eq!(read(&mut fs, "/repo/a.txt"), b"repo\n", "{ctx}");
    match op {
        Op::Begin => {
            if res.is_ok() {
                assert!(exists, "{ctx}");
            }
            if exists {
                assert_eq!(read(&mut fs, "/tmp/w/f.txt"), b"original\n", "{ctx}");
            }
        }
        Op::Apply => {
            assert!(exists, "{ctx}");
            // Prevzeti je atomicke: oba soubory, nebo zadny.
            let f = read(&mut fs, "/f.txt");
            let novy = fs.stat("/novy.txt").is_ok();
            if f == b"zmeneno\n" {
                assert!(novy, "{ctx}");
            } else {
                assert_eq!(f, b"original\n", "{ctx}");
                assert!(!novy && res.is_err(), "{ctx}");
            }
        }
        Op::Discard => {
            if res.is_ok() {
                assert!(!exists, "{ctx}");
            }
            assert_eq!(read(&mut fs, "/f.txt"), b"original\n", "{ctx}");
        }
    }
    // Po pripojeni jde "bokem" dal normalne pouzivat.
    if !exists {
        fs.aside_begin("/", CLONE).unwrap();
    }
    put(&mut fs, "/tmp/w/g.txt", b"g\n");
    let r = fs.aside_apply(CLONE, None).unwrap();
    assert!(r.conflicts.is_empty(), "{ctx}: {:?}", r.conflicts);
    fs.aside_discard(CLONE).unwrap();
    assert!(fs.snapshots_of("/").unwrap().is_empty(), "{ctx}");
    assert_eq!(read(&mut fs, "/g.txt"), b"g\n");
    assert_clean(&mut fs);
}

proptest! {
    #![proptest_config(ProptestConfig { cases: super::cases(32), ..ProptestConfig::default() })]

    #[test]
    fn crash_during_aside_begin(fail_after in 0usize..400, keep in proptest::collection::vec(any::<bool>(), 0..400)) {
        crash_scenario(Op::Begin, fail_after, keep);
    }

    #[test]
    fn crash_during_aside_apply(fail_after in 0usize..400, keep in proptest::collection::vec(any::<bool>(), 0..400)) {
        crash_scenario(Op::Apply, fail_after, keep);
    }

    #[test]
    fn crash_during_aside_discard(fail_after in 0usize..400, keep in proptest::collection::vec(any::<bool>(), 0..400)) {
        crash_scenario(Op::Discard, fail_after, keep);
    }
}

/// Pad presne mezi snapshotem a klonem (`aside_begin`) nebo mezi smazanim
/// klonu a snapshotu (`aside_discard`): zbyly snapshot uklidi pripojeni.
#[test]
fn leftover_aside_snapshot_is_cleaned_on_mount() {
    let mut fs = new_fs(16384);
    setup(&mut fs, 50);
    // Stav po padu v `aside_begin` za snapshotem.
    fs.aside_snapshot_only("/").unwrap();
    fs.commit().unwrap();
    assert_eq!(fs.snapshots_of("/").unwrap().len(), 1);
    let mut fs = remount(fs);
    assert!(fs.snapshots_of("/").unwrap().is_empty());
    assert_clean(&mut fs);
    // Stav po padu v `aside_discard` za smazanim klonu.
    fs.aside_begin("/", CLONE).unwrap();
    fs.delete_dataset(CLONE).unwrap();
    assert_eq!(fs.snapshots_of("/").unwrap().len(), 1);
    let mut fs = remount(fs);
    assert!(fs.snapshots_of("/").unwrap().is_empty());
    assert_clean(&mut fs);
    // Zivy klon "bokem" pripojeni nechava.
    fs.aside_begin("/", CLONE).unwrap();
    fs.commit().unwrap();
    let mut fs = remount(fs);
    assert_eq!(fs.snapshots_of("/").unwrap().len(), 1);
    assert_boundary(&mut fs, CLONE);
    assert_clean(&mut fs);
}

/// Dva soubezne klony korene s vnorenym datasetem nad stromem s mnoha
/// listy: kontrola konzistence po kazdem kroku (begin, zmeny, apply,
/// discard, znovupripojeni). Drive kontrola u klonu vynechala listy sdilene
/// se snapshotem a hlasila "neexistujici objekty".
#[test]
fn check_two_root_clones_with_nested_dataset() {
    let mut fs = new_fs(32768);
    setup(&mut fs, 600);
    assert_clean(&mut fs);
    fs.aside_begin("/", "/tmp/a").unwrap();
    assert_clean(&mut fs);
    fs.aside_begin("/", "/tmp/b").unwrap();
    assert_clean(&mut fs);
    assert_boundary(&mut fs, "/tmp/a");
    assert_boundary(&mut fs, "/tmp/b");
    put(&mut fs, "/tmp/a/f.txt", b"z A\n");
    put(&mut fs, "/tmp/a/d/soubor-5", b"z A\n");
    fs.remove("/tmp/a/d/soubor-7").unwrap();
    put(&mut fs, "/tmp/b/novy.txt", b"z B\n");
    fs.rename("/tmp/b/d/soubor-9", "/tmp/b/presunuty").unwrap();
    assert_clean(&mut fs);
    let r = fs.aside_apply("/tmp/a", None).unwrap();
    assert!(r.conflicts.is_empty(), "{:?}", r.conflicts);
    assert_clean(&mut fs);
    let r = fs.aside_apply("/tmp/b", None).unwrap();
    assert!(r.conflicts.is_empty(), "{:?}", r.conflicts);
    assert_eq!(read(&mut fs, "/f.txt"), b"z A\n");
    assert_eq!(read(&mut fs, "/novy.txt"), b"z B\n");
    assert!(fs.stat("/presunuty").is_ok());
    assert_eq!(fs.stat("/d/soubor-7").unwrap_err(), Error::NotFound);
    // Hranice ani druhy klon se neprenesly.
    assert_eq!(read(&mut fs, "/repo/a.txt"), b"repo\n");
    assert_eq!(fs.list("/tmp").unwrap().len(), 2);
    assert_clean(&mut fs);
    fs.aside_discard("/tmp/a").unwrap();
    assert_clean(&mut fs);
    let mut fs = remount(fs);
    assert_clean(&mut fs);
    assert_boundary(&mut fs, "/tmp/b");
    fs.aside_discard("/tmp/b").unwrap();
    assert!(fs.snapshots_of("/").unwrap().is_empty());
    assert_eq!(fs.datasets().unwrap().len(), 1);
    assert_clean(&mut fs);
    let mut fs = remount(fs);
    assert_clean(&mut fs);
}
