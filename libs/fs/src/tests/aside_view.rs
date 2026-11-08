//! "Bokem" se snapshotem puvodu jako pohledem pro undo (`keep_view`).

use super::{assert_clean, new_fs, remount};
use crate::Kind;

fn put(fs: &mut crate::Fs<crate::MemDevice>, p: &str, data: &[u8]) {
    match fs.create(p, Kind::File) {
        Ok(()) | Err(abi::Error::AlreadyExists) => {}
        Err(e) => panic!("{p}: {e:?}"),
    }
    fs.truncate(p, 0).unwrap();
    fs.write(p, 0, data).unwrap();
}

fn setup() -> crate::Fs<crate::MemDevice> {
    let mut fs = new_fs(8192);
    fs.create("/tmp", Kind::Dir).unwrap();
    put(&mut fs, "/a.txt", b"pred\n");
    fs.commit().unwrap();
    fs
}

/// Po prevzeti zustane snapshot puvodu jako pohled se stavem pred radkem;
/// uvolneni pred i po zahozeni klonu.
#[test]
fn kept_view_holds_state_before_line() {
    for release_first in [false, true] {
        let mut fs = setup();
        let b = fs.aside_begin_ex("/", "/tmp/k").unwrap();
        assert!(b.snapshot.starts_with("aside-"));
        put(&mut fs, "/tmp/k/a.txt", b"po\n");
        let r = fs.aside_apply_with("/tmp/k", None, true).unwrap();
        assert!(r.conflicts.is_empty());
        assert_eq!(fs.read("/a.txt", 0, 10).unwrap(), b"po\n");
        // Pohled: stav pred radkem (undo z nej cte).
        assert_eq!(fs.snapshot_read_of("/", &b.snapshot, "/a.txt", 0, 10).unwrap(), b"pred\n");
        assert!(fs.journal_last("/").unwrap() >= b.seq);
        if release_first {
            fs.release_view("/", &b.snapshot).unwrap();
            fs.aside_discard("/tmp/k").unwrap();
        } else {
            fs.aside_discard("/tmp/k").unwrap();
            assert_eq!(fs.snapshot_read_of("/", &b.snapshot, "/a.txt", 0, 10).unwrap(), b"pred\n");
            fs.release_view("/", &b.snapshot).unwrap();
        }
        assert!(fs.snapshots_of("/").unwrap().is_empty());
        assert_eq!(fs.snapshot_read_of("/", &b.snapshot, "/a.txt", 0, 10).unwrap_err(), abi::Error::NotFound);
        assert_clean(&mut fs);
    }
}

/// Bez `keep_view` se snapshot smaze s klonem (jako driv).
#[test]
fn without_keep_view_snapshot_goes_with_clone() {
    let mut fs = setup();
    let b = fs.aside_begin_ex("/", "/tmp/k").unwrap();
    put(&mut fs, "/tmp/k/a.txt", b"po\n");
    fs.aside_apply("/tmp/k", None).unwrap();
    fs.aside_discard("/tmp/k").unwrap();
    assert_eq!(fs.snapshot_read_of("/", &b.snapshot, "/a.txt", 0, 10).unwrap_err(), abi::Error::NotFound);
    assert_clean(&mut fs);
}

/// Pohled po padu: pri pripojeni se smaze (jako ostatni pohledy), klon
/// zustava, dokud ho nekdo nezahodi.
#[test]
fn kept_view_cleaned_after_crash() {
    let mut fs = setup();
    let b = fs.aside_begin_ex("/", "/tmp/k").unwrap();
    put(&mut fs, "/tmp/k/a.txt", b"po\n");
    fs.aside_apply_with("/tmp/k", None, true).unwrap();
    let mut fs = remount(fs);
    assert!(fs.snapshots_of("/").unwrap().is_empty());
    fs.aside_discard("/tmp/k").unwrap();
    assert_eq!(fs.snapshot_read_of("/", &b.snapshot, "/a.txt", 0, 10).unwrap_err(), abi::Error::NotFound);
    assert_clean(&mut fs);
}

/// Zacatek "bokem" je jeden celek: pad hned po nem necha snapshot i klon,
/// nebo nic (zaznam snapshotu jde do commitu klonu).
#[test]
fn aside_begin_crash_leaves_both_or_nothing() {
    let mut fs = setup();
    fs.aside_begin_ex("/", "/tmp/k").unwrap();
    let mut fs = remount(fs);
    assert!(fs.stat("/tmp/k").is_ok());
    fs.aside_discard("/tmp/k").unwrap();
    assert_clean(&mut fs);
    // Klon nevznikl (cil existuje): snapshot se hned smaze.
    fs.create("/tmp/obsazeno", Kind::Dir).unwrap();
    assert_eq!(fs.aside_begin_ex("/", "/tmp/obsazeno").unwrap_err(), abi::Error::AlreadyExists);
    fs.commit().unwrap();
    let mut fs = remount(fs);
    assert_clean(&mut fs);
}
