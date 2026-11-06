//! Format v3: zaznam objektu, dlouha jmena, symbolicke odkazy, limity.

use abi::Error;

use super::{assert_clean, new_fs, remount};
use crate::layout::{INODE_SIZE, Inode, KIND_SYMLINK, MAX_FILE_SIZE, SYMLINK_MAX};
use crate::{Kind, NAME_MAX};

#[test]
fn inode_roundtrip() {
    let i = Inode {
        kind: KIND_SYMLINK,
        flags: 3,
        acl: 7,
        size: 12,
        mtime: 1,
        ctime: 2,
        btime: 3,
        atime: 4,
        parent: 5,
        generation: 6,
        tree_bytes: 8,
        tree_files: 9,
        tree_dirs: 10,
        owner: 11,
        xattrs: 13,
    };
    let b = i.encode();
    assert_eq!(b.len(), INODE_SIZE);
    assert_eq!(Inode::decode(&b).unwrap(), i);
    // Neznamy druh a prilis velky soubor = poskozeny zaznam.
    let mut bad = b.clone();
    bad[0] = 9;
    assert_eq!(Inode::decode(&bad), Err(Error::Corrupted));
    let mut big = Inode { kind: 1, size: MAX_FILE_SIZE + 1, ..i }.encode();
    assert_eq!(Inode::decode(&big), Err(Error::Corrupted));
    big[8..16].copy_from_slice(&MAX_FILE_SIZE.to_le_bytes());
    assert!(Inode::decode(&big).is_ok());
}

#[test]
fn long_names() {
    let mut fs = new_fs(4096);
    let long = "ř".repeat(1024);
    let path = alloc::format!("/{long}");
    fs.create(&path, Kind::File).unwrap();
    fs.write(&path, 0, b"obsah").unwrap();
    let path2 = alloc::format!("/{}", "\u{1F600}".repeat(1024));
    fs.create(&path2, Kind::Dir).unwrap();
    assert_eq!(
        fs.create(&alloc::format!("/{}", "x".repeat(1025)), Kind::File).err(),
        Some(Error::InvalidArgs)
    );
    fs.commit().unwrap();
    let mut fs = remount(fs);
    let names: alloc::vec::Vec<_> = fs.list("/").unwrap().into_iter().map(|e| e.name).collect();
    assert!(names.contains(&long));
    assert!(names.iter().all(|n| n.len() <= NAME_MAX));
    assert_eq!(fs.read(&path, 0, 10).unwrap(), b"obsah");
    assert_clean(&mut fs);
}

#[test]
fn bidi_names_rejected() {
    let mut fs = new_fs(1024);
    for name in ["/faktura\u{202E}fdp.exe", "/a\u{2066}b", "/a\u{202A}"] {
        assert_eq!(fs.create(name, Kind::File).err(), Some(Error::InvalidArgs));
        assert_eq!(fs.symlink(name, "/x").err(), Some(Error::InvalidArgs));
    }
    fs.create("/ok", Kind::File).unwrap();
    assert_eq!(fs.rename("/ok", "/x\u{2069}").err(), Some(Error::InvalidArgs));
}

#[test]
fn symlinks() {
    let mut fs = new_fs(2048);
    fs.create("/d", Kind::Dir).unwrap();
    fs.symlink("/d/odkaz", "../cil/neexistuje").unwrap();
    let st = fs.stat("/d/odkaz").unwrap();
    assert_eq!(st.kind, Kind::Symlink);
    assert_eq!(st.size, "../cil/neexistuje".len() as u64);
    assert_eq!(fs.readlink("/d/odkaz").unwrap(), "../cil/neexistuje");
    // Odkaz se pri prochazeni nenasleduje a neni to soubor.
    assert_eq!(fs.readlink("/d").err(), Some(Error::WrongType));
    assert_eq!(fs.read("/d/odkaz", 0, 1).err(), Some(Error::WrongType));
    assert_eq!(fs.write("/d/odkaz", 0, b"x").err(), Some(Error::WrongType));
    assert_eq!(fs.stat("/d/odkaz/x").err(), Some(Error::NotADirectory));
    assert_eq!(fs.stat("/d").unwrap().tree_files, 1);
    assert_eq!(fs.symlink("/d/odkaz", "y").err(), Some(Error::AlreadyExists));
    assert_eq!(fs.symlink("/d/prazdny", "").err(), Some(Error::InvalidArgs));
    assert_eq!(fs.symlink("/d/nul", "a\0b").err(), Some(Error::InvalidArgs));
    let max = "t".repeat(SYMLINK_MAX);
    fs.symlink("/d/dlouhy", &max).unwrap();
    assert_eq!(fs.symlink("/d/delsi", &alloc::format!("{max}t")).err(), Some(Error::InvalidArgs));
    fs.rename("/d/dlouhy", "/presunuty").unwrap();
    assert_eq!(fs.stat("/").unwrap().tree_files, 2);
    fs.commit().unwrap();
    let mut fs = remount(fs);
    assert_eq!(fs.readlink("/presunuty").unwrap(), max);
    fs.remove("/presunuty").unwrap();
    assert_eq!(fs.stat("/").unwrap().tree_files, 1);
    assert_clean(&mut fs);
}

