//! Testy zamku obrazu: nejvys jeden zapisovatel, cteni vedle nej jen
//! s upozornenim.

use std::path::PathBuf;

use fs::{Algo, Kind};
use osfs::image::{create_image, open};

/// Novy obraz v docasnem adresari (jmeno podle testu, at se nepletou).
fn temp_image(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join("osfs-test");
    std::fs::create_dir_all(&dir).unwrap();
    let p = dir.join(format!("{name}-{}.img", std::process::id()));
    let _ = std::fs::remove_file(&p);
    create_image(&p, 8 << 20, "test", Algo::Xxh3).unwrap();
    p
}

#[test]
fn second_writer_refused() {
    let p = temp_image("writer");
    let (mut fs, lock) = open(&p, false).unwrap();
    assert!(lock.is_some());
    // Druhy zapisovatel (jako `osfs put` vedle `osfs mount`) je odmitnut.
    let err = open(&p, false).err().expect("druhy zapisovatel ma selhat");
    assert!(err.contains("jiny proces osfs"), "{err}");
    // Cteni projde, jen bez zamku (upozorni na stderr).
    let (_ro, ro_lock) = open(&p, true).unwrap();
    assert!(ro_lock.is_none());
    // Zamek neblokuje vlastni vstup/vystup svazku.
    fs.create("/a.txt", Kind::File).unwrap();
    fs.write("/a.txt", 0, b"ahoj").unwrap();
    fs.commit().unwrap();
    drop(fs);
    drop(lock);
    // Po uvolneni jde zapisovatel znovu.
    let (mut fs, lock) = open(&p, false).unwrap();
    assert!(lock.is_some());
    assert_eq!(fs.read("/a.txt", 0, 4).unwrap(), b"ahoj");
    drop(fs);
    drop(lock);
    std::fs::remove_file(&p).unwrap();
}

#[test]
fn readers_share() {
    let p = temp_image("readers");
    let (_a, la) = open(&p, true).unwrap();
    let (_b, lb) = open(&p, true).unwrap();
    assert!(la.is_some() && lb.is_some());
    // Zapisovatel ceka, az ctenari skonci.
    assert!(open(&p, false).is_err());
    drop((_a, la, _b, lb));
    std::fs::remove_file(&p).unwrap();
}
