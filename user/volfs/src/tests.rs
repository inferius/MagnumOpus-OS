use std::io::{Cursor, Read};

use volproto::{Error, Info, Reader, Stat, kind, op, parse_reply, request};

use crate::dev::MemDisk;
use crate::fat::FatVolume;
use crate::serve::handle;
use crate::{Volume, time};

const MIB: usize = 1024 * 1024;

/// Obraz FAT32 (nebo FAT16 pro male) naformatovany crate fatfs.
fn fat_image(size: usize, label: &[u8; 11]) -> Vec<u8> {
    let mut img = vec![0u8; size];
    let opts = fatfs::FormatVolumeOptions::new().volume_label(*label);
    fatfs::format_volume(Cursor::new(&mut img[..]), opts).unwrap();
    img
}

fn mount(img: Vec<u8>, ro: bool) -> FatVolume<MemDisk> {
    FatVolume::new(MemDisk { data: img, ss: 512, writes: 0 }, ro).unwrap()
}

/// Pozadavek pres protokol; vrati telo odpovedi nebo chybu.
fn call(vol: &mut dyn Volume, opcode: u32, args: impl FnOnce(&mut volproto::Writer)) -> Result<Vec<u8>, Error> {
    let mut w = request(9, opcode);
    args(&mut w);
    let out = handle(vol, &w.finish());
    let (id, r) = parse_reply(&out.reply).unwrap();
    assert_eq!(id, 9);
    r.map(|b| b.to_vec())
}

fn stat(vol: &mut dyn Volume, p: &str) -> Result<Stat, Error> {
    call(vol, op::STAT, |w| {
        w.str(p);
    })
    .map(|b| Stat::get(&mut Reader::new(&b)).unwrap())
}

fn list(vol: &mut dyn Volume, p: &str) -> Vec<String> {
    let b = call(vol, op::LIST, |w| {
        w.str(p);
    })
    .unwrap();
    let mut r = Reader::new(&b);
    let n = r.u32().unwrap();
    let mut v: Vec<String> = (0..n)
        .map(|_| {
            let name = r.str().unwrap();
            Stat::get(&mut r).unwrap();
            name
        })
        .collect();
    v.sort();
    v
}

/// Precte soubor nezavisle (cerstve pripojeni fatfs nad bajty obrazu).
fn host_read(img: &[u8], path: &str) -> Vec<u8> {
    let fs = fatfs::FileSystem::new(Cursor::new(img.to_vec()), fatfs::FsOptions::new()).unwrap();
    let mut f = fs.root_dir().open_file(path).unwrap();
    let mut v = Vec::new();
    f.read_to_end(&mut v).unwrap();
    v
}

