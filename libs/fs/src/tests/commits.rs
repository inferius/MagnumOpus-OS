//! Verzovane commity, logicke ID, vetve, merge a "zkusit zmeny bokem".

use abi::Error;

use super::{assert_clean, new_fs, remount};
use crate::layout::Timestamp;
use crate::{ConflictKind, Fs, Kind, MemDevice, SetTimes, snapkind};

fn t(secs: i64) -> Timestamp {
    Timestamp { secs, nanos: 0 }
}

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

/// Projekt: README, src/main.rs, data.bin.
fn project(fs: &mut Fs<MemDevice>) {
    fs.create_dataset("/repo").unwrap();
    put(fs, "/repo/README.md", b"# Projekt\n\nPopis.\n");
    fs.create("/repo/src", Kind::Dir).unwrap();
    put(fs, "/repo/src/main.rs", b"fn main() {\n    println!(\"ahoj\");\n}\n");
    put(fs, "/repo/data.bin", &[0u8, 1, 2, 3, 0, 255]);
}

#[test]
fn commit_and_log() {
    let mut fs = new_fs(16384);
    fs.set_time(t(100));
    project(&mut fs);
    assert!(fs.head("/repo").unwrap().is_none());
    let c1 = fs.commit_version("/repo", "prvni").unwrap();
    assert!(c1.parents.is_empty());
    fs.set_time(t(200));
    put(&mut fs, "/repo/README.md", b"# Projekt 2\n");
    let c2 = fs.commit_version("/repo", "druhy").unwrap();
    assert_eq!(c2.parents, [c1.id]);
    assert_ne!(c1.id, c2.id);
    let log = fs.log("/repo", 10).unwrap();
    assert_eq!(log.iter().map(|c| c.message.as_str()).collect::<Vec<_>>(), ["druhy", "prvni"]);
    assert_eq!(fs.head("/repo").unwrap().unwrap().id, c2.id);
    // Commit je snapshot druhu COMMIT (cist jako kazdy snapshot).
    let s = fs.snapshots_of("/repo").unwrap();
    assert!(s.iter().all(|x| x.kind == snapkind::COMMIT));
    assert_eq!(fs.snapshot_read_of("/repo", &c1.snapshot, "/README.md", 0, 100).unwrap(), b"# Projekt\n\nPopis.\n");
    // Prezije remount.
    let mut fs = remount(fs);
    assert_eq!(fs.head("/repo").unwrap().unwrap().id, c2.id);
    assert_clean(&mut fs);
}

/// Stejny logicky strom na jinem svazku (jine rozmisteni bloku, jine oid)
/// ma stejne logicke ID - akceptacni test navrhu.
#[test]
fn logical_id_is_layout_independent() {
    let build = |noise: bool| {
        let mut fs = new_fs(16384);
        fs.set_time(t(1000));
        if noise {
            // Jine rozmisteni: soubory navic, pak smazane; jine poradi.
            for i in 0..20 {
                put(&mut fs, &alloc::format!("/sum{i}"), &[i as u8; 9000]);
            }
            fs.commit().unwrap();
            for i in 0..20 {
                fs.remove(&alloc::format!("/sum{i}")).unwrap();
            }
        }
        fs.create_dataset("/repo").unwrap();
        fs.create("/repo/src", Kind::Dir).unwrap();
        let files: [(&str, &[u8]); 3] = [("/repo/src/a.rs", b"fn a() {}\n"), ("/repo/b.txt", &[5u8; 30_000]), ("/repo/c", b"c")];
        let order: alloc::vec::Vec<usize> = if noise { alloc::vec![2, 0, 1] } else { alloc::vec![0, 1, 2] };
        for i in order {
            put(&mut fs, files[i].0, files[i].1);
        }
        fs.set_xattr("/repo/c", "user.tag", b"x").unwrap();
        // Casy nastavit stejne (jako by je prenesl send/receive).
        for p in ["/repo/src/a.rs", "/repo/b.txt", "/repo/c", "/repo/src", "/repo"] {
            fs.set_times(p, SetTimes { mtime: Some(t(5)), btime: Some(t(4)), atime: None }).unwrap();
        }
        fs
    };
    let mut a = build(false);
    let mut b = build(true);
    assert_ne!(a.stat("/repo/c").unwrap().oid, b.stat("/repo/c").unwrap().oid);
    let ia = a.tree_id("/repo").unwrap();
    assert_eq!(ia, b.tree_id("/repo").unwrap());
    let ca = a.commit_version("/repo", "import").unwrap();
    let cb = b.commit_version("/repo", "import").unwrap();
    assert_eq!(ca.id, cb.id);
    // Zmena obsahu nebo atributu ID zmeni; pamet hashe se obnovi.
    put(&mut b, "/repo/c", b"C");
    b.set_times("/repo/c", SetTimes { mtime: Some(t(5)), btime: None, atime: None }).unwrap();
    assert_ne!(b.tree_id("/repo").unwrap(), ia);
    put(&mut b, "/repo/c", b"c");
    b.set_times("/repo/c", SetTimes { mtime: Some(t(5)), btime: None, atime: None }).unwrap();
    assert_eq!(b.tree_id("/repo").unwrap(), ia);
    b.remove_xattr("/repo/c", "user.tag").unwrap();
    assert_ne!(b.tree_id("/repo").unwrap(), ia);
    assert_clean(&mut a);
    assert_clean(&mut b);
}

