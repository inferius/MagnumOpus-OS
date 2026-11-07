//! Zurnal zmen: zaznamy operaci, slucovani zapisu, zverejneni az po
//! potvrzeni, stejna cisla po padu a prehrani logu, retence a JOURNAL_GAP,
//! pohled s pozici, opravneni.

use abi::Error;

use super::{assert_clean, new_fs, remount};
use crate::layout::Timestamp;
use crate::{Fs, JournalEntry, Kind, MemDevice, Principal, Settings, jattr, jop, rights};

fn ops(fs: &mut Fs<MemDevice>, ds: &str, from: u64) -> Vec<(u64, u8, String)> {
    fs.journal_read(ds, from, 1024)
        .unwrap()
        .entries
        .into_iter()
        .map(|(q, e)| (q, e.op, if e.new_name.is_empty() { e.name } else { e.new_name }))
        .collect()
}

fn entries(fs: &mut Fs<MemDevice>, ds: &str) -> Vec<(u64, JournalEntry)> {
    let first = fs.journal_read(ds, 1, 1).map(|p| p.first).unwrap_or(1);
    fs.journal_read(ds, first, 1024).unwrap().entries
}

#[test]
fn operations_are_journaled() {
    let mut fs = new_fs(4096);
    fs.set_time(Timestamp { secs: 1000, nanos: 5 });
    fs.create("/a", Kind::Dir).unwrap();
    fs.create("/a/f.txt", Kind::File).unwrap();
    fs.write("/a/f.txt", 0, b"ahoj").unwrap();
    fs.write("/a/f.txt", 4, b" svete").unwrap();
    fs.rename("/a/f.txt", "/g.txt").unwrap();
    fs.set_times("/g.txt", crate::SetTimes { mtime: Some(Timestamp { secs: 5, nanos: 0 }), btime: None, atime: None }).unwrap();
    fs.remove("/g.txt").unwrap();
    // Pred potvrzenim nic neni videt.
    assert!(fs.journal_read("/", 1, 100).unwrap().entries.is_empty());
    fs.commit().unwrap();
    let e = entries(&mut fs, "/");
    let kinds: Vec<u8> = e.iter().map(|(_, e)| e.op).collect();
    // Zapisy obsahu jsou sloucene do jednoho CONTENT na konci commitu.
    assert_eq!(kinds, [jop::CREATE, jop::CREATE, jop::RENAME, jop::ATTR, jop::REMOVE]);
    assert_eq!(e[2].1.name, "f.txt");
    assert_eq!(e[2].1.new_name, "g.txt");
    assert_eq!(e[3].1.aux, jattr::TIMES);
    assert_eq!(e[0].1.time, Timestamp { secs: 1000, nanos: 5 });
    // Poradova cisla souvisla od 1.
    assert_eq!(e.iter().map(|(q, _)| *q).collect::<Vec<_>>(), [1, 2, 3, 4, 5]);
    // Smazany objekt: CONTENT se uz nevytvori (objekt neexistuje).
    fs.create("/h", Kind::File).unwrap();
    fs.write("/h", 0, b"x").unwrap();
    fs.commit().unwrap();
    let last = ops(&mut fs, "/", 6);
    assert_eq!(last.iter().map(|x| x.1).collect::<Vec<_>>(), [jop::CREATE, jop::CONTENT]);
    assert_clean(&mut fs);
}

#[test]
fn content_entry_once_per_commit() {
    let mut fs = new_fs(4096);
    fs.create("/f", Kind::File).unwrap();
    for i in 0..100u64 {
        fs.write("/f", i * 10, b"0123456789").unwrap();
    }
    fs.commit().unwrap();
    let e = entries(&mut fs, "/");
    assert_eq!(e.len(), 2);
    assert_eq!(e[1].1.op, jop::CONTENT);
    assert_eq!(e[1].1.size, 1000);
}

