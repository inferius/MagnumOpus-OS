//! Disky vymennych medii pro testy (docs/cs/media.md): flash disky USB
//! (`usb-storage`, `removable=on`) s FAT32, exFAT a NTFS pri startu, superfloppy
//! FAT pro pripojeni za behu (QMP `device_add`), a kontrola na hostiteli
//! po testu: co test zapsal na FAT, musi byt v obrazu.
//!
//! Obsah musi sedet s testem `user/init/src/tests/removable.rs`.

use std::io::{Cursor, Read, Write};
use std::path::{Path, PathBuf};

use crate::Result;

const MIB: usize = 1024 * 1024;
/// Prvni sektor oddilu na disku s MBR.
const PART_LBA: usize = 2048;

/// Jmenovky a obsah (stejne konstanty v testu initu).
pub const FAT_LABEL: &str = "MOTEST";
pub const HOT_LABEL: &str = "HOTPLUG";
/// Soubor s dlouhym ceskym jmenem pripraveny na hostiteli.
pub const FAT_FILE: &str = "Přílišžluťoučký kůň.txt";
pub const FAT_TEXT: &str = "ahoj z hostitele\n";
/// Soubor, ktery zapise test (overi se tady po behu).
pub const GUEST_FILE: &str = "Zapsáno v systému/z-testu.txt";
pub const GUEST_TEXT: &str = "zapis z MagnumOpus\n";
/// Znacka v logu: test soubor zapsal a medium bezpecne odpojil.
pub const GUEST_MARK: &str = "media: zapsano a odpojeno";

/// exFAT: jmenovka, soubor z hostitele, soubor zapsany systemem (dlouhe
/// jmeno ve vnorene slozce, vic clusteru) a znacka v logu.
pub const EXFAT_LABEL: &str = "EXTEST";
pub const EXFAT_FILE: &str = "Dokumenty/Žluťoučký kůň - exFAT.txt";
pub const EXFAT_TEXT: &str = "ahoj z hostitele na exFAT\n";
pub const EXFAT_GUEST: &str = "Zapsáno v systému/Velmi dlouhé jméno souboru zapsaného systémem MagnumOpus.bin";
pub const EXFAT_GUEST_LEN: usize = 200_000;
pub const EXFAT_MARK: &str = "media: exfat zapsano a odpojeno";

/// Obsah souboru zapsaneho systemem (musi sedet s testem initu).
pub fn exfat_guest_byte(i: usize) -> u8 {
    (i * 31 % 251) as u8
}

fn label11(s: &str) -> [u8; 11] {
    let mut l = [b' '; 11];
    for (d, c) in l.iter_mut().zip(s.bytes()) {
        *d = c;
    }
    l
}

/// MBR s jednim oddilem typu `t` od sektoru 2048 do konce disku.
fn mbr(disk: &mut [u8], t: u8) {
    let sectors = (disk.len() / 512) as u32;
    let e = 446;
    disk[e + 4] = t;
    disk[e + 8..e + 12].copy_from_slice(&(PART_LBA as u32).to_le_bytes());
    disk[e + 12..e + 16].copy_from_slice(&(sectors - PART_LBA as u32).to_le_bytes());
    disk[510] = 0x55;
    disk[511] = 0xAA;
}

fn fat_format(part: &mut [u8], label: &str) -> Result<()> {
    let opts = fatfs::FormatVolumeOptions::new().volume_label(label11(label));
    fatfs::format_volume(Cursor::new(part), opts)?;
    Ok(())
}

/// Flash disk s MBR a oddilem FAT32 (0x0C), s pripravenym souborem.
pub fn fat(path: &Path) -> Result<PathBuf> {
    let mut d = vec![0u8; 64 * MIB];
    mbr(&mut d, 0x0C);
    let part = &mut d[PART_LBA * 512..];
    fat_format(part, FAT_LABEL)?;
    {
        let fs = fatfs::FileSystem::new(Cursor::new(&mut *part), fatfs::FsOptions::new())?;
        let mut f = fs.root_dir().create_file(FAT_FILE)?;
        f.write_all(FAT_TEXT.as_bytes())?;
        drop(f);
        fs.root_dir().create_dir("Složka")?.create_file("vnořený.txt")?.write_all(b"vnoreny\n")?;
        fs.unmount()?;
    }
    std::fs::write(path, &d)?;
    Ok(path.to_path_buf())
}

