//! Testy exFAT na hostiteli: obraz formatuje nezavisly crate `exfat-fs`
//! (MIT) a po zapisu ho jim i cte (overeni, ze to, co zapiseme, precte
//! jina implementace). Poskozene obrazy: nic nesmi spadnout.

use exfat_fs::dir::Root;
use exfat_fs::dir::entry::fs::FsElement;
use exfat_fs::disk::ReadOffset;
use exfat_fs::format::{Exfat, FormatVolumeOptionsBuilder};
use std::io::{Cursor, Read};
use std::time::SystemTime;
use volproto::{Error, kind};

use super::ExFatVolume;
use crate::Volume;
use crate::dev::MemDisk;

const MIB: usize = 1024 * 1024;

/// Novy obraz exFAT dane velikosti se jmenovkou.
pub fn format(size: usize, label: &str) -> Vec<u8> {
    let opts = FormatVolumeOptionsBuilder::default()
        .pack_bitmap(false)
        .full_format(false)
        .dev_size(size as u64)
        .label(exfat_fs::Label::new(label.to_string()).unwrap())
        .bytes_per_sector(512)
        .build()
        .unwrap();
    let mut f = Exfat::try_from::<SystemTime>(opts).unwrap();
    let mut c = Cursor::new(vec![0u8; size]);
    f.write::<SystemTime, _>(&mut c).unwrap();
    c.into_inner()
}

fn mount(img: Vec<u8>, ro: bool) -> ExFatVolume<MemDisk> {
    ExFatVolume::new(MemDisk { data: img, ss: 512, writes: 0 }, ro).unwrap()
}

/// Obraz po operacich (po odpojeni).
fn image(mut v: ExFatVolume<MemDisk>) -> Vec<u8> {
    v.unmount().unwrap();
    v.device().with_dev(|d| d.data.clone())
}

/// Obraz pro nezavisleho ctenare.
#[derive(Debug)]
struct Img(Vec<u8>);

impl ReadOffset for Img {
    type Err = std::io::Error;
    fn read_at(&self, off: u64, buf: &mut [u8]) -> Result<usize, Self::Err> {
        let s = (off as usize).min(self.0.len());
        let n = buf.len().min(self.0.len() - s);
        buf[..n].copy_from_slice(&self.0[s..s + n]);
        Ok(n)
    }
}

fn name_of(e: &FsElement<Img>) -> &str {
    match e {
        FsElement::F(f) => f.name(),
        FsElement::D(d) => d.name(),
    }
}

/// Obsah souboru (`parts` od teto slozky) podle nezavisleho ctenare.
fn read_in(items: &mut [FsElement<Img>], parts: &[&str]) -> Option<Vec<u8>> {
    let e = items.iter_mut().find(|e| name_of(e) == parts[0])?;
    match e {
        FsElement::F(f) if parts.len() == 1 => {
            let mut out = Vec::new();
            f.read_to_end(&mut out).ok()?;
            Some(out)
        }
        FsElement::D(d) if parts.len() > 1 => read_in(&mut d.open().ok()?, &parts[1..]),
        _ => None,
    }
}

fn list_in(items: &mut [FsElement<Img>], parts: &[&str]) -> Option<Vec<String>> {
    let Some((first, rest)) = parts.split_first() else {
        return Some(items.iter().map(|e| name_of(e).to_string()).collect());
    };
    match items.iter_mut().find(|e| name_of(e) == *first)? {
        FsElement::D(d) => list_in(&mut d.open().ok()?, rest),
        FsElement::F(_) => None,
    }
}

fn split_path(p: &str) -> Vec<&str> {
    p.split('/').filter(|p| !p.is_empty()).collect()
}

/// Obsah souboru `path` podle nezavisleho ctenare (`exfat-fs`).
fn foreign_read(img: &[u8], path: &str) -> Option<Vec<u8>> {
    let mut root = Root::open(Img(img.to_vec())).ok()?;
    read_in(root.items(), &split_path(path))
}

/// Jmena ve slozce podle nezavisleho ctenare.
fn foreign_list(img: &[u8], dir: &str) -> Option<Vec<String>> {
    let mut root = Root::open(Img(img.to_vec())).ok()?;
    list_in(root.items(), &split_path(dir))
}

const LONG: &str = "Přílišžluťoučký kůň úpěl ďábelské ódy - dlouhé jméno.txt";

#[test]
fn mount_info_and_empty_root() {
    let mut v = mount(format(32 * MIB, "EXTEST"), false);
    let i = v.info().unwrap();
    assert_eq!((i.fs.as_str(), i.label.as_str(), i.read_only), ("exfat", "EXTEST", false));
    assert!(i.total_bytes > 28 * MIB as u64 && i.free_bytes > 0 && i.free_bytes <= i.total_bytes);
    assert!(v.list("/").unwrap().is_empty());
    assert_eq!(v.stat("/").unwrap().kind, kind::DIR);
}