#[test]
fn fsync_publishes_and_replay_keeps_numbers() {
    let mut fs = new_fs(4096);
    fs.commit().unwrap();
    fs.create("/a", Kind::File).unwrap();
    fs.write("/a", 0, &[1u8; 10_000]).unwrap();
    fs.create("/b", Kind::Dir).unwrap();
    fs.fsync().unwrap();
    let before = entries(&mut fs, "/");
    assert_eq!(before.len(), 3, "{before:?}");
    // Druha davka: presun a zapis.
    fs.rename("/a", "/b/a").unwrap();
    fs.write("/b/a", 0, b"zmena").unwrap();
    fs.fsync().unwrap();
    let before = entries(&mut fs, "/");
    // Nepotvrzena zmena po padu zmizi i se svym cislem.
    fs.create("/ztracene", Kind::File).unwrap();
    let mut fs = remount(fs);
    let after = entries(&mut fs, "/");
    assert_eq!(after, before, "zurnal po prehrani logu musi vyjit stejne");
    assert!(fs.stat("/ztracene").is_err());
    // Dalsi cislo navaze.
    fs.create("/c", Kind::File).unwrap();
    fs.commit().unwrap();
    let all = entries(&mut fs, "/");
    assert_eq!(all.last().unwrap().0, before.last().unwrap().0 + 1);
    assert_clean(&mut fs);
}

#[test]
fn replay_keeps_actor() {
    let mut fs = new_fs(4096);
    fs.set_acl("/", Some(crate::Acl { flags: 0, grants: alloc::vec![crate::Grant { who: crate::EVERYONE, rights: rights::TRAVERSE, flags: crate::grant::THIS }] })).unwrap();
    fs.create("/sdilene", Kind::Dir).unwrap();
    fs.set_owner("/sdilene", 1000).unwrap();
    fs.commit().unwrap();
    fs.set_principal(Some(Principal::user(1000, &[])));
    fs.create("/sdilene/x", Kind::File).unwrap();
    fs.write("/sdilene/x", 0, b"data").unwrap();
    fs.set_principal(None);
    fs.create("/y", Kind::File).unwrap();
    fs.fsync().unwrap();
    let before = entries(&mut fs, "/");
    let mut fs = remount(fs);
    let after = entries(&mut fs, "/");
    assert_eq!(after, before);
    let actors: Vec<u64> = after.iter().map(|(_, e)| e.actor).collect();
    assert!(actors.contains(&1000) && actors.contains(&0), "{actors:?}");
}

#[test]
fn retention_and_gap() {
    let mut fs = new_fs(8192);
    fs.set_settings("/", &Settings { journal_max_entries: 10, ..Settings::default() }).unwrap();
    for i in 0..30 {
        fs.create(&alloc::format!("/f{i}"), Kind::File).unwrap();
        fs.commit().unwrap();
    }
    let page = fs.journal_read("/", 30, 100).unwrap();
    assert!(page.last - page.first < 10, "{page:?}");
    // Starsi cislo = JOURNAL_GAP, ne tiche vynechani.
    assert_eq!(fs.journal_read("/", 1, 10).unwrap_err(), Error::JournalGap);
    // Stari: zaznamy starsi nez limit zmizi.
    fs.set_settings("/", &Settings { journal_max_age: 100, ..Settings::default() }).unwrap();
    fs.set_time(Timestamp { secs: 1_000_000, nanos: 0 });
    fs.create("/nove", Kind::File).unwrap();
    fs.commit().unwrap();
    let last = fs.journal_last("/").unwrap();
    let first = fs.journal_read("/", last, 1).unwrap().first;
    let p = fs.journal_read("/", first, 100).unwrap();
    assert_eq!(p.entries.len(), 1);
    assert_eq!(p.entries[0].1.name, "nove");
    assert_clean(&mut fs);
}

#[test]
fn view_gives_consistent_start() {
    let mut fs = new_fs(4096);
    fs.create_dataset("/projekt").unwrap();
    fs.create("/projekt/a", Kind::File).unwrap();
    let v = fs.view("/projekt").unwrap();
    // Zmena po pohledu: v pohledu neni, odber od seq + 1 ji vrati.
    fs.create("/projekt/b", Kind::File).unwrap();
    fs.commit().unwrap();
    let names: Vec<String> = fs.snapshot_list_of("/projekt", &v.snapshot, "/").unwrap().into_iter().map(|e| e.name).collect();
    assert_eq!(names, ["a"]);
    let next: Vec<(u8, String)> = ops(&mut fs, "/projekt", v.seq + 1).into_iter().map(|x| (x.1, x.2)).collect();
    assert_eq!(next.iter().filter(|x| x.0 == jop::CREATE).map(|x| x.1.as_str()).collect::<Vec<_>>(), ["b"]);
    // Pohled neni ve vypisu snapshotu a po uvolneni zmizi.
    assert!(fs.snapshots_of("/projekt").unwrap().is_empty());
    fs.release_view("/projekt", &v.snapshot).unwrap();
    assert!(fs.snapshot_list_of("/projekt", &v.snapshot, "/").is_err());
    // Pohled z doby pred padem se pri pripojeni uklidi.
    let v2 = fs.view("/projekt").unwrap();
    let mut fs = remount(fs);
    assert!(fs.snapshot_list_of("/projekt", &v2.snapshot, "/").is_err());
    assert_clean(&mut fs);
}

