//! Handly: zivotnost otevreneho souboru, sirotci, atomicke nahrazeni,
//! smazani pri zavreni, sdileci rezimy a zamky rozsahu.

use abi::Error;

use super::{assert_clean, new_fs, remount};
use crate::{Kind, access, share};

const RW: u8 = access::READ | access::WRITE;

#[test]
fn reader_keeps_old_object_after_atomic_replace() {
    // A otevre report, B ho atomicky nahradi; A cte dal puvodni obsah,
    // nove otevreni dostane novy objekt.
    let mut fs = new_fs(4096);
    fs.commit().unwrap();
    let base = fs.info().free_blocks;
    fs.create("/report", Kind::File).unwrap();
    fs.write("/report", 0, &[1u8; 50_000]).unwrap();
    let a = fs.open("/report", access::READ, share::ALL).unwrap();
    fs.create("/report.tmp", Kind::File).unwrap();
    fs.write("/report.tmp", 0, &[2u8; 30_000]).unwrap();
    fs.rename_replace("/report.tmp", "/report").unwrap();
    assert_eq!(fs.read_h(a, 0, 50_000).unwrap(), [1u8; 50_000]);
    let b = fs.open("/report", access::READ, share::ALL).unwrap();
    assert_eq!(fs.read_h(b, 0, 30_000).unwrap(), [2u8; 30_000]);
    assert_eq!(fs.stat("/report").unwrap().size, 30_000);
    // Souhrn adresare pocita jen pojmenovane soubory.
    assert_eq!(fs.stat("/").unwrap().tree_files, 1);
    fs.close(a).unwrap();
    fs.close(b).unwrap();
    fs.remove("/report").unwrap();
    for _ in 0..crate::REUSE_DELAY + 2 {
        fs.commit().unwrap();
    }
    assert!(fs.info().free_blocks + 8 >= base, "sirotek po zavreni neuvolnen");
    assert_clean(&mut fs);
}

#[test]
fn removed_open_file_lives_until_close_and_survives_crash_cleanup() {
    let mut fs = new_fs(4096);
    fs.create("/f", Kind::File).unwrap();
    fs.write("/f", 0, b"obsah").unwrap();
    let h = fs.open("/f", RW, share::ALL).unwrap();
    fs.remove("/f").unwrap();
    assert_eq!(fs.stat("/f").err(), Some(Error::NotFound));
    // Handle dal funguje: cteni, zapis, zmena velikosti.
    fs.write_h(h, 5, &[7u8; 20_000]).unwrap();
    assert_eq!(fs.read_h(h, 0, 5).unwrap(), b"obsah");
    assert_eq!(fs.stat_h(h).unwrap().size, 20_005);
    fs.set_len_h(h, 3).unwrap();
    // Commit se sirotkem, pak "pad" (handle ztracen): pripojeni ho uklidi.
    fs.commit().unwrap();
    assert_clean(&mut fs);
    let mut fs = remount(fs);
    assert!(fs.open_handles().is_empty());
    assert_clean(&mut fs);
    // A s fsync misto commitu: prehrani logu sirotka nevzkrisi.
    fs.create("/g", Kind::File).unwrap();
    fs.commit().unwrap();
    let h = fs.open("/g", RW, share::ALL).unwrap();
    fs.remove("/g").unwrap();
    fs.write_h(h, 0, &[3u8; 10_000]).unwrap();
    fs.fsync().unwrap();
    let mut fs = remount(fs);
    assert_eq!(fs.stat("/g").err(), Some(Error::NotFound));
    assert_clean(&mut fs);
}

#[test]
fn share_modes() {
    let mut fs = new_fs(2048);
    fs.create("/s", Kind::File).unwrap();
    // A sdili jen cteni: nikdo jiny nesmi zapisovat ani mazat.
    let a = fs.open("/s", access::READ, share::READ).unwrap();
    assert_eq!(fs.open("/s", access::WRITE, share::ALL).err(), Some(Error::Locked));
    let r = fs.open("/s", access::READ, share::ALL).err();
    // Druhy ctenar musi sdilet cteni A - sdili vse, tedy i cteni: projde.
    assert_eq!(r, None);
    assert_eq!(fs.write("/s", 0, b"x").err(), Some(Error::Locked));
    assert_eq!(fs.remove("/s").err(), Some(Error::Locked));
    assert_eq!(fs.rename("/s", "/t").err(), Some(Error::Locked));
    // Novy handle, ktery nesdili cteni, zatimco A cte: odmitnut.
    assert_eq!(fs.open("/s", access::READ, 0).err(), Some(Error::Locked));
    fs.close(a).unwrap();
    for (h, _, _) in fs.open_handles() {
        fs.close(h).unwrap();
    }
    fs.write("/s", 0, b"x").unwrap();
    fs.rename("/s", "/t").unwrap();
}