#[test]
fn fat_long_names_write_read_rename_remove() {
    let mut vol = mount(fat_image(64 * MIB, b"FLASHKA    "), false);
    let info = call(&mut vol, op::INFO, |_| {}).map(|b| Info::get(&mut Reader::new(&b)).unwrap()).unwrap();
    assert_eq!((info.fs.as_str(), info.label.as_str(), info.read_only), ("fat", "FLASHKA", false));
    assert!(info.free_bytes > 60 * MIB as u64 && info.total_bytes >= info.free_bytes);

    call(&mut vol, op::CREATE, |w| {
        w.str("/Dokumenty").u8(kind::DIR).u8(1);
    })
    .unwrap();
    let name = "/Dokumenty/Přílišžluťoučký kůň - dlouhé jméno.txt";
    call(&mut vol, op::CREATE, |w| {
        w.str(name).u8(kind::FILE).u8(1);
    })
    .unwrap();
    assert_eq!(
        call(&mut vol, op::CREATE, |w| {
            w.str(name).u8(kind::FILE).u8(1);
        }),
        Err(Error::AlreadyExists),
        "exkluzivne"
    );
    // Zapis ve dvou kusech s dirou.
    let data: Vec<u8> = (0..70_000u32).map(|i| (i % 251) as u8).collect();
    call(&mut vol, op::WRITE, |w| {
        w.str(name).u64(0).bytes(&data[..30_000]);
    })
    .unwrap();
    call(&mut vol, op::WRITE, |w| {
        w.str(name).u64(30_000).bytes(&data[30_000..60_000]);
    })
    .unwrap();
    call(&mut vol, op::WRITE, |w| {
        w.str(name).u64(65_000).bytes(&data[65_000..]);
    })
    .unwrap();
    let st = stat(&mut vol, name).unwrap();
    assert_eq!((st.kind, st.size), (kind::FILE, 70_000));
    let back = call(&mut vol, op::READ, |w| {
        w.str(name).u64(29_990).u32(20);
    })
    .unwrap();
    assert_eq!(back, data[29_990..30_010]);
    // Velikost pismen se pri hledani nerozlisuje.
    assert_eq!(stat(&mut vol, "/dokumenty/PŘÍLIŠŽLUŤOUČKÝ KŮŇ - DLOUHÉ JMÉNO.TXT").map(|s| s.size), Ok(70_000));
    assert_eq!(list(&mut vol, "/Dokumenty"), ["Přílišžluťoučký kůň - dlouhé jméno.txt"]);

    // Obsah na mediu hned po operaci (bez SYNC): nezavisle cteni obrazu.
    let img = vol.device().with_dev(|d| d.data.clone());
    let host = host_read(&img, &name[1..]);
    assert_eq!(&host[..60_000], &data[..60_000]);
    assert!(host[60_000..65_000].iter().all(|&b| b == 0), "dira = nuly");

    // Prejmenovani, zmenseni, smazani.
    call(&mut vol, op::RENAME, |w| {
        w.str(name).str("/kratke.txt").u8(0);
    })
    .unwrap();
    call(&mut vol, op::SET_LEN, |w| {
        w.str("/kratke.txt").u64(10);
    })
    .unwrap();
    assert_eq!(stat(&mut vol, "/kratke.txt").map(|s| s.size), Ok(10));
    assert_eq!(
        call(&mut vol, op::REMOVE, |w| {
            w.str("/Dokumenty");
        }),
        Ok(Vec::new()),
        "prazdna slozka jde smazat"
    );
    call(&mut vol, op::CREATE, |w| {
        w.str("/a.txt").u8(kind::FILE).u8(0);
    })
    .unwrap();
    assert_eq!(
        call(&mut vol, op::RENAME, |w| {
            w.str("/a.txt").str("/kratke.txt").u8(0);
        }),
        Err(Error::AlreadyExists)
    );
    call(&mut vol, op::RENAME, |w| {
        w.str("/a.txt").str("/kratke.txt").u8(1);
    })
    .unwrap();
    assert_eq!(stat(&mut vol, "/kratke.txt").map(|s| s.size), Ok(0), "nahrazeno");
    call(&mut vol, op::UNMOUNT, |_| {}).unwrap();
    let img = vol.device().with_dev(|d| d.data.clone());
    assert_eq!(host_read(&img, "kratke.txt"), b"");
}

#[test]
fn fat_errors_and_read_only() {
    let mut vol = mount(fat_image(16 * MIB, b"NO NAME    "), false);
    assert_eq!(stat(&mut vol, "/neni"), Err(Error::NotFound));
    assert_eq!(
        call(&mut vol, op::READ, |w| {
            w.str("/a/../b").u64(0).u32(1);
        }),
        Err(Error::InvalidArgs)
    );
    call(&mut vol, op::CREATE, |w| {
        w.str("/d").u8(kind::DIR).u8(0);
    })
    .unwrap();
    call(&mut vol, op::CREATE, |w| {
        w.str("/d/f").u8(kind::FILE).u8(0);
    })
    .unwrap();
    assert_eq!(
        call(&mut vol, op::REMOVE, |w| {
            w.str("/d");
        }),
        Err(Error::NotEmpty)
    );
    let img = vol.device().with_dev(|d| d.data.clone());
    let mut ro = mount(img, true);
    assert_eq!(list(&mut ro, "/"), ["d"]);
    assert_eq!(
        call(&mut ro, op::WRITE, |w| {
            w.str("/d/f").u64(0).bytes(b"x");
        }),
        Err(Error::ReadOnly)
    );
    assert_eq!(call(&mut ro, 999, |_| {}), Err(Error::NotSupported));
}

#[test]
fn fat_full_disk_is_no_space() {
    let mut vol = mount(fat_image(4 * MIB, b"MALY       "), false);
    call(&mut vol, op::CREATE, |w| {
        w.str("/velky").u8(kind::FILE).u8(0);
    })
    .unwrap();
    let chunk = vec![7u8; volproto::MAX_IO];
    let mut off = 0u64;
    let err = loop {
        match call(&mut vol, op::WRITE, |w| {
            w.str("/velky").u64(off).bytes(&chunk);
        }) {
            Ok(_) => off += chunk.len() as u64,
            Err(e) => break e,
        }
        assert!(off < 8 * MIB as u64, "zapis za kapacitu media");
    };
    assert_eq!(err, Error::NoSpace);
}