/// Flash disk s MBR a oddilem NTFS (0x07) z testovaciho obrazu crate ntfs.
pub fn ntfs(path: &Path) -> Result<PathBuf> {
    let img = std::fs::read(crate::workspace_root().join("third_party/test-images/ntfs/testfs1"))?;
    let mut d = vec![0u8; PART_LBA * 512 + img.len()];
    mbr(&mut d, 0x07);
    d[PART_LBA * 512..].copy_from_slice(&img);
    std::fs::write(path, &d)?;
    Ok(path.to_path_buf())
}

/// Novy svazek exFAT (formatovac `exfat-fs`).
fn exfat_format(size: usize, label: &str) -> Result<Vec<u8>> {
    use exfat_fs::format::{Exfat, FormatVolumeOptionsBuilder};
    let opts = FormatVolumeOptionsBuilder::default()
        .pack_bitmap(false)
        .full_format(false)
        .dev_size(size as u64)
        .label(exfat_fs::Label::new(label.to_string()).ok_or("jmenovka exFAT")?)
        .bytes_per_sector(512)
        .build()
        .map_err(|e| format!("exFAT: volby formatovani: {e}"))?;
    let mut f = Exfat::try_from::<std::time::SystemTime>(opts).map_err(|e| format!("exFAT: {e:?}"))?;
    let mut c = Cursor::new(vec![0u8; size]);
    f.write::<std::time::SystemTime, _>(&mut c).map_err(|e| format!("exFAT: zapis: {e:?}"))?;
    Ok(c.into_inner())
}

/// Flash disk s MBR a oddilem exFAT (0x07) s pripravenym souborem (zapis
/// vlastni implementaci `volfs`, overeni po testu nezavislym ctenarem).
pub fn exfat(path: &Path) -> Result<PathBuf> {
    use volfs::Volume;
    let size = 48 * MIB;
    let part = exfat_format(size - PART_LBA * 512, EXFAT_LABEL)?;
    let dev = volfs::dev::MemDisk { data: part, ss: 512, writes: 0 };
    let mut v = volfs::exfat::ExFatVolume::new(dev, false).map_err(|e| format!("exFAT: {e}"))?;
    let e = |e: volproto::Error| format!("exFAT: priprava: {e}");
    v.create("/Dokumenty", volproto::kind::DIR, true).map_err(e)?;
    let f = format!("/{EXFAT_FILE}");
    v.create(&f, volproto::kind::FILE, true).map_err(e)?;
    v.write(&f, 0, EXFAT_TEXT.as_bytes()).map_err(e)?;
    v.unmount().map_err(e)?;
    let part = v.device().with_dev(|d| std::mem::take(&mut d.data));
    let mut d = vec![0u8; size];
    mbr(&mut d, 0x07);
    d[PART_LBA * 512..].copy_from_slice(&part);
    std::fs::write(path, &d)?;
    Ok(path.to_path_buf())
}

/// Obraz pro nezavisleho ctenare `exfat-fs`.
struct Img(Vec<u8>);

impl exfat_fs::disk::ReadOffset for Img {
    type Err = std::io::Error;
    fn read_at(&self, off: u64, buf: &mut [u8]) -> std::result::Result<usize, Self::Err> {
        let s = (off as usize).min(self.0.len());
        let n = buf.len().min(self.0.len() - s);
        buf[..n].copy_from_slice(&self.0[s..s + n]);
        Ok(n)
    }
}