#[test]
fn symlink_survives_fsync() {
    let mut fs = new_fs(2048);
    fs.commit().unwrap();
    fs.symlink("/odkaz", "/nekam").unwrap();
    fs.fsync().unwrap();
    // Bez commitu: odkaz se musi obnovit z intent logu.
    let mut fs = remount(fs);
    assert_eq!(fs.info().replayed, 1);
    assert_eq!(fs.readlink("/odkaz").unwrap(), "/nekam");
    assert_eq!(fs.stat("/odkaz").unwrap().kind, Kind::Symlink);
    assert_clean(&mut fs);
}

#[test]
fn file_size_limit() {
    let mut fs = new_fs(2048);
    fs.create("/ridky", Kind::File).unwrap();
    assert_eq!(fs.truncate("/ridky", MAX_FILE_SIZE + 1).err(), Some(Error::OutOfRange));
    assert_eq!(fs.write("/ridky", MAX_FILE_SIZE, b"x").err(), Some(Error::OutOfRange));
    assert_eq!(fs.write("/ridky", u64::MAX, b"x").err(), Some(Error::OutOfRange));
    // Obri ridky soubor: posledni bajt jde zapsat i precist.
    fs.write("/ridky", MAX_FILE_SIZE - 1, b"z").unwrap();
    assert_eq!(fs.stat("/ridky").unwrap().size, MAX_FILE_SIZE);
    assert_eq!(fs.read("/ridky", MAX_FILE_SIZE - 1, 8).unwrap(), b"z");
    assert_eq!(fs.read("/ridky", 0, 4).unwrap(), [0; 4]);
    // Druhy obri soubor: souhrn adresare saturuje, nepretece.
    fs.create("/ridky2", Kind::File).unwrap();
    fs.truncate("/ridky2", MAX_FILE_SIZE).unwrap();
    fs.truncate("/ridky2", MAX_FILE_SIZE).unwrap();
    fs.create("/ridky3", Kind::File).unwrap();
    fs.truncate("/ridky3", MAX_FILE_SIZE).unwrap();
    assert_eq!(fs.stat("/").unwrap().tree_bytes, u64::MAX);
    fs.commit().unwrap();
    let mut fs = remount(fs);
    assert_eq!(fs.read("/ridky", MAX_FILE_SIZE - 1, 1).unwrap(), b"z");
    // Saturace je trvala: zmenseni nesmi dat mensi soucet, nez je pravda.
    fs.truncate("/ridky", 10).unwrap();
    assert_eq!(fs.stat("/").unwrap().tree_bytes, u64::MAX);
    assert_clean(&mut fs);
}

#[test]
fn created_objects_record_generation() {
    let mut fs = new_fs(1024);
    fs.create("/a", Kind::File).unwrap();
    let g1 = fs.stat("/a").unwrap().generation;
    fs.commit().unwrap();
    fs.remove("/a").unwrap();
    fs.create("/a", Kind::File).unwrap();
    let st = fs.stat("/a").unwrap();
    assert!(st.generation > g1);
    assert_eq!(st.btime, st.mtime);
}

#[test]
fn generation_survives_replay() {
    let mut fs = new_fs(2048);
    fs.commit().unwrap();
    fs.create("/f", Kind::File).unwrap();
    fs.symlink("/l", "f").unwrap();
    let (gf, gl) = (fs.stat("/f").unwrap().generation, fs.stat("/l").unwrap().generation);
    fs.fsync().unwrap();
    let mut fs = remount(fs);
    assert_eq!(fs.stat("/f").unwrap().generation, gf);
    assert_eq!(fs.stat("/l").unwrap().generation, gl);
}

#[test]
fn fragmented_free_space_does_not_hang() {
    // Disk zaplneny soubory po jednom bloku, pak smazany kazdy druhy: volne
    // misto jsou useky po 1 bloku, do kterych se uzel (4 bloky) nevejde.
    let mut fs = new_fs(1024);
    let block = alloc::vec![7u8; crate::BLOCK_SIZE];
    let mut n = 0;
    loop {
        let p = alloc::format!("/f{n}");
        if fs.create(&p, Kind::File).is_err() || fs.write(&p, 0, &block).is_err() {
            break;
        }
        // Rezerva commitu musi stacit vzdy (`ensure_room`).
        fs.commit().unwrap();
        n += 1;
    }
    for i in (0..n).step_by(2) {
        let _ = fs.remove(&alloc::format!("/f{i}"));
    }
    // Musi skoncit (uspech nebo NO_SPACE), ne se zacyklit.
    let r = fs.commit();
    assert!(matches!(r, Ok(()) | Err(Error::NoSpace)), "{r:?}");
}