#[test]
fn damaged_images_do_not_hang() {
    // Nahodne poskozeny obraz: pripojeni nebo operace selze, nebo projde;
    // panika je v procesu media pripustna (medium zmizi), zacykleni ne.
    let base = fat_image(8 * MIB, b"POSKOZENY  ");
    let mut seed = 0x1234_5678u64;
    for round in 0..24 {
        let mut img = base.clone();
        for _ in 0..(16 + round * 8) {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            let at = (seed as usize) % (512 * 64);
            img[at] = (seed >> 32) as u8;
        }
        let r = std::panic::catch_unwind(move || {
            if let Ok(mut vol) = FatVolume::new(MemDisk { data: img, ss: 512, writes: 0 }, false) {
                let _ = vol.list("/");
                let _ = vol.info();
                let _ = vol.create("/x.txt", kind::FILE, false);
                let _ = vol.write("/x.txt", 0, b"abc");
                let _ = vol.read("/x.txt", 0, 3);
            }
        });
        let _ = r;
    }
}

#[test]
fn fat_time_roundtrip() {
    let t = 1_763_300_000i64; // 2025-11-16
    assert_eq!(time::from_fat(time::to_fat(t)), t - t % 2, "FAT: rozliseni 2 s");
    assert_eq!(time::days_from_civil(1970, 1, 1), 0);
    assert_eq!(time::civil_from_days(time::days_from_civil(2024, 2, 29)), (2024, 2, 29));
    assert_eq!(time::from_ntfs(116_444_736_000_000_000), 0);
}

/// Testovaci obraz NTFS (third_party/test-images/ntfs, viz README).
fn ntfs_image() -> Vec<u8> {
    std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../../third_party/test-images/ntfs/testfs1")).unwrap()
}

#[test]
fn ntfs_read_only() {
    let mut vol = crate::ntfs::NtfsVolume::new(MemDisk { data: ntfs_image(), ss: 512, writes: 0 }).unwrap();
    let info = vol.info().unwrap();
    assert_eq!((info.fs.as_str(), info.label.as_str(), info.read_only), ("ntfs", "mylabel", true));
    let names = list(&mut vol, "/");
    for n in ["1000-bytes-file", "empty-file", "file-with-12345", "many_subdirs", "sparse-file"] {
        assert!(names.iter().any(|x| x == n), "{n} ve vypisu {names:?}");
    }
    assert!(!names.iter().any(|n| n.starts_with('$')), "systemove soubory skryte");
    assert_eq!(vol.read("/file-with-12345", 0, 100).unwrap(), b"12345");
    assert_eq!(vol.read("/1000-bytes-file", 995, 100).unwrap(), b"12345");
    let st = stat(&mut vol, "/1000-bytes-file").unwrap();
    assert_eq!((st.kind, st.size, st.read_only), (kind::FILE, 1000, true));
    // Rezny soubor: konec dat a nuly mezi.
    assert_eq!(vol.read("/sparse-file", 500_000, 5).unwrap(), b"11111");
    assert_eq!(vol.read("/sparse-file", 100, 4).unwrap(), [0; 4]);
    // empty-file: zmena 2021-01-01 13:37 (mistni cas skriptu = UTC v testu).
    let st = stat(&mut vol, "/empty-file").unwrap();
    assert_eq!((st.kind, st.size), (kind::FILE, 0));
    assert_eq!(list(&mut vol, "/many_subdirs").len(), 512);
    assert_eq!(stat(&mut vol, "/MANY_SUBDIRS/512").map(|s| s.kind), Ok(kind::DIR), "velikost pismen");
    assert_eq!(stat(&mut vol, "/neni"), Err(Error::NotFound));
    assert_eq!(vol.write("/file-with-12345", 0, b"x"), Err(Error::ReadOnly));
    assert_eq!(vol.create("/novy", kind::FILE, false), Err(Error::ReadOnly));
}

#[test]
fn ntfs_damaged_does_not_hang() {
    let base = ntfs_image();
    let mut seed = 0x9e37_79b9u64;
    for round in 0..16 {
        let mut img = base.clone();
        for _ in 0..(32 + round * 16) {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            let at = (seed as usize) % img.len();
            img[at] = (seed >> 24) as u8;
        }
        let _ = std::panic::catch_unwind(move || {
            if let Ok(mut vol) = crate::ntfs::NtfsVolume::new(MemDisk { data: img, ss: 512, writes: 0 }) {
                let _ = vol.list("/");
                let _ = vol.list("/many_subdirs");
                let _ = vol.read("/1000-bytes-file", 0, 1000);
                let _ = vol.info();
            }
        });
    }
}