#[test]
fn journal_needs_manage() {
    let mut fs = new_fs(4096);
    fs.create("/home", Kind::Dir).unwrap();
    fs.create_dataset("/home/alice").unwrap();
    fs.set_owner("/home/alice", 1000).unwrap();
    fs.set_acl("/", Some(crate::Acl { flags: 0, grants: alloc::vec![crate::Grant { who: crate::EVERYONE, rights: rights::TRAVERSE, flags: crate::grant::BOTH }] })).unwrap();
    fs.commit().unwrap();
    fs.set_principal(Some(Principal::user(1001, &[])));
    assert_eq!(fs.journal_read("/home/alice", 1, 10).unwrap_err(), Error::AccessDenied);
    fs.set_principal(Some(Principal::user(1000, &[])));
    fs.journal_read("/home/alice", 1, 10).unwrap();
    assert_eq!(fs.journal_read("/", 1, 10).unwrap_err(), Error::AccessDenied);
}

#[test]
fn datasets_and_snapshots_in_journal() {
    let mut fs = new_fs(4096);
    fs.create_dataset("/p").unwrap();
    fs.snapshot_of("/p", "v1").unwrap();
    let root: Vec<u8> = entries(&mut fs, "/").iter().map(|(_, e)| e.op).collect();
    assert_eq!(root, [jop::CREATE]);
    let p = entries(&mut fs, "/p");
    assert_eq!(p.len(), 1);
    assert_eq!((p[0].1.op, p[0].1.name.as_str()), (jop::SNAPSHOT, "v1"));
    let s = fs.snapshots_of("/p").unwrap();
    assert_eq!(s[0].kind, crate::snapkind::MANUAL);
    // Snapshot obsahuje zurnal az do sveho vzniku (zaznam SNAPSHOT az po nem).
    assert_eq!(s[0].journal_seq, 0);
    assert_clean(&mut fs);
}

/// Cas zaznamu CONTENT = posledni zmena obsahu: po prehrani logu stejny,
/// i kdyz fsync prisel pozdeji nez zapis.
#[test]
fn content_time_survives_replay() {
    let mut fs = new_fs(4096);
    fs.commit().unwrap();
    fs.set_time(Timestamp { secs: 100, nanos: 0 });
    fs.create("/f", Kind::File).unwrap();
    fs.write("/f", 0, &[3u8; 9000]).unwrap();
    fs.set_time(Timestamp { secs: 105, nanos: 0 });
    fs.create("/g", Kind::File).unwrap();
    fs.fsync().unwrap();
    let before = entries(&mut fs, "/");
    let content = before.iter().find(|(_, e)| e.op == jop::CONTENT).unwrap();
    assert_eq!(content.1.time.secs, 100);
    let mut fs = remount(fs);
    assert_eq!(entries(&mut fs, "/"), before);
}

/// Retence podle stari i v datasetu, ktery se prestal menit.
#[test]
fn journal_tick_prunes_quiet_dataset() {
    let mut fs = new_fs(4096);
    fs.set_settings("/", &Settings { journal_max_age: 100, ..Settings::default() }).unwrap();
    fs.set_time(Timestamp { secs: 10, nanos: 0 });
    fs.create("/a", Kind::File).unwrap();
    fs.commit().unwrap();
    assert!(!fs.journal_tick(Timestamp { secs: 50, nanos: 0 }).unwrap());
    assert!(fs.journal_tick(Timestamp { secs: 1000, nanos: 0 }).unwrap());
    assert_eq!(fs.journal_read("/", 1, 10).unwrap_err(), Error::JournalGap);
    assert_clean(&mut fs);
}
