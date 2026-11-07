//! Historie: verze souboru, obnova verze, vraceni operaci, automaticke
//! snapshoty s pravidly uchovani; slouceni textu (diff3).

use abi::Error;
use proptest::prelude::*;

use super::{assert_clean, cases, new_fs};
use crate::layout::Timestamp;
use crate::merge3;
use crate::{AutoRule, Fs, Kind, MemDevice, RestoreMode, Settings, Undone, jop, snapkind};

fn t(secs: i64) -> Timestamp {
    Timestamp { secs, nanos: 0 }
}

fn write_file(fs: &mut Fs<MemDevice>, p: &str, d: &[u8]) {
    if fs.stat(p).is_err() {
        fs.create(p, Kind::File).unwrap();
    }
    fs.truncate(p, 0).unwrap();
    fs.write(p, 0, d).unwrap();
}

fn read(fs: &mut Fs<MemDevice>, p: &str) -> Vec<u8> {
    fs.read(p, 0, 1 << 20).unwrap()
}

/// Posledni zaznam zurnalu operace `op` v datasetu.
fn last_seq(fs: &mut Fs<MemDevice>, ds: &str, op: u8) -> u64 {
    fs.commit().unwrap();
    let last = fs.journal_last(ds).unwrap();
    let first = fs.journal_read(ds, last, 1).unwrap().first;
    fs.journal_read(ds, first, 1024).unwrap().entries.iter().rev().find(|(_, e)| e.op == op).unwrap().0
}

#[test]
fn file_versions_follow_renames() {
    let mut fs = new_fs(8192);
    fs.create_dataset("/p").unwrap();
    write_file(&mut fs, "/p/zprava.txt", b"verze 1");
    fs.snapshot_of("/p", "s1").unwrap();
    fs.snapshot_of("/p", "s1b").unwrap();
    write_file(&mut fs, "/p/zprava.txt", b"verze 2");
    fs.snapshot_of("/p", "s2").unwrap();
    fs.rename("/p/zprava.txt", "/p/report.txt").unwrap();
    write_file(&mut fs, "/p/report.txt", b"verze 3");
    let v = fs.file_versions("/p/report.txt").unwrap();
    // s1 a s1b obsahuji stejnou verzi: jen jednou (nejstarsi snapshot).
    assert_eq!(v.iter().map(|x| x.snapshot.as_str()).collect::<Vec<_>>(), ["s1", "s2"]);
    // Obnova verze jako kopie (snapshot ma stare jmeno - hleda se podle oid).
    fs.restore_version("/p/report.txt", "s1", RestoreMode::Copy("/p/report (verze 1).txt")).unwrap();
    assert_eq!(read(&mut fs, "/p/report (verze 1).txt"), b"verze 1");
    // Nahrazeni: aktualni jde do kose.
    fs.restore_version("/p/report.txt", "s2", RestoreMode::Replace).unwrap();
    assert_eq!(read(&mut fs, "/p/report.txt"), b"verze 2");
    let trash = fs.trash_list("/p").unwrap();
    assert_eq!(trash.len(), 1);
    fs.trash_restore("/p", trash[0].id, Some("/p/stara.txt")).unwrap();
    assert_eq!(read(&mut fs, "/p/stara.txt"), b"verze 3");
    assert_clean(&mut fs);
}

#[test]
fn undo_rename_trash_create() {
    let mut fs = new_fs(8192);
    fs.create("/a", Kind::Dir).unwrap();
    write_file(&mut fs, "/a/f.txt", b"data");
    fs.rename("/a/f.txt", "/g.txt").unwrap();
    let seq = last_seq(&mut fs, "/", jop::RENAME);
    assert_eq!(fs.undo("/", seq).unwrap(), Undone::MovedBack("/a/f.txt".into()));
    assert_eq!(read(&mut fs, "/a/f.txt"), b"data");
    // Podruhe uz objekt na miste neni: konflikt.
    assert_eq!(fs.undo("/", seq).unwrap_err(), Error::Conflict);
    fs.trash("/a/f.txt").unwrap();
    let seq = last_seq(&mut fs, "/", jop::TRASH);
    assert_eq!(fs.undo("/", seq).unwrap(), Undone::Restored("/a/f.txt".into()));
    fs.create("/novy", Kind::File).unwrap();
    let seq = last_seq(&mut fs, "/", jop::CREATE);
    assert!(matches!(fs.undo("/", seq).unwrap(), Undone::Trashed(_)));
    assert_eq!(fs.stat("/novy").unwrap_err(), Error::NotFound);
    assert_clean(&mut fs);
}

