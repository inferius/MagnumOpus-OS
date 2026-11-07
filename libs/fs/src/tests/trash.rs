//! Kos, rozsirene atributy a expirace souboru.

use abi::Error;

use super::{assert_clean, new_fs, remount};
use crate::layout::Timestamp;
use crate::{Fs, Kind, MemDevice, Principal, Settings, access, jop, rights, share};

fn t(secs: i64) -> Timestamp {
    Timestamp { secs, nanos: 0 }
}

fn setup() -> Fs<MemDevice> {
    let mut fs = new_fs(8192);
    fs.create("/dok", Kind::Dir).unwrap();
    fs.create("/dok/a.txt", Kind::File).unwrap();
    fs.write("/dok/a.txt", 0, b"obsah a").unwrap();
    fs.create("/dok/sub", Kind::Dir).unwrap();
    fs.create("/dok/sub/b.txt", Kind::File).unwrap();
    fs.write("/dok/sub/b.txt", 0, &[7u8; 20_000]).unwrap();
    fs.commit().unwrap();
    fs
}

#[test]
fn trash_restore_purge() {
    let mut fs = setup();
    let used = fs.stat("/").unwrap().tree_bytes;
    let id = fs.trash("/dok/a.txt").unwrap();
    assert_eq!(fs.stat("/dok/a.txt").unwrap_err(), Error::NotFound);
    // Kos nic nekopiruje a nepatri do souhrnu korene.
    assert_eq!(fs.stat("/").unwrap().tree_bytes, used - 7);
    let list = fs.trash_list("/").unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!((list[0].id, list[0].name.as_str(), list[0].original_dir.as_str()), (id, "a.txt", "/dok"));
    // Obnova na puvodni misto.
    assert_eq!(fs.trash_restore("/", id, None).unwrap(), "/dok/a.txt");
    assert_eq!(fs.read("/dok/a.txt", 0, 100).unwrap(), b"obsah a");
    assert!(fs.trash_list("/").unwrap().is_empty());
    // Slozka do kose a trvale vysypani.
    let id = fs.trash("/dok/sub").unwrap();
    fs.commit().unwrap();
    let free = fs.info().free_blocks;
    fs.trash_purge("/", id).unwrap();
    for _ in 0..6 {
        fs.commit().unwrap();
        fs.test_store().mark_changed();
    }
    assert!(fs.info().free_blocks > free, "vysypani musi uvolnit misto");
    assert!(fs.trash_list("/").unwrap().is_empty());
    assert_clean(&mut fs);
    let kinds: Vec<u8> = fs.journal_read("/", 1, 100).unwrap().entries.iter().map(|(_, e)| e.op).collect();
    assert!(kinds.ends_with(&[jop::TRASH, jop::RESTORE, jop::TRASH, jop::REMOVE]), "{kinds:?}");
}

#[test]
fn restore_conflicts_and_dest() {
    let mut fs = setup();
    let id = fs.trash("/dok/a.txt").unwrap();
    fs.create("/dok/a.txt", Kind::File).unwrap();
    assert_eq!(fs.trash_restore("/", id, None).unwrap_err(), Error::AlreadyExists);
    assert_eq!(fs.trash_restore("/", id, Some("/dok/a (obnoveno).txt")).unwrap(), "/dok/a (obnoveno).txt");
    // Puvodni slozka zmizela: NOT_FOUND, jde jen do zvolene.
    let id = fs.trash("/dok/sub/b.txt").unwrap();
    fs.remove("/dok/sub").unwrap();
    assert_eq!(fs.trash_restore("/", id, None).unwrap_err(), Error::NotFound);
    fs.trash_restore("/", id, Some("/b.txt")).unwrap();
    assert_eq!(fs.stat("/b.txt").unwrap().size, 20_000);
    assert_clean(&mut fs);
}