#[test]
fn branch_and_clean_merge() {
    let mut fs = new_fs(16384);
    project(&mut fs);
    let c1 = fs.commit_version("/repo", "zaklad").unwrap();
    fs.branch("/repo", "pokus", "/pokus").unwrap();
    assert_eq!(fs.branches("/repo").unwrap()[0].path, "/pokus");
    assert_eq!(fs.head("/pokus").unwrap().unwrap().id, c1.id);
    // Vetev: novy soubor, zmena README na konci, presun main.rs.
    put(&mut fs, "/pokus/NOVY.md", b"novy\n");
    put(&mut fs, "/pokus/README.md", b"# Projekt\n\nPopis.\nDalsi radek.\n");
    fs.rename("/pokus/src/main.rs", "/pokus/src/app.rs").unwrap();
    fs.remove("/pokus/data.bin").unwrap();
    let cb = fs.commit_version("/pokus", "pokus").unwrap();
    assert_eq!(cb.parents, [c1.id]);
    // Hlavni: zmena zacatku README.
    put(&mut fs, "/repo/README.md", b"# Projekt X\n\nPopis.\n");
    let cm = fs.commit_version("/repo", "hlavni").unwrap();
    let r = fs.merge("/repo", "/pokus").unwrap();
    assert!(r.conflicts.is_empty(), "{:?}", r.conflicts);
    let mc = r.commit.unwrap();
    assert_eq!(mc.parents, [cm.id, cb.id]);
    assert_eq!(read(&mut fs, "/repo/README.md"), b"# Projekt X\n\nPopis.\nDalsi radek.\n");
    assert_eq!(read(&mut fs, "/repo/NOVY.md"), b"novy\n");
    assert!(fs.stat("/repo/src/app.rs").is_ok());
    assert_eq!(fs.stat("/repo/src/main.rs").unwrap_err(), Error::NotFound);
    // Smazany soubor je v kosi (ne trvale).
    assert!(fs.trash_list("/repo").unwrap().iter().any(|e| e.name == "data.bin"));
    // Podruhe uz neni co slucovat.
    assert!(fs.merge("/repo", "/pokus").unwrap().applied.is_empty());
    assert_clean(&mut fs);
}