#[test]
fn write_long_names_and_verify_on_host() {
    let mut v = mount(format(32 * MIB, "EXTEST"), false);
    v.create("/Složka", kind::DIR, true).unwrap();
    v.create("/Složka/Vnořená", kind::DIR, true).unwrap();
    let f = format!("/Složka/Vnořená/{LONG}");
    v.create(&f, kind::FILE, true).unwrap();
    assert_eq!(v.write(&f, 0, b"ahoj z exFAT\n").unwrap(), 13);
    assert_eq!(v.read(&f, 0, 100).unwrap(), b"ahoj z exFAT\n");
    let st = v.stat(&f).unwrap();
    assert_eq!((st.kind, st.size), (kind::FILE, 13));
    assert!(st.mtime > 1_700_000_000, "cas zmeny {}", st.mtime);
    // Velikost pismen: tabulka z media (i pro diakritiku).
    assert_eq!(v.read(&format!("/SLOŽKA/vnořená/{}", LONG.to_uppercase()), 0, 4).unwrap(), b"ahoj");
    assert_eq!(v.create("/složka", kind::DIR, true), Err(Error::AlreadyExists));
    let img = image(v);
    assert_eq!(foreign_read(&img, &f).as_deref(), Some(&b"ahoj z exFAT\n"[..]));
    assert_eq!(foreign_list(&img, "/Složka").unwrap(), vec!["Vnořená".to_string()]);
    // Po odpojeni znovu pripojit: cisty svazek, stejny obsah.
    let mut v = mount(img, false);
    assert_eq!(v.read(&f, 0, 100).unwrap(), b"ahoj z exFAT\n");
}

#[test]
fn big_fragmented_files_and_holes() {
    let mut v = mount(format(32 * MIB, "EXTEST"), false);
    v.create("/a.bin", kind::FILE, true).unwrap();
    v.create("/b.bin", kind::FILE, true).unwrap();
    // Stridave zapisy: clustery obou souboru se prolinaji (retez FAT
    // po prevodu ze souvisleho souboru).
    let pa: Vec<u8> = (0..300_000u32).map(|i| (i * 7 % 251) as u8).collect();
    let pb: Vec<u8> = (0..300_000u32).map(|i| (i * 13 % 241) as u8).collect();
    for k in (0..pa.len()).step_by(20_000) {
        let e = (k + 20_000).min(pa.len());
        v.write("/a.bin", k as u64, &pa[k..e]).unwrap();
        v.write("/b.bin", k as u64, &pb[k..e]).unwrap();
    }
    assert_eq!(v.read("/a.bin", 0, pa.len() + 10).unwrap(), pa);
    assert_eq!(v.read("/b.bin", 123_456, 1000).unwrap(), &pb[123_456..124_456]);
    // Dira: zapis za koncem, mezi nuly.
    v.create("/dira", kind::FILE, true).unwrap();
    v.write("/dira", 100_000, b"konec").unwrap();
    let d = v.read("/dira", 0, 200_000).unwrap();
    assert_eq!(d.len(), 100_005);
    assert!(d[..100_000].iter().all(|&b| b == 0) && &d[100_000..] == b"konec");
    // Zvetseni bez zapisu (ValidDataLength): nuly; zmenseni uvolni misto.
    let free = v.info().unwrap().free_bytes;
    v.set_len("/a.bin", 1_000_000).unwrap();
    assert!(v.read("/a.bin", 300_000, 1000).unwrap().iter().all(|&b| b == 0));
    v.set_len("/a.bin", 1000).unwrap();
    assert!(v.info().unwrap().free_bytes > free);
    assert_eq!(v.read("/a.bin", 0, 5000).unwrap(), &pa[..1000]);
    v.set_len("/b.bin", 0).unwrap();
    assert_eq!(v.stat("/b.bin").unwrap().size, 0);
    v.write("/b.bin", 0, b"znovu").unwrap();
    let img = image(v);
    assert_eq!(foreign_read(&img, "/a.bin").unwrap(), &pa[..1000]);
    assert_eq!(foreign_read(&img, "/b.bin").unwrap(), b"znovu");
    assert_eq!(foreign_read(&img, "/dira").unwrap(), d);
}

