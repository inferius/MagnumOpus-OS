//! Hodnoty z disku se nesmi brat na viru: poskozeny (ale soucet sedi -
//! chyba softwaru) extent, obsah maleho souboru nebo cyklus rodicu musi
//! skoncit `CORRUPTED`, ne ctenim mimo, dvojim uvolnenim nebo zacyklenim.

use abi::Error;

use super::{new_fs, remount};
use crate::layout::{Extent, Key, item};
use crate::{Kind, hash::Csum};

#[test]
fn extent_outside_volume_is_corrupted() {
    let mut fs = new_fs(1024);
    fs.create("/f", Kind::File).unwrap();
    fs.truncate("/f", 3 * 4096).unwrap();
    let (oid, _) = fs.test_inode("/f").unwrap();
    let bad = Extent { phys: 5000, birth: 1, csums: alloc::vec![Csum::default(); 3] };
    fs.test_put(Key::new(oid, item::EXTENT, 0), bad.encode()).unwrap();
    assert_eq!(fs.read("/f", 0, 10).err(), Some(Error::Corrupted));
    assert_eq!(fs.truncate("/f", 0).err(), Some(Error::Corrupted));
}

#[test]
fn freeing_free_blocks_poisons_instead_of_double_free() {
    let mut fs = new_fs(1024);
    fs.create("/f", Kind::File).unwrap();
    fs.truncate("/f", 3 * 4096).unwrap();
    fs.commit().unwrap();
    let (oid, _) = fs.test_inode("/f").unwrap();
    // Extent ve svazku, ale na volnych blocich (blizko konce disku).
    let bad = Extent { phys: 1000, birth: 1, csums: alloc::vec![Csum::default(); 3] };
    fs.test_put(Key::new(oid, item::EXTENT, 0), bad.encode()).unwrap();
    // Zkraceni by bloky "uvolnilo" podruhe: odmitnuto, svazek ve stavu chyby.
    assert_eq!(fs.truncate("/f", 0).err(), Some(Error::Corrupted));
    assert!(fs.commit().is_err());
    assert_eq!(fs.create("/g", Kind::File).err(), Some(Error::IoError));
    // Na disku zustal posledni commit.
    let mut fs = remount(fs);
    assert_eq!(fs.stat("/f").unwrap().size, 3 * 4096);
}

#[test]
fn inline_length_mismatch_is_corrupted() {
    let mut fs = new_fs(1024);
    fs.create("/m", Kind::File).unwrap();
    fs.write("/m", 0, b"0123456789").unwrap();
    let (oid, _) = fs.test_inode("/m").unwrap();
    fs.test_put(Key::new(oid, item::INLINE, 0), b"01234".to_vec()).unwrap();
    assert_eq!(fs.read("/m", 0, 10).err(), Some(Error::Corrupted));
    assert_eq!(fs.write("/m", 2, b"x").err(), Some(Error::Corrupted));
}

#[test]
fn parent_cycle_is_corrupted_and_operation_rolled_back() {
    let mut fs = new_fs(1024);
    fs.create("/a", Kind::Dir).unwrap();
    fs.create("/a/b", Kind::Dir).unwrap();
    fs.commit().unwrap();
    // Cyklus: rodic /a je /a/b.
    let (a, mut ia) = fs.test_inode("/a").unwrap();
    let (b, _) = fs.test_inode("/a/b").unwrap();
    ia.parent = b;
    fs.test_put(Key::new(a, item::INODE, 0), ia.encode()).unwrap();
    // Vytvoreni projde cast zmen a pak narazi na cyklus pri souhrnech:
    // CORRUPTED a stav chyby (napul hotova operace se nepotvrdi).
    assert_eq!(fs.create("/a/b/c", Kind::File).err(), Some(Error::Corrupted));
    assert_eq!(fs.fsync().err(), Some(Error::IoError));
    // Presun do vlastniho podstromu s cyklem take skonci, nezacykli se.
    let mut fs = remount(fs);
    assert!(fs.stat("/a/b").is_ok());
    assert_eq!(fs.stat("/a/b/c").err(), Some(Error::NotFound));
}