#[test]
fn undo_remove_and_content_from_snapshot() {
    let mut fs = new_fs(8192);
    write_file(&mut fs, "/f.txt", b"puvodni");
    write_file(&mut fs, "/g.txt", b"g puvodni");
    fs.snapshot("pred").unwrap();
    fs.remove("/f.txt").unwrap();
    let seq = last_seq(&mut fs, "/", jop::REMOVE);
    match fs.undo("/", seq).unwrap() {
        Undone::FromSnapshot { path, snapshot, trashed } => {
            assert_eq!((path.as_str(), snapshot.as_str(), trashed), ("/f.txt", "pred", None));
        }
        u => panic!("{u:?}"),
    }
    assert_eq!(read(&mut fs, "/f.txt"), b"puvodni");
    write_file(&mut fs, "/g.txt", b"prepsano");
    let g = fs.stat("/g.txt").unwrap().oid;
    fs.commit().unwrap();
    let seq = fs.journal_read("/", 1, 1024).unwrap().entries.iter().rev().find(|(_, e)| e.op == jop::CONTENT && e.oid == g).unwrap().0;
    let Undone::FromSnapshot { trashed: Some(id), .. } = fs.undo("/", seq).unwrap() else { panic!() };
    assert_eq!(read(&mut fs, "/g.txt"), b"g puvodni");
    // Prepsana verze se neztratila (kos).
    fs.trash_restore("/", id, Some("/g-prepsano.txt")).unwrap();
    assert_eq!(read(&mut fs, "/g-prepsano.txt"), b"prepsano");
    // Bez snapshotu pred operaci: NOT_FOUND.
    fs.create("/bez", Kind::File).unwrap();
    fs.remove("/bez").unwrap();
    let seq = last_seq(&mut fs, "/", jop::REMOVE);
    assert_eq!(fs.undo("/", seq).unwrap_err(), Error::NotFound);
    assert_clean(&mut fs);
}

#[test]
fn auto_snapshots_rules() {
    let mut fs = new_fs(16384);
    fs.create_dataset("/home").unwrap();
    // 5 minut / drzet 3, hodinove / drzet 2.
    let rules = alloc::vec![AutoRule { interval: 300, keep: 3 }, AutoRule { interval: 3600, keep: 2 }];
    fs.set_settings("/home", &Settings { rules, ..Settings::default() }).unwrap();
    let mut made = 0;
    for step in 0..30i64 {
        let now = t(10_000 + step * 300);
        fs.set_time(now);
        write_file(&mut fs, "/home/f", alloc::format!("{step}").as_bytes());
        made += fs.auto_snapshot_tick(now).unwrap().0;
    }
    assert_eq!(made, 30);
    let autos: Vec<_> = fs.snapshots_of("/home").unwrap().into_iter().filter(|s| s.kind == snapkind::AUTO).collect();
    // 3 nejnovejsi petiminutove + nejnovejsi z predchoziho hodinoveho intervalu.
    assert!(autos.len() <= 5, "{autos:?}");
    assert!(autos.len() >= 3);
    let times: Vec<i64> = autos.iter().map(|s| s.time.secs).collect();
    assert!(times.contains(&(10_000 + 29 * 300)));
    // Beze zmeny se novy snapshot nedela.
    let now = t(10_000 + 31 * 300);
    assert_eq!(fs.auto_snapshot_tick(now).unwrap().0, 0);
    // Rucni snapshot pravidla nemazou.
    fs.snapshot_of("/home", "rucni").unwrap();
    fs.auto_snapshot_tick(t(100_000)).unwrap();
    assert!(fs.snapshots_of("/home").unwrap().iter().any(|s| s.name == "rucni"));
    // Bez pravidel nic.
    assert_eq!(fs.auto_snapshot_tick(t(200_000)).unwrap().0, 0);
    assert_eq!(fs.snapshots_of("/").unwrap().len(), 0);
    assert_clean(&mut fs);
}

