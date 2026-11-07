//! "Zkusit zmeny bokem", klony a vetve nad datasetem s vnorenymi datasety:
//! vnorene datasety jsou v klonu hranice (videt, ale nejde do nich).

use abi::Error;

use super::{assert_clean, new_fs, remount};
use crate::{Acl, ChangeKind, ConflictKind, EVERYONE, Fs, Grant, Kind, MemDevice, Principal, grant, rights};

fn put(fs: &mut Fs<MemDevice>, p: &str, d: &[u8]) {
    if fs.stat(p).is_err() {
        fs.create(p, Kind::File).unwrap();
    }
    fs.truncate(p, 0).unwrap();
    fs.write(p, 0, d).unwrap();
}

fn read(fs: &mut Fs<MemDevice>, p: &str) -> Vec<u8> {
    fs.read(p, 0, 1 << 20).unwrap()
}

/// Koren s vnorenym datasetem /repo, souborem /f a slozkou /tmp.
fn setup(fs: &mut Fs<MemDevice>) {
    fs.create_dataset("/repo").unwrap();
    put(fs, "/repo/a.txt", b"repo\n");
    put(fs, "/f.txt", b"original\n");
    fs.create("/tmp", Kind::Dir).unwrap();
}

/// Cesta vnoreneho datasetu `p` v klonu je hranice: videt, vsechno ostatni
/// `NOT_SUPPORTED`.
fn assert_boundary(fs: &mut Fs<MemDevice>, p: &str, clone_id: u64) {
    let (parent, name) = p.rsplit_once('/').unwrap();
    let parent = if parent.is_empty() { "/" } else { parent };
    let e = fs.list(parent).unwrap().into_iter().find(|e| e.name == name).unwrap();
    assert_eq!(e.kind, Kind::Dataset);
    let st = fs.stat(p).unwrap();
    assert_eq!(st.kind, Kind::Dataset);
    assert_eq!(st.rights, 0);
    assert_eq!(st.dataset, clone_id);
    let inner = alloc::format!("{p}/x");
    assert_eq!(fs.list(p).unwrap_err(), Error::NotSupported);
    assert_eq!(fs.stat(&inner).unwrap_err(), Error::NotSupported);
    assert_eq!(fs.create(&inner, Kind::File).unwrap_err(), Error::NotSupported);
    assert_eq!(fs.write(&inner, 0, b"x").unwrap_err(), Error::NotSupported);
    assert_eq!(fs.write(&alloc::format!("{p}/a.txt"), 0, b"x").unwrap_err(), Error::NotSupported);
    assert_eq!(fs.create_dataset(&inner).unwrap_err(), Error::NotSupported);
    assert_eq!(fs.rename(p, &alloc::format!("{parent}/jinam")).unwrap_err(), Error::NotSupported);
    assert_eq!(fs.remove(p).unwrap_err(), Error::NotSupported);
    assert_eq!(fs.trash(p).unwrap_err(), Error::NotSupported);
    assert_eq!(fs.delete_dataset(p).unwrap_err(), Error::NotSupported);
    assert_eq!(fs.aside_begin(p, "/jine-bokem").unwrap_err(), Error::NotSupported);
}

fn dataset_paths(fs: &mut Fs<MemDevice>) -> Vec<(u64, String)> {
    fs.datasets().unwrap().into_iter().map(|d| (d.id, d.path)).collect()
}