#[test]
fn delete_on_close() {
    let mut fs = new_fs(2048);
    fs.create("/tmp1", Kind::File).unwrap();
    let h = fs.open("/tmp1", access::READ, share::ALL).unwrap();
    assert_eq!(fs.set_delete_on_close(h, true).err(), Some(Error::AccessDenied));
    let d = fs.open("/tmp1", access::DELETE, share::ALL).unwrap();
    fs.set_delete_on_close(d, true).unwrap();
    fs.close(d).unwrap();
    // Jeste otevrene druhym handlem: jmeno zustava.
    assert!(fs.stat("/tmp1").is_ok());
    fs.close(h).unwrap();
    assert_eq!(fs.stat("/tmp1").err(), Some(Error::NotFound));
    assert_eq!(fs.close(h).err(), Some(Error::BadHandle));
    assert_clean(&mut fs);
}

#[test]
fn range_locks() {
    let mut fs = new_fs(2048);
    fs.create("/db", Kind::File).unwrap();
    fs.write("/db", 0, &[0u8; 8192]).unwrap();
    let a = fs.open("/db", RW, share::ALL).unwrap();
    let b = fs.open("/db", RW, share::ALL).unwrap();
    // Poradni vyhradni zamek: brani jen jinym zamkum.
    fs.lock_range(a, 0, 100, true, false).unwrap();
    assert_eq!(fs.lock_range(b, 50, 10, false, false).err(), Some(Error::Locked));
    fs.write_h(b, 0, b"poradni zapis projde").unwrap();
    fs.lock_range(b, 100, 100, false, false).unwrap();
    fs.lock_range(a, 150, 10, false, false).unwrap();
    // Povinny vyhradni zamek: brani i zapisu a cteni jinych.
    fs.lock_range(a, 1000, 100, true, true).unwrap();
    assert_eq!(fs.write_h(b, 1050, b"x").err(), Some(Error::Locked));
    assert_eq!(fs.read_h(b, 1050, 1).err(), Some(Error::Locked));
    assert_eq!(fs.write("/db", 1099, b"x").err(), Some(Error::Locked));
    fs.write_h(a, 1050, b"vlastnik smi").unwrap();
    assert_eq!(fs.set_len_h(b, 500).err(), Some(Error::Locked));
    fs.unlock_range(a, 1000, 100).unwrap();
    fs.write_h(b, 1050, b"ok").unwrap();
    // Zavreni uvolni zamky.
    fs.lock_range(a, 2000, 0, true, true).unwrap();
    fs.close(a).unwrap();
    fs.write_h(b, 4000, b"po zavreni").unwrap();
    fs.close(b).unwrap();
}

#[test]
fn handles_across_datasets_and_renames() {
    let mut fs = new_fs(4096);
    fs.create_dataset("/p").unwrap();
    fs.create("/p/a", Kind::File).unwrap();
    let h = fs.open("/p/a", RW, share::ALL).unwrap();
    fs.rename("/p/a", "/p/b").unwrap();
    fs.write_h(h, 0, b"pres prejmenovani").unwrap();
    assert_eq!(fs.read("/p/b", 0, 100).unwrap(), b"pres prejmenovani");
    assert_eq!(fs.stat_h(h).unwrap().dataset, fs.stat("/p").unwrap().dataset);
    fs.close(h).unwrap();
    let mut fs = remount({
        fs.commit().unwrap();
        fs
    });
    assert_clean(&mut fs);
}

#[test]
fn conditional_save_refuses_silent_overwrite() {
    // Editor si pamatuje revizi; mezitim soubor zmeni agent - podminene
    // ulozeni editoru vrati CONFLICT a nic nezmeni.
    let mut fs = new_fs(2048);
    fs.create("/config.toml", Kind::File).unwrap();
    fs.write("/config.toml", 0, b"a = 1").unwrap();
    let rev = fs.revision("/config.toml").unwrap();
    fs.write("/config.toml", 0, b"a = 2").unwrap(); // agent
    fs.create("/config.toml.tmp", Kind::File).unwrap();
    fs.write("/config.toml.tmp", 0, b"a = 3").unwrap(); // editor
    assert_eq!(fs.rename_replace_if("/config.toml.tmp", "/config.toml", Some(rev)).err(), Some(Error::Conflict));
    assert_eq!(fs.read("/config.toml", 0, 10).unwrap(), b"a = 2");
    assert!(fs.stat("/config.toml.tmp").is_ok());
    // S aktualni revizi projde.
    let now = fs.revision("/config.toml").unwrap();
    let new = fs.rename_replace_if("/config.toml.tmp", "/config.toml", Some(now)).unwrap();
    assert_eq!(fs.read("/config.toml", 0, 10).unwrap(), b"a = 3");
    assert_ne!(new, now);
    // None = cil nesmi existovat.
    fs.create("/x.tmp", Kind::File).unwrap();
    assert_eq!(fs.rename_replace_if("/x.tmp", "/config.toml", None).err(), Some(Error::Conflict));
    fs.rename_replace_if("/x.tmp", "/novy", None).unwrap();
    assert_clean(&mut fs);
}