#[test]
fn settings_validate() {
    let mut fs = new_fs(4096);
    let bad = Settings { rules: alloc::vec![AutoRule { interval: 10, keep: 1 }], ..Settings::default() };
    assert_eq!(fs.set_settings("/", &bad).unwrap_err(), Error::InvalidArgs);
    let ok = Settings { trash_max_age: 7 * 86_400, rules: alloc::vec![AutoRule { interval: 3600, keep: 24 }], ..Settings::default() };
    fs.set_settings("/", &ok).unwrap();
    assert_eq!(fs.settings("/").unwrap(), ok);
    let mut fs = super::remount(fs);
    assert_eq!(fs.settings("/").unwrap(), ok);
}

// --- slouceni textu ---

#[test]
fn merge3_clean_and_conflict() {
    let base = b"a\nb\nc\nd\n";
    let ours = b"a\nB\nc\nd\n";
    let theirs = b"a\nb\nc\nD\n";
    let m = merge3::merge(base, ours, theirs, "nase", "jejich");
    assert_eq!((m.data.as_slice(), m.conflicts), (&b"a\nB\nc\nD\n"[..], 0));
    // Stejna zmena na obou stranach.
    let m = merge3::merge(base, ours, ours, "nase", "jejich");
    assert_eq!((m.data.as_slice(), m.conflicts), (&ours[..], 0));
    // Ruzna zmena stejneho radku.
    let m = merge3::merge(base, b"a\nX\nc\nd\n", b"a\nY\nc\nd\n", "nase", "jejich");
    assert_eq!(m.conflicts, 1);
    assert_eq!(m.data, b"a\n<<<<<<< nase\nX\n=======\nY\n>>>>>>> jejich\nc\nd\n");
    // Bez koncoveho radku.
    let m = merge3::merge(b"x", b"y", b"z", "n", "j");
    assert_eq!(m.data, b"<<<<<<< n\ny\n=======\nz\n>>>>>>> j\n");
    assert!(merge3::is_text(b"ahoj\n"));
    assert!(!merge3::is_text(b"a\0b"));
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(cases(64)))]

    /// Zmena jen na jedne strane se prevezme beze ztraty (vysledek = ta strana).
    #[test]
    fn merge3_one_side(base in proptest::collection::vec("[a-c]{0,3}\n", 0..20), edits in proptest::collection::vec((0usize..25, "[a-d]{0,3}\n", 0u8..3), 0..6)) {
        let base: String = base.concat();
        let mut lines: Vec<String> = base.lines().map(|l| alloc::format!("{l}\n")).collect();
        for (pos, text, kind) in edits {
            let pos = pos.min(lines.len());
            match kind {
                0 => lines.insert(pos, text),
                1 if pos < lines.len() => { lines.remove(pos); }
                _ if pos < lines.len() => lines[pos] = text,
                _ => {}
            }
        }
        let changed: String = lines.concat();
        let m = merge3::merge(base.as_bytes(), changed.as_bytes(), base.as_bytes(), "n", "j");
        prop_assert_eq!(m.conflicts, 0);
        prop_assert_eq!(&m.data, changed.as_bytes());
        let m = merge3::merge(base.as_bytes(), base.as_bytes(), changed.as_bytes(), "n", "j");
        prop_assert_eq!(m.conflicts, 0);
        prop_assert_eq!(&m.data, changed.as_bytes());
    }
}