/// Soubor z obrazu exFAT podle nezavisleho ctenare.
fn exfat_read(part: Vec<u8>, path: &str) -> Result<Vec<u8>> {
    use exfat_fs::dir::Root;
    use exfat_fs::dir::entry::fs::FsElement;
    fn find(items: &mut [FsElement<Img>], parts: &[&str]) -> Option<Vec<u8>> {
        let e = items.iter_mut().find(|e| match e {
            FsElement::F(f) => f.name() == parts[0],
            FsElement::D(d) => d.name() == parts[0],
        })?;
        match e {
            FsElement::F(f) if parts.len() == 1 => {
                let mut out = Vec::new();
                f.read_to_end(&mut out).ok()?;
                Some(out)
            }
            FsElement::D(d) if parts.len() > 1 => find(&mut d.open().ok()?, &parts[1..]),
            _ => None,
        }
    }
    let mut root = Root::open(Img(part)).map_err(|_| "medium exFAT: nezavisly ctenar obraz neotevrel")?;
    let parts: Vec<&str> = path.split('/').filter(|p| !p.is_empty()).collect();
    find(root.items(), &parts).ok_or_else(|| format!("medium exFAT: soubor {path} neni v obrazu").into())
}

/// Po behu testu: soubor zapsany systemem na exFAT precte nezavisly ctenar
/// a svazek je oznaceny jako cisty (bezpecne odpojeni).
pub fn verify_exfat(path: &Path) -> Result<()> {
    let d = std::fs::read(path)?;
    let part = d[PART_LBA * 512..].to_vec();
    let flags = u16::from_le_bytes([part[106], part[107]]);
    if flags & 2 != 0 {
        return Err("medium exFAT: svazek po odpojeni neni oznaceny jako cisty".into());
    }
    let data = exfat_read(part.clone(), EXFAT_GUEST)?;
    if data.len() != EXFAT_GUEST_LEN || data.iter().enumerate().any(|(i, &b)| b != exfat_guest_byte(i)) {
        return Err(format!("medium exFAT: obsah {EXFAT_GUEST} nesedi ({} B)", data.len()).into());
    }
    if exfat_read(part, EXFAT_FILE)? != EXFAT_TEXT.as_bytes() {
        return Err("medium exFAT: puvodni soubor zmenen".into());
    }
    println!("xtask: medium exFAT: zapis systemu overen na hostiteli (nezavisly ctenar exfat-fs)");
    Ok(())
}

/// Flash disk bez tabulky oddilu (superfloppy) s FAT, pro pripojeni za behu.
pub fn hot(path: &Path) -> Result<PathBuf> {
    let mut d = vec![0u8; 16 * MIB];
    fat_format(&mut d, HOT_LABEL)?;
    {
        let fs = fatfs::FileSystem::new(Cursor::new(&mut d[..]), fatfs::FsOptions::new())?;
        fs.root_dir().create_file("hot.txt")?.write_all(b"za behu\n")?;
        fs.unmount()?;
    }
    std::fs::write(path, &d)?;
    Ok(path.to_path_buf())
}

/// Po behu testu: soubor zapsany systemem je v obrazu FAT (cte ho
/// nezavisle crate fatfs na hostiteli) a FS je cisty.
pub fn verify_fat(path: &Path) -> Result<()> {
    let mut d = std::fs::read(path)?;
    let part = &mut d[PART_LBA * 512..];
    let fs = fatfs::FileSystem::new(Cursor::new(part), fatfs::FsOptions::new())?;
    let mut text = String::new();
    fs.root_dir()
        .open_file(GUEST_FILE)
        .map_err(|e| format!("medium FAT: soubor {GUEST_FILE} zapsany systemem neni v obrazu: {e}"))?
        .read_to_string(&mut text)?;
    if text != GUEST_TEXT {
        return Err(format!("medium FAT: obsah {GUEST_FILE} nesedi: {text:?}").into());
    }
    let mut orig = String::new();
    fs.root_dir().open_file(FAT_FILE)?.read_to_string(&mut orig)?;
    if orig != FAT_TEXT {
        return Err(format!("medium FAT: puvodni soubor zmenen: {orig:?}").into());
    }
    println!("xtask: medium FAT: zapis systemu overen na hostiteli");
    Ok(())
}