#[test]
fn merge_conflicts_text_and_binary() {
    let mut fs = new_fs(16384);
    project(&mut fs);
    fs.commit_version("/repo", "zaklad").unwrap();
    fs.branch("/repo", "b", "/b").unwrap();
    put(&mut fs, "/b/README.md", b"# Vetev\n\nPopis.\n");
    put(&mut fs, "/b/data.bin", &[9u8, 0, 9]);
    fs.commit_version("/b", "b").unwrap();
    put(&mut fs, "/repo/README.md", b"# Hlavni\n\nPopis.\n");
    put(&mut fs, "/repo/data.bin", &[7u8, 0, 7]);
    fs.commit_version("/repo", "hlavni").unwrap();
    let r = fs.merge("/repo", "/b").unwrap();
    assert!(r.commit.is_none());
    let kinds: Vec<(ConflictKind, &str)> = r.conflicts.iter().map(|c| (c.kind, c.path.as_str())).collect();
    assert!(kinds.contains(&(ConflictKind::Content, "/README.md")), "{kinds:?}");
    assert!(kinds.contains(&(ConflictKind::Content, "/data.bin")), "{kinds:?}");
    assert_eq!(read(&mut fs, "/repo/README.md"), b"<<<<<<< nase\n# Hlavni\n=======\n# Vetev\n>>>>>>> b\n\nPopis.\n");
    // Binarni: nase zustala, jejich vedle.
    assert_eq!(read(&mut fs, "/repo/data.bin"), [7u8, 0, 7]);
    assert_eq!(read(&mut fs, "/repo/data (konflikt b).bin"), [9u8, 0, 9]);
    // Po vyreseni commit se dvema rodici.
    put(&mut fs, "/repo/README.md", b"# Oboji\n\nPopis.\n");
    let c = fs.commit_version("/repo", "vyreseno").unwrap();
    assert_eq!(c.parents.len(), 2);
    assert_clean(&mut fs);
}

#[test]
fn aside_apply_select_and_conflict() {
    let mut fs = new_fs(16384);
    project(&mut fs);
    fs.aside_begin("/repo", "/repo-bokem").unwrap();
    put(&mut fs, "/repo-bokem/README.md", b"# Bokem\n");
    put(&mut fs, "/repo-bokem/src/lib.rs", b"pub fn x() {}\n");
    put(&mut fs, "/repo-bokem/data.bin", &[1u8, 1]);
    fs.remove("/repo-bokem/src/main.rs").unwrap();
    // Original mezitim zmenil data.bin.
    put(&mut fs, "/repo/data.bin", &[2u8, 2]);
    let changes = fs.diff_clone("/repo-bokem").unwrap();
    assert_eq!(changes.len(), 4);
    // Jen vybrane: src (novy lib.rs, smazany main.rs).
    let sel = [alloc::string::String::from("/src")];
    let r = fs.aside_apply("/repo-bokem", Some(&sel)).unwrap();
    assert!(r.conflicts.is_empty(), "{:?}", r.conflicts);
    assert_eq!(read(&mut fs, "/repo/src/lib.rs"), b"pub fn x() {}\n");
    assert_eq!(fs.stat("/repo/src/main.rs").unwrap_err(), Error::NotFound);
    assert_eq!(read(&mut fs, "/repo/README.md"), b"# Projekt\n\nPopis.\n");
    // Zbytek: README se prevezme, data.bin je konflikt a zustane.
    let r = fs.aside_apply("/repo-bokem", None).unwrap();
    assert_eq!(r.conflicts.len(), 1, "{:?}", r.conflicts);
    assert_eq!(r.conflicts[0].path, "/data.bin");
    assert_eq!(read(&mut fs, "/repo/README.md"), b"# Bokem\n");
    assert_eq!(read(&mut fs, "/repo/data.bin"), [2u8, 2]);
    // Zahodit jednim krokem: klon i jeho snapshot.
    fs.aside_discard("/repo-bokem").unwrap();
    assert_eq!(fs.stat("/repo-bokem").unwrap_err(), Error::NotFound);
    assert!(fs.snapshots_of("/repo").unwrap().is_empty());
    assert_clean(&mut fs);
}

#[test]
fn aside_new_dirs_and_moves() {
    let mut fs = new_fs(16384);
    project(&mut fs);
    fs.aside_begin("/repo", "/w").unwrap();
    fs.create("/w/docs", Kind::Dir).unwrap();
    fs.create("/w/docs/api", Kind::Dir).unwrap();
    put(&mut fs, "/w/docs/api/index.md", b"api\n");
    fs.rename("/w/README.md", "/w/docs/README.md").unwrap();
    // Vymena jmen pres pomocnou slozku.
    fs.rename("/w/src", "/w/kod").unwrap();
    let r = fs.aside_apply("/w", None).unwrap();
    assert!(r.conflicts.is_empty(), "{:?}", r.conflicts);
    assert_eq!(read(&mut fs, "/repo/docs/api/index.md"), b"api\n");
    assert_eq!(read(&mut fs, "/repo/docs/README.md"), b"# Projekt\n\nPopis.\n");
    assert!(fs.stat("/repo/kod/main.rs").is_ok());
    assert!(fs.list("/repo").unwrap().iter().all(|e| !e.name.starts_with(".slucovani")));
    assert_clean(&mut fs);
}