#[test]
fn many_files_grow_directory() {
    let mut v = mount(format(32 * MIB, "EXTEST"), false);
    v.create("/mnoho", kind::DIR, true).unwrap();
    for i in 0..300 {
        let p = format!("/mnoho/soubor s delsim jmenem {i:03}.txt");
        v.create(&p, kind::FILE, true).unwrap();
        v.write(&p, 0, format!("{i}").as_bytes()).unwrap();
    }
    // I v koreni (retez FAT bez sady polozek).
    // Soubory s daty: `exfat-fs` neprecte prazdny soubor s prvnim clusterem 0
    // (specifikace ho dovoluje, Windows ho tak vytvari; chyba ctenare).
    for i in 0..200 {
        let p = format!("/koren-{i:03} s dlouhym jmenem souboru");
        v.create(&p, kind::FILE, true).unwrap();
        v.write(&p, 0, b"k").unwrap();
    }
    assert_eq!(v.list("/").unwrap().len(), 201);
    assert_eq!(v.list("/mnoho").unwrap().len(), 300);
    assert_eq!(v.read("/mnoho/soubor s delsim jmenem 299.txt", 0, 10).unwrap(), b"299");
    let img = image(v);
    assert_eq!(foreign_list(&img, "/mnoho").unwrap().len(), 300);
    assert_eq!(foreign_read(&img, "/mnoho/soubor s delsim jmenem 123.txt").unwrap(), b"123");
    assert_eq!(foreign_list(&img, "/").unwrap().len(), 201);
}

#[test]
fn rename_remove_and_errors() {
    let mut v = mount(format(32 * MIB, "EXTEST"), false);
    v.create("/d", kind::DIR, true).unwrap();
    v.create("/d/x.txt", kind::FILE, true).unwrap();
    v.write("/d/x.txt", 0, b"x").unwrap();
    v.create("/y.txt", kind::FILE, true).unwrap();
    // Na miste (kratsi jmeno), delsi jmeno (nove sloty), do jine slozky.
    v.rename("/d/x.txt", "/d/z.txt", false).unwrap();
    v.rename("/d/z.txt", &format!("/d/{LONG}"), false).unwrap();
    v.rename(&format!("/d/{LONG}"), "/presunuto.txt", false).unwrap();
    assert_eq!(v.read("/presunuto.txt", 0, 10).unwrap(), b"x");
    assert!(v.list("/d").unwrap().is_empty());
    // Velikost pismen na miste.
    v.rename("/presunuto.txt", "/PRESUNUTO.txt", false).unwrap();
    assert_eq!(v.list("/").unwrap().iter().filter(|(n, _)| n == "PRESUNUTO.txt").count(), 1);
    assert_eq!(v.rename("/PRESUNUTO.txt", "/y.txt", false), Err(Error::AlreadyExists));
    v.rename("/PRESUNUTO.txt", "/y.txt", true).unwrap();
    assert_eq!(v.read("/y.txt", 0, 10).unwrap(), b"x");
    assert_eq!(v.rename("/d", "/d/pod", false), Err(Error::InvalidArgs));
    v.create("/d/a", kind::FILE, true).unwrap();
    assert_eq!(v.remove("/d"), Err(Error::NotEmpty));
    v.remove("/d/a").unwrap();
    v.remove("/d").unwrap();
    assert_eq!(v.stat("/d"), Err(Error::NotFound));
    assert_eq!(v.create("/a:b", kind::FILE, true), Err(Error::InvalidArgs));
    assert_eq!(v.create("/tecka.", kind::FILE, true), Err(Error::InvalidArgs));
    assert_eq!(v.create(&format!("/{}", "x".repeat(256)), kind::FILE, true), Err(Error::InvalidArgs));
    assert_eq!(v.read("/neni", 0, 1), Err(Error::NotFound));
    assert_eq!(v.read("/y.txt/x", 0, 1), Err(Error::NotADirectory));
    let img = image(v);
    assert_eq!(foreign_read(&img, "/y.txt").unwrap(), b"x");
    assert_eq!(foreign_list(&img, "/").unwrap(), vec!["y.txt".to_string()]);
}

#[test]
fn no_space() {
    let mut v = mount(format(4 * MIB, "MALY"), false);
    let free = v.info().unwrap().free_bytes;
    v.create("/velky", kind::FILE, true).unwrap();
    let chunk = vec![0xAAu8; 256 * 1024];
    let mut off = 0u64;
    let err = loop {
        match v.write("/velky", off, &chunk) {
            Ok(_) => off += chunk.len() as u64,
            Err(e) => break e,
        }
    };
    assert_eq!(err, Error::NoSpace);
    // Neuspesny zapis nic nezmenil; smazani misto vrati.
    assert_eq!(v.stat("/velky").unwrap().size, off);
    v.remove("/velky").unwrap();
    assert_eq!(v.info().unwrap().free_bytes, free, "misto po smazani zpet");
}