#[test]
fn aside_root_with_nested_dataset() {
    let mut fs = new_fs(16384);
    setup(&mut fs);
    let repo_id = fs.stat("/repo").unwrap().dataset;
    let id = fs.aside_begin("/", "/tmp/.aside-1").unwrap();
    assert_boundary(&mut fs, "/tmp/.aside-1/repo", id);
    // Obsah vnoreneho datasetu se nezmenil a jde do nej jen pres original.
    assert_eq!(read(&mut fs, "/repo/a.txt"), b"repo\n");
    assert_eq!(fs.stat("/repo").unwrap().dataset, repo_id);
    // Cesty datasetu podle originalu (klon hranici nehlasi).
    assert_eq!(dataset_paths(&mut fs), [(repo_id, "/repo".into()), (id, "/tmp/.aside-1".into())]);
    put(&mut fs, "/tmp/.aside-1/f.txt", b"zmeneno\n");
    put(&mut fs, "/tmp/.aside-1/novy.txt", b"novy\n");
    // Hranice prezije pripojeni znovu.
    fs.fsync().unwrap();
    let mut fs = remount(fs);
    assert_boundary(&mut fs, "/tmp/.aside-1/repo", id);
    let changes = fs.diff_clone("/tmp/.aside-1").unwrap();
    let paths: Vec<&str> = changes.iter().map(|c| c.path.as_str()).collect();
    assert_eq!(paths, ["/f.txt", "/novy.txt"]);
    assert_eq!(changes[0].kind, ChangeKind::Modified);
    let r = fs.aside_apply("/tmp/.aside-1", None).unwrap();
    assert!(r.conflicts.is_empty(), "{:?}", r.conflicts);
    assert_eq!(read(&mut fs, "/f.txt"), b"zmeneno\n");
    assert_eq!(read(&mut fs, "/novy.txt"), b"novy\n");
    // Vnoreny dataset ani klon samotny se neprenesl ani nezmenil.
    assert_eq!(read(&mut fs, "/repo/a.txt"), b"repo\n");
    assert_eq!(fs.list("/repo").unwrap().len(), 1);
    assert_eq!(fs.list("/tmp").unwrap().len(), 1);
    assert_eq!(dataset_paths(&mut fs), [(repo_id, "/repo".into()), (id, "/tmp/.aside-1".into())]);
    assert_clean(&mut fs);
    fs.aside_discard("/tmp/.aside-1").unwrap();
    assert_eq!(fs.stat("/tmp/.aside-1").unwrap_err(), Error::NotFound);
    assert!(fs.snapshots_of("/").unwrap().is_empty());
    assert_eq!(dataset_paths(&mut fs), [(repo_id, "/repo".into())]);
    assert_eq!(read(&mut fs, "/repo/a.txt"), b"repo\n");
    assert_clean(&mut fs);
    let mut fs = remount(fs);
    assert_clean(&mut fs);
}

/// Smazani slozky s vnorenym datasetem v klonu: v originalu konflikt,
/// dataset zustane.
#[test]
fn aside_dir_with_dataset_not_trashed() {
    let mut fs = new_fs(16384);
    fs.create("/d", Kind::Dir).unwrap();
    fs.create_dataset("/d/repo").unwrap();
    put(&mut fs, "/d/repo/a", b"a");
    fs.create("/e", Kind::Dir).unwrap();
    fs.aside_begin("/", "/e/w").unwrap();
    // Hranice slozku drzi neprazdnou i v klonu.
    assert_eq!(fs.remove("/e/w/d").unwrap_err(), Error::NotEmpty);
    // Do kose jde (hranice s ni), original se ale nezmeni.
    fs.trash("/e/w/d").unwrap();
    let r = fs.aside_apply("/e/w", None).unwrap();
    assert_eq!(r.conflicts.len(), 1, "{:?}", r.conflicts);
    assert_eq!(r.conflicts[0].kind, ConflictKind::DirNotEmpty);
    assert_eq!(read(&mut fs, "/d/repo/a"), b"a");
    fs.aside_discard("/e/w").unwrap();
    assert_clean(&mut fs);
}

/// Klon smi lezet uvnitr originalu; slozka s klonem smazana v klonu se
/// v originalu nesmaze (byl by s ni i klon).
#[test]
fn aside_clone_dir_not_trashed_by_own_apply() {
    let mut fs = new_fs(16384);
    setup(&mut fs);
    fs.aside_begin("/", "/tmp/.aside-1").unwrap();
    // V klonu je /tmp prazdna (klon vznikl az po snapshotu).
    assert!(fs.list("/tmp/.aside-1/tmp").unwrap().is_empty());
    fs.remove("/tmp/.aside-1/tmp").unwrap();
    let r = fs.aside_apply("/tmp/.aside-1", None).unwrap();
    assert_eq!(r.conflicts.len(), 1, "{:?}", r.conflicts);
    assert_eq!(r.conflicts[0].kind, ConflictKind::DirNotEmpty);
    assert!(fs.stat("/tmp/.aside-1/f.txt").is_ok());
    fs.aside_discard("/tmp/.aside-1").unwrap();
    assert!(fs.list("/tmp").unwrap().is_empty());
    assert_clean(&mut fs);
}

/// Novy soubor v klonu se jmenem, ktere v originalu mezitim zabral dataset
/// (klon jine relace): konflikt, ne chyba.
#[test]
fn aside_new_name_taken_by_dataset() {
    let mut fs = new_fs(16384);
    setup(&mut fs);
    fs.aside_begin("/", "/tmp/.aside-1").unwrap();
    put(&mut fs, "/tmp/.aside-1/tmp/.aside-2", b"soubor");
    fs.aside_begin("/", "/tmp/.aside-2").unwrap();
    let r = fs.aside_apply("/tmp/.aside-1", None).unwrap();
    assert_eq!(r.conflicts.len(), 1, "{:?}", r.conflicts);
    assert_eq!(r.conflicts[0].kind, ConflictKind::BothAdded);
    assert_eq!(fs.stat("/tmp/.aside-2").unwrap().dataset, fs.stat("/tmp/.aside-2/f.txt").unwrap().dataset);
    fs.aside_discard("/tmp/.aside-1").unwrap();
    fs.aside_discard("/tmp/.aside-2").unwrap();
    assert_clean(&mut fs);
}