#[test]
fn reopen_after_service_restart_keeps_orphans() {
    // Pad sluzby: klient drzi (dataset, oid, generace). Nova instance se
    // pripoji s keep_orphans a otevre soubor i smazany sirotek znovu.
    let mut fs = new_fs(4096);
    fs.create("/d", Kind::Dir).unwrap();
    fs.create("/d/zivy", Kind::File).unwrap();
    fs.write("/d/zivy", 0, b"zivy obsah").unwrap();
    fs.create("/smazany", Kind::File).unwrap();
    fs.write("/smazany", 0, b"obsah sirotka").unwrap();
    let a = fs.open("/d/zivy", RW, share::ALL).unwrap();
    let b = fs.open("/smazany", access::READ, share::ALL).unwrap();
    let ra = fs.revision_h(a).unwrap();
    let rb = fs.revision_h(b).unwrap();
    fs.remove("/smazany").unwrap();
    fs.commit().unwrap();
    // Pad: nova instance se sirotky.
    let dev = fs.into_device();
    let opts = crate::MountOptions { keep_orphans: true, ..Default::default() };
    let mut fs = crate::Fs::mount_with(dev, opts).unwrap();
    let (a2, path) = fs.reopen(ra.dataset, ra.oid, ra.generation, RW, share::ALL).unwrap();
    assert_eq!(path.as_deref(), Some("/d/zivy"));
    assert_eq!(fs.read_h(a2, 0, 100).unwrap(), b"zivy obsah");
    let (b2, path) = fs.reopen(rb.dataset, rb.oid, rb.generation, access::READ, share::ALL).unwrap();
    assert_eq!(path, None);
    assert_eq!(fs.read_h(b2, 0, 100).unwrap(), b"obsah sirotka");
    // Spatna generace = objekt zanikl.
    assert_eq!(fs.reopen(ra.dataset, ra.oid, ra.generation + 7, RW, share::ALL).err(), Some(Error::NotFound));
    // Uklid po lhute: otevreny sirotek zustane, po zavreni zmizi.
    fs.cleanup_unopened_orphans().unwrap();
    assert_eq!(fs.read_h(b2, 0, 5).unwrap(), b"obsah");
    fs.close(b2).unwrap();
    fs.close(a2).unwrap();
    fs.commit().unwrap();
    assert_clean(&mut fs);
    // Bez keep_orphans (start systemu) sirotek pri pripojeni zmizi.
    let mut fs = remount(fs);
    assert_eq!(fs.reopen(rb.dataset, rb.oid, rb.generation, access::READ, share::ALL).err(), Some(Error::NotFound));
}

#[test]
fn reopen_checks_rights_again() {
    let mut fs = new_fs(4096);
    fs.create("/tajne", Kind::File).unwrap();
    let h = fs.open("/tajne", access::READ, share::ALL).unwrap();
    let r = fs.revision_h(h).unwrap();
    fs.close(h).unwrap();
    // Uzivatel bez prav: znovuotevreni neprojde ani po restartu (objekt,
    // ktery nevidi, je pro nej NOT_FOUND).
    fs.set_principal(Some(crate::Principal { user: 1000, groups: alloc::vec::Vec::new() }));
    let r2 = fs.reopen(r.dataset, r.oid, r.generation, access::READ, share::ALL);
    assert!(matches!(r2, Err(Error::NotFound | Error::AccessDenied)), "{r2:?}");
}

#[test]
fn pinned_handle_ignores_share_modes_and_keeps_orphan() {
    // fsd drzi interni handle pro mmap: zapisuje i pres handle klienta jen
    // pro cteni, nebrani otevreni se zakazem zapisu, sirotek zustane.
    let mut fs = new_fs(4096);
    fs.create("/m", Kind::File).unwrap();
    fs.write("/m", 0, b"abc").unwrap();
    let h = fs.open("/m", access::READ, share::ALL).unwrap();
    let pin = fs.pin_h(h).unwrap();
    fs.close(h).unwrap();
    // Jiny klient smi otevrit se zakazem zapisu (interni handle nema pristup).
    let deny = fs.open("/m", access::READ, share::READ).unwrap();
    fs.write_h(pin, 1, b"XY").unwrap();
    assert_eq!(fs.read_h(pin, 0, 3).unwrap(), b"aXY");
    fs.close(deny).unwrap();
    // Povinny zamek jineho handlu interni zapis nezastavi.
    let l = fs.open("/m", RW, share::ALL).unwrap();
    fs.lock_range(l, 0, 10, true, true).unwrap();
    fs.write_h(pin, 0, b"Z").unwrap();
    fs.close(l).unwrap();
    // Smazani: objekt zije, dokud interni handle neni zavreny.
    fs.remove("/m").unwrap();
    assert_eq!(fs.read_h(pin, 0, 3).unwrap(), b"ZXY");
    fs.set_len_h(pin, 1).unwrap();
    assert_eq!(fs.stat_h(pin).unwrap().size, 1);
    fs.close(pin).unwrap();
    assert_eq!(fs.pin_h(pin).err(), Some(Error::BadHandle));
    assert_clean(&mut fs);
}