#[test]
fn merge_requires_commits() {
    let mut fs = new_fs(8192);
    project(&mut fs);
    assert_eq!(fs.branch("/repo", "x", "/x").unwrap_err(), Error::BadState);
    fs.commit_version("/repo", "a").unwrap();
    fs.branch("/repo", "x", "/x").unwrap();
    assert_eq!(fs.branch("/repo", "X", "/y").unwrap_err(), Error::AlreadyExists);
    fs.delete_branch("/repo", "x").unwrap();
    assert!(fs.branches("/repo").unwrap().is_empty());
    assert_eq!(fs.stat("/x").unwrap_err(), Error::NotFound);
    assert_clean(&mut fs);
}

/// Stejna sekvence jako systemovy test (init `fs_hist.rs`) nad korenovym
/// datasetem: vetev a merge korene, zrusit vetev, "bokem" korene, fsck.
#[test]
fn root_dataset_branch_merge_aside() {
    let mut fs = new_fs(16384);
    put(&mut fs, "/vitejte.txt", b"ahoj\n");
    let v = fs.view("/").unwrap();
    fs.release_view("/", &v.snapshot).unwrap();
    fs.create("/f2-kos", Kind::Dir).unwrap();
    put(&mut fs, "/f2-kos/smazat.txt", b"x");
    let id = fs.trash("/f2-kos").unwrap();
    fs.trash_purge("/", id).unwrap();
    put(&mut fs, "/f2-repo.txt", b"radek 1\nradek 2\n");
    let c1 = fs.commit_version("/", "zaklad").unwrap();
    fs.branch("/", "f2-vetev", "/f2-vetev").unwrap();
    put(&mut fs, "/f2-vetev/f2-repo.txt", b"radek 1\nradek 2\nradek 3\n");
    fs.create("/f2-vetev/f2-novy.txt", Kind::File).unwrap();
    let cb = fs.commit_version("/f2-vetev", "vetev").unwrap();
    assert_eq!(cb.parents, [c1.id]);
    let r = fs.merge("/", "/f2-vetev").unwrap();
    assert!(r.conflicts.is_empty(), "{:?}", r.conflicts);
    assert_clean(&mut fs);
    fs.delete_branch("/", "f2-vetev").unwrap();
    assert_clean(&mut fs);
    put(&mut fs, "/f2-bokem.txt", b"original");
    fs.aside_begin("/", "/f2-pokus").unwrap();
    put(&mut fs, "/f2-pokus/f2-bokem.txt", b"ZMENENO");
    let sel = [alloc::string::String::from("/f2-bokem.txt")];
    fs.aside_apply("/f2-pokus", Some(&sel)).unwrap();
    fs.aside_discard("/f2-pokus").unwrap();
    assert_clean(&mut fs);
    let mut fs = remount(fs);
    assert_clean(&mut fs);
}

/// Cesta vetve (datasetu) podle zpetneho odkazu: vnoreni, presun, pad.
#[test]
fn branch_path_follows_moves() {
    let mut fs = new_fs(16384);
    fs.create("/a", Kind::Dir).unwrap();
    fs.create_dataset("/a/repo").unwrap();
    put(&mut fs, "/a/repo/f", b"x");
    fs.commit_version("/a/repo", "c").unwrap();
    fs.create("/w", Kind::Dir).unwrap();
    fs.branch("/a/repo", "b", "/w/b").unwrap();
    assert_eq!(fs.branches("/a/repo").unwrap()[0].path, "/w/b");
    fs.create("/w/sub", Kind::Dir).unwrap();
    fs.rename("/w/b", "/w/sub/b2").unwrap();
    assert_eq!(fs.branches("/a/repo").unwrap()[0].path, "/w/sub/b2");
    fs.fsync().unwrap();
    let mut fs = remount(fs);
    assert_eq!(fs.branches("/a/repo").unwrap()[0].path, "/w/sub/b2");
    fs.rename("/a", "/c").unwrap();
    fs.rename("/w/sub", "/w/s").unwrap();
    assert_eq!(fs.branches("/c/repo").unwrap()[0].path, "/w/s/b2");
    assert_clean(&mut fs);
}