#[test]
fn dirty_flag_and_read_only() {
    let img = format(16 * MIB, "EXTEST");
    let mut v = mount(img.clone(), false);
    v.create("/f", kind::FILE, true).unwrap();
    let flags = v.device().with_dev(|d| u16::from_le_bytes([d.data[106], d.data[107]]));
    assert_eq!(flags & 2, 2, "spinavy svazek behem zapisu");
    let img2 = image(v);
    assert_eq!(u16::from_le_bytes([img2[106], img2[107]]) & 2, 0, "cisty po odpojeni");
    // Jen pro cteni: zadny zapis, ani priznak.
    let mut v = mount(img2.clone(), true);
    assert_eq!(v.create("/g", kind::FILE, true), Err(Error::ReadOnly));
    assert_eq!(v.write("/f", 0, b"x"), Err(Error::ReadOnly));
    assert!(v.info().unwrap().read_only);
    v.unmount().unwrap();
    assert_eq!(v.device().with_dev(|d| d.writes), 0);
}

#[test]
fn damaged_boot_uses_backup_read_only() {
    let mut img = format(16 * MIB, "EXTEST");
    {
        let mut v = mount(img.clone(), false);
        v.create("/f", kind::FILE, true).unwrap();
        v.write("/f", 0, b"data").unwrap();
        img = image(v);
    }
    // Hlavni boot sektor poskozen (kontrolni soucet nesedi).
    let mut bad = img.clone();
    bad[100] ^= 0xFF;
    let mut v = mount(bad, false);
    assert!(v.info().unwrap().read_only);
    assert_eq!(v.read("/f", 0, 10).unwrap(), b"data");
    // Obe oblasti poskozene: nepripoji se.
    let mut bad = img.clone();
    bad[100] ^= 0xFF;
    bad[12 * 512 + 100] ^= 0xFF;
    assert!(ExFatVolume::new(MemDisk { data: bad, ss: 512, writes: 0 }, false).is_err());
    // Poskozena sada polozek (kontrolni soucet): soubor zmizi ze seznamu.
    let v = mount(img.clone(), true);
    let root = v.g.root;
    let pos = v.g.cluster_pos(root) as usize;
    drop(v);
    let mut bad = img.clone();
    let slot = (0..64).find(|i| bad[pos + i * 32] == 0x85).unwrap();
    bad[pos + slot * 32 + 4] ^= 0x01;
    let mut v = mount(bad, false);
    assert!(v.list("/").unwrap().is_empty());
    assert_eq!(v.read("/f", 0, 1), Err(Error::NotFound));
}

/// Jednoduchy generator (deterministicky, bez zavislosti).
struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        self.0 >> 33
    }
}

#[test]
fn fuzz_damaged_images_never_panic() {
    let mut base = format(8 * MIB, "FUZZ");
    {
        let mut v = mount(base.clone(), false);
        v.create("/d", kind::DIR, true).unwrap();
        for i in 0..20 {
            let p = format!("/d/soubor cislo {i}");
            v.create(&p, kind::FILE, true).unwrap();
            v.write(&p, 0, &vec![i as u8; 5000 * i]).unwrap();
        }
        base = image(v);
    }
    // Metadata lezi v prvnich ~1,5 MiB (boot, FAT, bitmapa, koren, slozka).
    let meta = 2 * MIB;
    let mut rng = Lcg(0x5eed);
    let mut mounted = 0;
    for round in 0..300 {
        let mut img = base.clone();
        for _ in 0..1 + rng.next() % 40 {
            let p = (rng.next() as usize) % meta;
            img[p] = rng.next() as u8;
        }
        // Obcas cely sektor nahodne.
        if round % 7 == 0 {
            let s = (rng.next() as usize % (meta / 512)) * 512;
            for b in &mut img[s..s + 512] {
                *b = rng.next() as u8;
            }
        }
        let Ok(mut v) = ExFatVolume::new(MemDisk { data: img, ss: 512, writes: 0 }, false) else { continue };
        mounted += 1;
        let _ = v.info();
        if let Ok(l) = v.list("/d") {
            for (n, _) in l.iter().take(5) {
                let p = format!("/d/{n}");
                let _ = v.read(&p, 0, 100_000);
                let _ = v.write(&p, 3000, b"zapis do poskozeneho");
                let _ = v.set_len(&p, 10);
                let _ = v.rename(&p, "/presun", true);
            }
        }
        let _ = v.create("/novy", kind::DIR, true);
        let _ = v.create("/novy/soubor", kind::FILE, true);
        let _ = v.write("/novy/soubor", 0, &[1u8; 70_000]);
        let _ = v.remove("/d/soubor cislo 3");
        let _ = v.list("/");
        let _ = v.unmount();
    }
    // Poskozeni casto zasahne jen data nebo slozky: vetsina obrazu se pripoji
    // a operace bezi nad poskozenymi strukturami.
    assert!(mounted > 100, "pripojeno jen {mounted} z 300");
}