#[test]
fn open_handle_survives_trash() {
    let mut fs = setup();
    let h = fs.open("/dok/a.txt", access::READ | access::WRITE, share::ALL).unwrap();
    let id = fs.trash("/dok/a.txt").unwrap();
    fs.write_h(h, 0, b"OBSAH").unwrap();
    assert_eq!(fs.read_h(h, 0, 100).unwrap(), b"OBSAH a");
    fs.close(h).unwrap();
    fs.trash_restore("/", id, None).unwrap();
    assert_eq!(fs.read("/dok/a.txt", 0, 100).unwrap(), b"OBSAH a");
    // Bez sdileni mazani nejde ani do kose.
    let h = fs.open("/dok/a.txt", access::READ, share::READ).unwrap();
    assert_eq!(fs.trash("/dok/a.txt").unwrap_err(), Error::Locked);
    fs.close(h).unwrap();
}

#[test]
fn trash_survives_crash_via_log() {
    let mut fs = setup();
    let a = fs.trash("/dok/a.txt").unwrap();
    let b = fs.trash("/dok/sub").unwrap();
    fs.trash_restore("/", a, Some("/a2.txt")).unwrap();
    fs.trash_purge("/", b).unwrap();
    fs.fsync().unwrap();
    let before = fs.journal_read("/", 1, 100).unwrap().entries;
    let mut fs = remount(fs);
    assert_eq!(fs.read("/a2.txt", 0, 100).unwrap(), b"obsah a");
    assert!(fs.trash_list("/").unwrap().is_empty());
    assert_eq!(fs.stat("/dok/sub").unwrap_err(), Error::NotFound);
    assert_eq!(fs.journal_read("/", 1, 100).unwrap().entries, before);
    assert_clean(&mut fs);
}

#[test]
fn trash_tick_by_age_and_dataset_rule() {
    let mut fs = setup();
    fs.set_settings("/", &Settings { trash_max_age: 3600, ..Settings::default() }).unwrap();
    fs.set_time(t(1000));
    fs.trash("/dok/a.txt").unwrap();
    fs.set_time(t(4000));
    fs.trash("/dok/sub").unwrap();
    // a.txt je starsi nez hodina, sub ne.
    assert_eq!(fs.trash_tick(t(4700)).unwrap(), 1);
    let left = fs.trash_list("/").unwrap();
    assert_eq!(left.len(), 1);
    assert_eq!(left[0].name, "sub");
    assert_eq!(fs.trash_tick(t(4700)).unwrap(), 0);
    assert_clean(&mut fs);
}

#[test]
fn trash_visibility_and_rights() {
    let mut fs = new_fs(8192);
    fs.set_acl("/", Some(crate::Acl { flags: 0, grants: alloc::vec![crate::Grant { who: crate::EVERYONE, rights: rights::TRAVERSE, flags: crate::grant::THIS }] })).unwrap();
    fs.create_dataset("/sdilene").unwrap();
    fs.set_acl("/sdilene", Some(crate::Acl { flags: 0, grants: alloc::vec![crate::Grant { who: crate::EVERYONE, rights: rights::EDIT, flags: crate::grant::BOTH }] })).unwrap();
    fs.create("/sdilene/alice.txt", Kind::File).unwrap();
    fs.create("/sdilene/bob.txt", Kind::File).unwrap();
    fs.set_principal(Some(Principal::user(1000, &[])));
    let a = fs.trash("/sdilene/alice.txt").unwrap();
    fs.set_principal(Some(Principal::user(1001, &[])));
    fs.trash("/sdilene/bob.txt").unwrap();
    // Bob vidi jen svuj zaznam a cizi nesmi vysypat ani obnovit.
    let mine = fs.trash_list("/sdilene").unwrap();
    assert_eq!(mine.len(), 1);
    assert_eq!(mine[0].name, "bob.txt");
    assert_eq!(fs.trash_purge("/sdilene", a).unwrap_err(), Error::AccessDenied);
    assert_eq!(fs.trash_restore("/sdilene", a, None).unwrap_err(), Error::NotFound);
    // Spravce (system) vidi vse.
    fs.set_principal(None);
    assert_eq!(fs.trash_list("/sdilene").unwrap().len(), 2);
    assert_clean(&mut fs);
}

