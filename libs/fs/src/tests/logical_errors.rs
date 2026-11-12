//! Audit stavu chyby: logicka chyba operace (uz existuje, neexistuje,
//! neprazdne, spatny typ, neplatny argument) nesmi prepnout svazek do stavu
//! chyby. Kdyz operace zmeni stav v pameti a az pak zjisti logickou chybu,
//! `guarded` svazek "otravi" a kazdy dalsi zapis vraci `IO_ERROR` (tak
//! vznikla chyba zakladani datasetu z VMware).
//!
//! Kazdy pripad ma pripravu, operaci, ktera ma selhat logicky, a po ni se
//! zkusi zapis a commit. Vysledek je seznam vsech provinilcu najednou.

extern crate std;

use std::format;
use std::string::String;
use std::vec::Vec;

use abi::Error;

use super::new_fs;
use crate::{Fs, Kind, MemDevice, RestoreMode, Result};

type Op = fn(&mut Fs<MemDevice>) -> Result<()>;

/// Spolecny zaklad: soubor, slozka s obsahem, dataset se snapshotem.
fn setup(fs: &mut Fs<MemDevice>) {
    fs.create("/f", Kind::File).unwrap();
    fs.write("/f", 0, b"data").unwrap();
    fs.create("/d", Kind::Dir).unwrap();
    fs.create("/d/x", Kind::File).unwrap();
    fs.create("/prazdna", Kind::Dir).unwrap();
    fs.create_dataset("/ds").unwrap();
    fs.create("/ds/g", Kind::File).unwrap();
    fs.write("/ds/g", 0, b"v1").unwrap();
    fs.snapshot_of("/ds", "S").unwrap();
    fs.commit().unwrap();
}

/// (jmeno, operace, ktera ma selhat logicky)
const CASES: &[(&str, Op)] = &[
    ("create existujici soubor", |fs| fs.create("/f", Kind::File)),
    ("create existujici slozka", |fs| fs.create("/d", Kind::Dir)),
    ("create bez rodice", |fs| fs.create("/nic/a", Kind::File)),
    ("create v souboru", |fs| fs.create("/f/a", Kind::File)),
    ("create neplatne jmeno", |fs| fs.create("/a\u{202e}b", Kind::File)),
    ("symlink na existujici jmeno", |fs| fs.symlink("/f", "/x")),
    ("remove neexistuje", |fs| fs.remove("/nic")),
    ("remove neprazdna slozka", |fs| fs.remove("/d")),
    ("remove dataset", |fs| fs.remove("/ds")),
    ("rename zdroj neexistuje", |fs| fs.rename("/nic", "/b")),
    ("rename na neprazdnou slozku", |fs| fs.rename("/prazdna", "/d")),
    ("rename slozky do sebe", |fs| fs.rename("/d", "/d/pod")),
    ("rename mezi datasety", |fs| fs.rename("/f", "/ds/f")),
    ("rename soubor na slozku", |fs| fs.rename("/f", "/prazdna")),
    ("rename_replace neexistuje", |fs| fs.rename_replace("/nic", "/f")),
    ("write do slozky", |fs| fs.write("/d", 0, b"x")),
    ("write neexistuje", |fs| fs.write("/nic", 0, b"x")),
    ("truncate slozky", |fs| fs.truncate("/d", 0)),
    ("fallocate slozky", |fs| fs.fallocate("/d", 0, 4096, false)),
    ("set_times neexistuje", |fs| fs.set_times("/nic", Default::default())),
    ("set_owner neexistuje", |fs| fs.set_owner("/nic", 7)),
    ("set_acl neexistuje", |fs| fs.set_acl("/nic", None)),
    ("set_xattr neexistuje", |fs| fs.set_xattr("/nic", "user.a", b"1")),
    ("remove_xattr chybi", |fs| fs.remove_xattr("/f", "user.nic")),
    ("set_expires neexistuje", |fs| fs.set_expires("/nic", None)),
    ("trash neexistuje", |fs| fs.trash("/nic").map(|_| ())),
    ("trash_restore neexistuje", |fs| fs.trash_restore("/", 999_999, None).map(|_| ())),
    ("trash_purge neexistuje", |fs| fs.trash_purge("/", 999_999)),
    ("create_dataset existuje", |fs| fs.create_dataset("/ds").map(|_| ())),
    ("create_dataset na soubor", |fs| fs.create_dataset("/f").map(|_| ())),
    ("create_dataset bez rodice", |fs| fs.create_dataset("/nic/ds").map(|_| ())),
    ("delete_dataset neexistuje", |fs| fs.delete_dataset("/nic")),
    ("delete_dataset na slozku", |fs| fs.delete_dataset("/d")),
    ("clone_snapshot na existujici", |fs| fs.clone_snapshot("/ds", "S", "/f").map(|_| ())),
    ("clone_snapshot chybi snapshot", |fs| fs.clone_snapshot("/ds", "NIC", "/k").map(|_| ())),
    ("clone_path na existujici", |fs| fs.clone_path("/f", "/d").map(|_| ())),
    ("clone_path zdroj chybi", |fs| fs.clone_path("/nic", "/k").map(|_| ())),
    ("clone_range zdroj chybi", |fs| fs.clone_range("/nic", 0, "/f", 0, 4096)),
    ("snapshot_of existujici jmeno", |fs| fs.snapshot_of("/ds", "S").map(|_| ())),
    ("snapshot_of neni dataset", |fs| fs.snapshot_of("/d", "T").map(|_| ())),
    ("delete_snapshot_of chybi", |fs| fs.delete_snapshot_of("/ds", "NIC")),
    ("restore_copy na existujici", |fs| fs.restore_copy("/ds", "S", "/g", "/ds/g").map(|_| ())),
    ("restore_copy chybi snapshot", |fs| fs.restore_copy("/ds", "NIC", "/g", "/ds/h").map(|_| ())),
    ("restore_version chybi snapshot", |fs| {
        fs.restore_version("/ds/g", "NIC", RestoreMode::Replace).map(|_| ())
    }),
    ("commit_version neni dataset", |fs| fs.commit_version("/d", "m").map(|_| ())),
    ("branch na existujici cestu", |fs| fs.branch("/ds", "b", "/f").map(|_| ())),
    ("delete_branch chybi", |fs| fs.delete_branch("/ds", "nic")),
    ("release_view chybi", |fs| fs.release_view("/ds", "nic")),
    ("merge neexistujici zdroj", |fs| fs.merge("/ds", "/nic").map(|_| ())),
    ("aside_begin na existujici", |fs| fs.aside_begin("/ds", "/f").map(|_| ())),
    ("aside_apply neni klon", |fs| fs.aside_apply("/d", None).map(|_| ())),
    ("aside_discard neni klon", |fs| fs.aside_discard("/d").map(|_| ())),
    ("undo neznamy krok", |fs| fs.undo("/ds", 999_999).map(|_| ())),
];