/// Dva soucasne klony korene uvnitr korene (dve relace).
fn two_sessions(apply_first: bool) {
    let mut fs = new_fs(16384);
    setup(&mut fs);
    put(&mut fs, "/g.txt", b"g\n");
    let a = fs.aside_begin("/", "/tmp/.aside-1").unwrap();
    let b = fs.aside_begin("/", "/tmp/.aside-2").unwrap();
    // A druhy klon nevidi (vznikl az po nem), B vidi A jako hranici.
    assert_eq!(fs.stat("/tmp/.aside-1/tmp/.aside-2").unwrap_err(), Error::NotFound);
    assert_boundary(&mut fs, "/tmp/.aside-2/tmp/.aside-1", b);
    assert_boundary(&mut fs, "/tmp/.aside-2/repo", b);
    assert_boundary(&mut fs, "/tmp/.aside-1/repo", a);
    put(&mut fs, "/tmp/.aside-1/f.txt", b"z A\n");
    put(&mut fs, "/tmp/.aside-2/g.txt", b"z B\n");
    put(&mut fs, "/tmp/.aside-2/b.txt", b"novy z B\n");
    // Zmeny klonu nehlasi hranice (vnoreny dataset ani druhy klon).
    let paths = |fs: &mut Fs<MemDevice>, c: &str| -> Vec<String> {
        fs.diff_clone(c).unwrap().into_iter().map(|c| c.path).collect()
    };
    assert_eq!(paths(&mut fs, "/tmp/.aside-1"), ["/f.txt"]);
    assert_eq!(paths(&mut fs, "/tmp/.aside-2"), ["/b.txt", "/g.txt"]);
    let (first, second) = if apply_first { ("/tmp/.aside-1", "/tmp/.aside-2") } else { ("/tmp/.aside-2", "/tmp/.aside-1") };
    let r = fs.aside_apply(first, None).unwrap();
    assert!(r.conflicts.is_empty(), "{:?}", r.conflicts);
    // Druhy klon zustal cely a funkcni.
    let other = if apply_first { b } else { a };
    assert_eq!(fs.stat(second).unwrap().dataset, other);
    assert!(fs.list("/tmp").unwrap().iter().all(|e| e.kind == Kind::Dataset));
    assert_eq!(fs.list("/tmp").unwrap().len(), 2);
    fs.aside_discard(second).unwrap();
    // Druhy klon zmizel, prvni zustal; zmeny jen z prvniho.
    assert_eq!(fs.stat(second).unwrap_err(), Error::NotFound);
    assert!(fs.stat(first).is_ok());
    if apply_first {
        assert_eq!(read(&mut fs, "/f.txt"), b"z A\n");
        assert_eq!(read(&mut fs, "/g.txt"), b"g\n");
        assert_eq!(fs.stat("/b.txt").unwrap_err(), Error::NotFound);
    } else {
        assert_eq!(read(&mut fs, "/f.txt"), b"original\n");
        assert_eq!(read(&mut fs, "/g.txt"), b"z B\n");
        assert_eq!(read(&mut fs, "/b.txt"), b"novy z B\n");
    }
    assert_eq!(read(&mut fs, "/repo/a.txt"), b"repo\n");
    assert_clean(&mut fs);
    // Prvni klon jde dal pouzivat: dalsi zmena, apply, discard.
    put(&mut fs, &alloc::format!("{first}/h.txt"), b"h\n");
    let r = fs.aside_apply(first, Some(&[String::from("/h.txt")])).unwrap();
    assert!(r.conflicts.is_empty(), "{:?}", r.conflicts);
    assert_eq!(read(&mut fs, "/h.txt"), b"h\n");
    // Zaniknuty klon v B (pri apply_first = false) je v A hranice na
    // neexistujici dataset - stale videt, porad nepristupny.
    if !apply_first {
        let st = fs.stat("/tmp/.aside-2/tmp/.aside-1").unwrap();
        assert_eq!(st.kind, Kind::Dataset);
        // Novy klon muze dostat cislo zanikleho; stara polozka v B k nemu
        // nevede.
        let c = fs.aside_begin("/", "/tmp/.aside-1").unwrap();
        assert_boundary(&mut fs, "/tmp/.aside-2/tmp/.aside-1", b);
        assert!(fs.stat("/tmp/.aside-1/f.txt").is_ok());
        assert_eq!(fs.stat("/tmp/.aside-1").unwrap().dataset, c);
        fs.aside_discard("/tmp/.aside-1").unwrap();
    }
    fs.aside_discard(first).unwrap();
    assert!(fs.list("/tmp").unwrap().is_empty());
    assert!(fs.snapshots_of("/").unwrap().is_empty());
    assert_eq!(fs.datasets().unwrap().len(), 1);
    assert_clean(&mut fs);
    let mut fs = remount(fs);
    assert_clean(&mut fs);
}

