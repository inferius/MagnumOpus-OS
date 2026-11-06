//! Prvni ukazka verzovane projektove slozky: snapshot, zapisovatelny klon,
//! zmeny, rozdil, obnova jednoho souboru jako kopie - a totez s padem
//! uprostred zapisu.

use abi::Error;

use super::{assert_clean, new_fs, remount};
use crate::{ChangeKind, Kind};

fn project(fs: &mut crate::Fs<crate::MemDevice>) {
    fs.create_dataset("/projekt").unwrap();
    for (p, d) in [("/projekt/README.md", "# Projekt\n"), ("/projekt/cfg.toml", "a = 1\n"), ("/projekt/stary.txt", "pryc\n")] {
        fs.create(p, Kind::File).unwrap();
        fs.write(p, 0, d.as_bytes()).unwrap();
    }
    fs.create("/projekt/src", Kind::Dir).unwrap();
    fs.create("/projekt/src/main.rs", Kind::File).unwrap();
    fs.write("/projekt/src/main.rs", 0, b"fn main() {}\n").unwrap();
    fs.snapshot_of("/projekt", "v1").unwrap();
}

fn kinds(changes: &[crate::Change]) -> Vec<(ChangeKind, String)> {
    changes.iter().map(|c| (c.kind, c.path.clone())).collect()
}

#[test]
fn diff_snapshot_against_live() {
    let mut fs = new_fs(8192);
    project(&mut fs);
    fs.write("/projekt/cfg.toml", 0, b"a = 2\n").unwrap();
    fs.remove("/projekt/stary.txt").unwrap();
    fs.create("/projekt/novy.txt", Kind::File).unwrap();
    fs.rename("/projekt/src/main.rs", "/projekt/src/app.rs").unwrap();
    let d = fs.diff("/projekt", "v1", None).unwrap();
    assert_eq!(
        kinds(&d),
        alloc::vec![
            (ChangeKind::Modified, "/cfg.toml".into()),
            (ChangeKind::Added, "/novy.txt".into()),
            (ChangeKind::Moved, "/src/app.rs".into()),
            (ChangeKind::Removed, "/stary.txt".into()),
        ]
    );
    assert_eq!(d[2].old_path.as_deref(), Some("/src/main.rs"));
    // Snapshot proti snapshotu dava totez.
    fs.snapshot_of("/projekt", "v2").unwrap();
    assert_eq!(kinds(&fs.diff("/projekt", "v1", Some("v2")).unwrap()), kinds(&d));
    assert!(fs.diff("/projekt", "v2", None).unwrap().is_empty());
}

#[test]
fn versioned_project_demo() {
    // Snapshot, zapisovatelny klon, zmeny v klonu, rozdil, obnova jednoho
    // souboru jako kopie vedle originalu.
    let mut fs = new_fs(8192);
    project(&mut fs);
    fs.clone_snapshot("/projekt", "v1", "/projekt-bokem").unwrap();
    fs.write("/projekt-bokem/cfg.toml", 0, b"a = 3\n").unwrap();
    fs.create("/projekt-bokem/src/lib.rs", Kind::File).unwrap();
    let d = fs.diff_clone("/projekt-bokem").unwrap();
    assert_eq!(
        kinds(&d),
        alloc::vec![(ChangeKind::Modified, "/cfg.toml".into()), (ChangeKind::Added, "/src/lib.rs".into())]
    );
    // Originál se klonem nezmenil.
    assert_eq!(fs.read("/projekt/cfg.toml", 0, 10).unwrap(), b"a = 1\n");
    // Obnova jednoho souboru ze snapshotu jako kopie do klonu.
    fs.restore_copy("/projekt", "v1", "/cfg.toml", "/projekt-bokem/cfg (v1).toml").unwrap();
    assert_eq!(fs.read("/projekt-bokem/cfg (v1).toml", 0, 10).unwrap(), b"a = 1\n");
    assert_eq!(fs.read("/projekt-bokem/cfg.toml", 0, 10).unwrap(), b"a = 3\n");
    // Obnova cele slozky.
    fs.restore_copy("/projekt", "v1", "/src", "/src-v1").unwrap();
    assert_eq!(fs.read("/src-v1/main.rs", 0, 100).unwrap(), b"fn main() {}\n");
    assert_eq!(
        fs.restore_copy("/projekt", "v1", "/cfg.toml", "/src-v1/main.rs").err(),
        Some(Error::AlreadyExists)
    );
    fs.commit().unwrap();
    let mut fs = remount(fs);
    assert_eq!(fs.diff_clone("/projekt-bokem").unwrap().len(), 3);
    assert_clean(&mut fs);
}

#[test]
fn demo_with_crash_in_the_middle_of_write() {
    let mut fs = new_fs(8192);
    project(&mut fs);
    fs.clone_snapshot("/projekt", "v1", "/bokem").unwrap();
    // Potvrzena zmena (fsync) a pak nepotvrzeny zapis - pad.
    fs.write("/bokem/cfg.toml", 0, b"a = 9\n").unwrap();
    fs.fsync().unwrap();
    fs.create("/bokem/rozdelany.txt", Kind::File).unwrap();
    fs.write("/bokem/rozdelany.txt", 0, &[1u8; 300_000]).unwrap();
    let mut fs = remount(fs);
    // Potvrzene prezilo, nepotvrzene zmizelo cele; diff i obnova funguji.
    assert_eq!(fs.read("/bokem/cfg.toml", 0, 10).unwrap(), b"a = 9\n");
    assert_eq!(fs.stat("/bokem/rozdelany.txt").err(), Some(Error::NotFound));
    assert_eq!(kinds(&fs.diff_clone("/bokem").unwrap()), alloc::vec![(ChangeKind::Modified, "/cfg.toml".into())]);
    fs.restore_copy("/projekt", "v1", "/cfg.toml", "/bokem/cfg.orig").unwrap();
    assert_eq!(fs.read("/bokem/cfg.orig", 0, 10).unwrap(), b"a = 1\n");
    assert_clean(&mut fs);
}