#[test]
fn logical_errors_keep_volume_writable() {
    let mut bad: Vec<String> = Vec::new();
    for (i, (name, op)) in CASES.iter().enumerate() {
        let mut fs = new_fs(8192);
        setup(&mut fs);
        let r = op(&mut fs);
        let e = match r {
            Ok(()) => {
                bad.push(format!("{name}: neocekavane uspelo"));
                continue;
            }
            Err(e) => e,
        };
        if e == Error::IoError {
            bad.push(format!("{name}: vratilo IO_ERROR"));
            continue;
        }
        // Svazek musi zustat zapisovatelny: zapis a commit.
        let p = format!("/po-{i}");
        let after = fs
            .create(&p, Kind::File)
            .and_then(|_| fs.write(&p, 0, b"po chybe"))
            .and_then(|_| fs.commit());
        if let Err(after) = after {
            bad.push(format!("{name}: chyba {e:?}, pak zapis {after:?}"));
        }
    }
    assert!(bad.is_empty(), "operace prepinaji svazek do stavu chyby:\n{}", bad.join("\n"));
}

/// Plny svazek: zapis, klon a dataset skonci `NO_SPACE` (ne stavem chyby)
/// a po uvolneni mista jde zase zapisovat a commitovat.
#[test]
fn no_space_keeps_volume_writable() {
    let mut fs = new_fs(2048);
    setup(&mut fs);
    let chunk = std::vec![0x5Au8; 256 * 1024];
    let mut n = 0;
    let err = loop {
        let p = format!("/plny-{n}");
        let r = fs.create(&p, Kind::File).and_then(|_| fs.write(&p, 0, &chunk)).and_then(|_| fs.commit());
        match r {
            Ok(()) => n += 1,
            Err(e) => break e,
        }
        assert!(n < 1000, "svazek se nezaplnil");
    };
    assert_eq!(err, Error::NoSpace, "plny svazek");
    for (name, r) in [
        ("klon", fs.clone_path("/plny-0", "/plny-klon").map(|_| ())),
        ("dataset", fs.create_dataset("/plny-ds").map(|_| ())),
    ] {
        assert_ne!(r, Err(Error::IoError), "{name} na plnem svazku");
    }
    for i in 0..n {
        let _ = fs.remove(&format!("/plny-{i}"));
    }
    fs.commit().unwrap();
    fs.create("/po-uvolneni", Kind::File).unwrap();
    fs.write("/po-uvolneni", 0, b"ok").unwrap();
    fs.commit().unwrap();
}