#[test]
fn xattrs_basic_and_namespaces() {
    let mut fs = setup();
    fs.set_xattr("/dok/a.txt", "user.barva", b"modra").unwrap();
    fs.set_xattr("/dok/a.txt", "user.stitek", b"prace").unwrap();
    assert_eq!(fs.get_xattr("/dok/a.txt", "user.barva").unwrap(), b"modra");
    assert_eq!(fs.list_xattrs("/dok/a.txt").unwrap(), ["user.barva", "user.stitek"]);
    fs.set_xattr("/dok/a.txt", "user.barva", b"zelena").unwrap();
    assert_eq!(fs.get_xattr("/dok/a.txt", "user.barva").unwrap(), b"zelena");
    fs.remove_xattr("/dok/a.txt", "user.stitek").unwrap();
    assert_eq!(fs.get_xattr("/dok/a.txt", "user.stitek").unwrap_err(), Error::NotFound);
    assert_eq!(fs.set_xattr("/dok/a.txt", "jiny.x", b"").unwrap_err(), Error::InvalidArgs);
    // sys.* jen pres vlastni API (uzivatel).
    fs.set_acl("/", Some(crate::Acl { flags: 0, grants: alloc::vec![crate::Grant { who: crate::EVERYONE, rights: rights::EDIT, flags: crate::grant::BOTH }] })).unwrap();
    fs.set_principal(Some(Principal::user(1000, &[])));
    assert_eq!(fs.set_xattr("/dok/a.txt", "sys.expires", &[0; 12]).unwrap_err(), Error::AccessDenied);
    fs.set_principal(None);
    // Prezije fsync a pad; smazani objektu atributy smaze.
    fs.fsync().unwrap();
    let mut fs = remount(fs);
    assert_eq!(fs.get_xattr("/dok/a.txt", "user.barva").unwrap(), b"zelena");
    assert_clean(&mut fs);
    fs.remove("/dok/a.txt").unwrap();
    assert_clean(&mut fs);
}

#[test]
fn expiry_moves_to_trash() {
    let mut fs = setup();
    fs.set_expires("/dok/a.txt", Some(t(5000))).unwrap();
    fs.set_expires("/dok/sub", Some(t(9000))).unwrap();
    assert_eq!(fs.expires("/dok/a.txt").unwrap(), Some(t(5000)));
    assert_clean(&mut fs);
    assert_eq!(fs.expire_tick(t(4999)).unwrap(), 0);
    assert_eq!(fs.expire_tick(t(5000)).unwrap(), 1);
    assert_eq!(fs.stat("/dok/a.txt").unwrap_err(), Error::NotFound);
    let trash = fs.trash_list("/").unwrap();
    assert_eq!(trash[0].name, "a.txt");
    // Po obnove uz expirace neplati.
    fs.trash_restore("/", trash[0].id, None).unwrap();
    assert_eq!(fs.expires("/dok/a.txt").unwrap(), None);
    // Zruseni expirace.
    fs.set_expires("/dok/sub", None).unwrap();
    assert_eq!(fs.expire_tick(t(10_000)).unwrap(), 0);
    // Smazani objektu s expiraci uklidi index (fsck).
    fs.set_expires("/dok/a.txt", Some(t(20_000))).unwrap();
    fs.remove("/dok/a.txt").unwrap();
    assert_clean(&mut fs);
    // Prezije pad pres intent log.
    fs.set_expires("/dok/sub/b.txt", Some(t(30_000))).unwrap();
    fs.fsync().unwrap();
    let mut fs = remount(fs);
    assert_eq!(fs.expires("/dok/sub/b.txt").unwrap(), Some(t(30_000)));
    assert_clean(&mut fs);
}

#[test]
fn trash_in_other_dataset() {
    let mut fs = new_fs(8192);
    fs.create_dataset("/projekt").unwrap();
    fs.create("/projekt/f", Kind::File).unwrap();
    let id = fs.trash("/projekt/f").unwrap();
    assert!(fs.trash_list("/").unwrap().is_empty());
    assert_eq!(fs.trash_list("/projekt").unwrap()[0].id, id);
    // Dataset do kose nejde.
    assert_eq!(fs.trash("/projekt").unwrap_err(), Error::WrongType);
    fs.commit().unwrap();
    assert_clean(&mut fs);
}