#[test]
fn two_sessions_apply_first_discard_second() {
    two_sessions(true);
}

#[test]
fn two_sessions_apply_second_discard_first() {
    two_sessions(false);
}

/// `clone_snapshot` korene s vnorenym datasetem: hranice, smazani klonu.
#[test]
fn clone_snapshot_with_nested_dataset() {
    let mut fs = new_fs(16384);
    setup(&mut fs);
    fs.snapshot("s0").unwrap();
    fs.snapshot("s").unwrap();
    let id = fs.clone_snapshot("/", "s", "/klon").unwrap();
    assert_boundary(&mut fs, "/klon/repo", id);
    // Starsi snapshoty puvodu klon nezdedil.
    assert!(fs.snapshots_of("/klon").unwrap().is_empty());
    assert_eq!(fs.snapshots_of("/").unwrap().len(), 2);
    put(&mut fs, "/klon/f.txt", b"v klonu\n");
    assert_eq!(read(&mut fs, "/f.txt"), b"original\n");
    fs.delete_dataset("/klon").unwrap();
    fs.delete_snapshot_of("/", "s").unwrap();
    assert_eq!(read(&mut fs, "/repo/a.txt"), b"repo\n");
    assert_clean(&mut fs);
}

/// Vetev korene s vnorenym datasetem: hranice ve vetvi, merge dataset
/// nemeni.
#[test]
fn branch_with_nested_dataset() {
    let mut fs = new_fs(16384);
    setup(&mut fs);
    fs.commit_version("/", "zaklad").unwrap();
    let id = fs.branch("/", "v", "/vetev").unwrap();
    assert_boundary(&mut fs, "/vetev/repo", id);
    put(&mut fs, "/vetev/f.txt", b"z vetve\n");
    fs.commit_version("/vetev", "zmena").unwrap();
    let r = fs.merge("/", "/vetev").unwrap();
    assert!(r.conflicts.is_empty(), "{:?}", r.conflicts);
    assert_eq!(read(&mut fs, "/f.txt"), b"z vetve\n");
    assert_eq!(read(&mut fs, "/repo/a.txt"), b"repo\n");
    fs.delete_branch("/", "v").unwrap();
    assert_eq!(fs.stat("/vetev").unwrap_err(), Error::NotFound);
    assert_clean(&mut fs);
}

/// S kontrolou opravneni (pruchod `walk`): hranice stejne jako bez nich.
#[test]
fn boundary_with_permission_checks() {
    let mut fs = new_fs(16384);
    setup(&mut fs);
    let all = Some(Acl { flags: 0, grants: alloc::vec![Grant { who: EVERYONE, rights: rights::ALL, flags: grant::BOTH }] });
    fs.set_acl("/", all.clone()).unwrap();
    fs.set_acl("/repo", all).unwrap();
    let id = fs.aside_begin("/", "/tmp/.aside-1").unwrap();
    fs.set_principal(Some(Principal::user(1000, &[])));
    assert!(fs.list("/tmp/.aside-1").unwrap().iter().any(|e| e.name == "repo" && e.kind == Kind::Dataset));
    assert_eq!(fs.stat("/tmp/.aside-1/repo").unwrap().kind, Kind::Dataset);
    assert_eq!(fs.stat("/tmp/.aside-1/repo").unwrap().dataset, id);
    assert_eq!(fs.read("/tmp/.aside-1/repo/a.txt", 0, 10).unwrap_err(), Error::NotSupported);
    assert_eq!(fs.create("/tmp/.aside-1/repo/x", Kind::File).unwrap_err(), Error::NotSupported);
    assert_eq!(fs.remove("/tmp/.aside-1/repo").unwrap_err(), Error::NotSupported);
    assert_eq!(read(&mut fs, "/repo/a.txt"), b"repo
");
    assert_eq!(fs.datasets().unwrap().len(), 2);
    fs.set_principal(None);
    fs.aside_discard("/tmp/.aside-1").unwrap();
    assert_clean(&mut fs);
}
