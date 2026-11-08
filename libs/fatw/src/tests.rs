//! Testy zapisu FAT32: vysledek cte crate `fatfs` (nezavisla implementace).

use std::io::{Cursor, Read, Write};

use super::*;

struct Mem(Vec<u8>);

impl Device for Mem {
    fn read(&mut self, lba: u64, buf: &mut [u8]) -> Result<()> {
        let o = lba as usize * SECTOR;
        buf.copy_from_slice(self.0.get(o..o + buf.len()).ok_or(Error::Io)?);
        Ok(())
    }
    fn write(&mut self, lba: u64, data: &[u8]) -> Result<()> {
        let o = lba as usize * SECTOR;
        self.0.get_mut(o..o + data.len()).ok_or(Error::Io)?.copy_from_slice(data);
        Ok(())
    }
    fn flush(&mut self) -> Result<()> {
        Ok(())
    }
}

/// FAT32 64 MiB s cizim souborem v korenu.
fn image() -> Vec<u8> {
    let mut img = vec![0u8; 64 << 20];
    let opts = fatfs::FormatVolumeOptions::new().fat_type(fatfs::FatType::Fat32).volume_label(*b"MO-LOGY    ");
    fatfs::format_volume(Cursor::new(&mut img[..]), opts).unwrap();
    {
        let fs = fatfs::FileSystem::new(Cursor::new(&mut img[..]), fatfs::FsOptions::new()).unwrap();
        let mut f = fs.root_dir().create_file("cizi.txt").unwrap();
        f.write_all(b"cizi data").unwrap();
    }
    img
}

fn read(img: &mut [u8], path: &str) -> Vec<u8> {
    let fs = fatfs::FileSystem::new(Cursor::new(img), fatfs::FsOptions::new()).unwrap();
    let mut f = fs.root_dir().open_file(path).unwrap();
    let mut v = Vec::new();
    f.read_to_end(&mut v).unwrap();
    v
}

#[test]
fn names() {
    assert_eq!(&short_name("boot42.log").unwrap(), b"BOOT42  LOG");
    assert_eq!(&short_name("MAGNUMOS").unwrap(), b"MAGNUMOS   ");
    assert!(short_name("prilisdlouhe.txt").is_err());
    assert!(short_name("a b.txt").is_err());
    assert!(short_name("x.toolong").is_err());
}

#[test]
fn create_overwrite_and_keep_foreign() {
    let t = Stamp::new(2026, 11, 8, 12, 30, 10);
    let mut v = Volume::open(Mem(image())).unwrap();
    let big: Vec<u8> = (0..200_000u32).map(|i| (i % 251) as u8).collect();
    v.write_file("MAGNUMOS", "REPORT.TXT", &big, t).unwrap();
    v.write_file("MAGNUMOS", "BOOT1.LOG", b"log 1\n", t).unwrap();
    // Prepis kratsim obsahem (stary retez se uvolni).
    v.write_file("MAGNUMOS", "REPORT.TXT", b"kratsi", t).unwrap();
    v.write_file("MAGNUMOS", "EMPTY.TXT", b"", t).unwrap();
    let mut img = v.into_device().0;
    assert_eq!(read(&mut img, "MAGNUMOS/REPORT.TXT"), b"kratsi");
    assert_eq!(read(&mut img, "MAGNUMOS/BOOT1.LOG"), b"log 1\n");
    assert_eq!(read(&mut img, "MAGNUMOS/EMPTY.TXT"), b"");
    assert_eq!(read(&mut img, "cizi.txt"), b"cizi data");
    // Uvolnene misto: fatfs spocita volne clustery a zadny nechybi navic.
    let fs = fatfs::FileSystem::new(Cursor::new(&mut img[..]), fatfs::FsOptions::new()).unwrap();
    let st = fs.stats().unwrap();
    // Obsazeno: koren, cizi soubor, slozka, 2 male soubory = 5 clusteru.
    assert_eq!(st.total_clusters() - st.free_clusters(), 5);
}

#[test]
fn many_files_extend_directory() {
    let t = Stamp::default();
    let mut v = Volume::open(Mem(image())).unwrap();
    for i in 0..300 {
        v.write_file("MAGNUMOS", &format!("F{i}.TXT"), format!("soubor {i}").as_bytes(), t).unwrap();
    }
    let mut img = v.into_device().0;
    for i in [0, 150, 299] {
        assert_eq!(read(&mut img, &format!("MAGNUMOS/F{i}.TXT")), format!("soubor {i}").as_bytes());
    }
}

#[test]
fn rejects_non_fat32() {
    assert_eq!(Volume::open(Mem(vec![0u8; 1 << 20])).err(), Some(Error::NotFat32));
}
