//! Rychly rozdil a slouceni (preskakovani sdilenych podstromu, bodove
//! dotazy): shoda s puvodnim uplnym pruchodem na nahodnych zmenach a cena
//! umerna zmene, ne velikosti stromu.

use alloc::string::String;
use alloc::vec::Vec;

use super::{assert_clean, new_fs};
use crate::fs::set_reference_mode;
use crate::{Fs, Kind, MemDevice};

/// Jednoduchy deterministicky generator (xorshift64*).
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
}

/// Slozky a soubory/odkazy pod `root` (bez vstupu do datasetu).
fn walk(fs: &mut Fs<MemDevice>, root: &str) -> (Vec<String>, Vec<String>) {
    let mut dirs = alloc::vec![String::from(root)];
    let mut files = Vec::new();
    let mut i = 0;
    while i < dirs.len() {
        let d = dirs[i].clone();
        for e in fs.list(&d).unwrap() {
            let p = if d == "/" { alloc::format!("/{}", e.name) } else { alloc::format!("{d}/{}", e.name) };
            match e.kind {
                Kind::Dir => dirs.push(p),
                Kind::File | Kind::Symlink => files.push(p),
                Kind::Dataset => {}
            }
        }
        i += 1;
    }
    (dirs, files)
}

/// Obsah: kratky (inline) nebo pres vic bloku.
fn content(rng: &mut Rng) -> Vec<u8> {
    let len = match rng.below(4) {
        0 => rng.below(40),
        1 => 100 + rng.below(3000),
        _ => 4096 + rng.below(9000),
    };
    let b = rng.next() as u8;
    (0..len).map(|i| b.wrapping_add(i as u8)).collect()
}

/// `n` nahodnych operaci pod `root` (chyby jako "uz existuje" nevadi).
fn random_ops(fs: &mut Fs<MemDevice>, root: &str, rng: &mut Rng, n: usize) {
    for _ in 0..n {
        let (dirs, files) = walk(fs, root);
        let dir = dirs[rng.below(dirs.len())].clone();
        let name = alloc::format!("n{}", rng.below(60));
        let new = if dir == "/" { alloc::format!("/{name}") } else { alloc::format!("{dir}/{name}") };
        match rng.below(10) {
            0 | 1 => {
                if fs.create(&new, Kind::File).is_ok() {
                    let _ = fs.write(&new, 0, &content(rng));
                }
            }
            2 => {
                let _ = fs.create(&new, Kind::Dir);
            }
            3 | 4 if !files.is_empty() => {
                let f = &files[rng.below(files.len())];
                let off = rng.below(6000) as u64;
                let _ = fs.write(f, off, &content(rng));
            }
            5 if !files.is_empty() => {
                let f = &files[rng.below(files.len())];
                let _ = fs.remove(f);
            }
            6 => {
                // Presun souboru nebo slozky (ne do vlastniho podstromu).
                let all: Vec<&String> = files.iter().chain(dirs.iter().skip(1)).collect();
                if !all.is_empty() {
                    let src = all[rng.below(all.len())].clone();
                    if !new.starts_with(&alloc::format!("{src}/")) && new != src {
                        let _ = fs.rename(&src, &new);
                    }
                }
            }
            7 => {
                let all: Vec<&String> = files.iter().chain(dirs.iter().skip(1)).collect();
                if !all.is_empty() {
                    let p = all[rng.below(all.len())].clone();
                    let _ = fs.trash(&p);
                }
            }
            8 => {
                let _ = fs.symlink(&new, &alloc::format!("cil-{}", rng.below(1000)));
            }
            _ if !files.is_empty() => {
                let f = &files[rng.below(files.len())];
                let _ = fs.truncate(f, rng.below(5000) as u64);
            }
            _ => {}
        }
    }
}

/// Zaklad: dost souboru, aby mel strom vic urovni.
fn populate(fs: &mut Fs<MemDevice>, root: &str, rng: &mut Rng, dirs: usize, per_dir: usize) {
    for d in 0..dirs {
        let dp = if root == "/" { alloc::format!("/d{d}") } else { alloc::format!("{root}/d{d}") };
        fs.create(&dp, Kind::Dir).unwrap();
        for f in 0..per_dir {
            let fp = alloc::format!("{dp}/f{f}");
            fs.create(&fp, Kind::File).unwrap();
            if rng.below(3) == 0 {
                fs.write(&fp, 0, &content(rng)).unwrap();
            }
        }
    }
}

fn both<T: PartialEq + core::fmt::Debug>(f: &mut dyn FnMut() -> T) -> T {
    set_reference_mode(true);
    let r = f();
    set_reference_mode(false);
    let fast = f();
    assert_eq!(fast, r, "rychla cesta se lisi od reference");
    fast
}

#[test]
fn diff_matches_full_walk() {
    for seed in 1..=super::cases(12) as u64 {
        let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15));
        let mut fs = new_fs(32768);
        fs.create_dataset("/p").unwrap();
        populate(&mut fs, "/p", &mut rng, 6, 40);
        random_ops(&mut fs, "/p", &mut rng, 40);
        fs.snapshot_of("/p", "a").unwrap();
        let n = 30 + rng.below(60);
        random_ops(&mut fs, "/p", &mut rng, n);
        fs.snapshot_of("/p", "b").unwrap();
        random_ops(&mut fs, "/p", &mut rng, 20);
        let d1 = both(&mut || fs.diff("/p", "a", Some("b")).unwrap());
        assert!(!d1.is_empty());
        both(&mut || fs.diff("/p", "a", None).unwrap());
        both(&mut || fs.diff("/p", "b", None).unwrap());
        assert!(both(&mut || fs.diff("/p", "b", Some("b")).unwrap()).is_empty());
        // Klon: zmeny v nem proti vychozimu snapshotu.
        fs.clone_snapshot("/p", "a", "/c").unwrap();
        random_ops(&mut fs, "/c", &mut rng, 40);
        both(&mut || fs.diff_clone("/c").unwrap());
        fs.commit().unwrap();
        both(&mut || fs.diff_clone("/c").unwrap());
    }
}

/// Otisk stromu `root`: cesty, druhy a obsah (bez vstupu do datasetu).
fn dump(fs: &mut Fs<MemDevice>, root: &str) -> Vec<(String, Vec<u8>)> {
    let (dirs, files) = walk(fs, root);
    let mut out: Vec<(String, Vec<u8>)> = dirs.into_iter().map(|d| (d, Vec::new())).collect();
    for f in files {
        let data = match fs.readlink(&f) {
            Ok(t) => t.into_bytes(),
            Err(_) => fs.read(&f, 0, 1 << 20).unwrap(),
        };
        out.push((f, data));
    }
    out.sort();
    out
}

/// Scenar "bokem" korene: zmeny v klonu i v originalu, prevzeti.
fn aside_scenario(seed: u64, reference: bool, select: bool) -> (crate::MergeReport, Vec<(String, Vec<u8>)>) {
    let mut rng = Rng(seed.wrapping_mul(0xD1B5_4A32_D192_ED03));
    let mut fs = new_fs(32768);
    populate(&mut fs, "/", &mut rng, 5, 30);
    random_ops(&mut fs, "/", &mut rng, 30);
    fs.aside_begin("/", "/w").unwrap();
    let n = 30 + rng.below(40);
    random_ops(&mut fs, "/w", &mut rng, n);
    // Soubezne zmeny originalu (mimo klon): konflikty.
    for _ in 0..(rng.below(12)) {
        let (_, files) = walk(&mut fs, "/");
        let files: Vec<String> = files.into_iter().filter(|f| !f.starts_with("/w/")).collect();
        if files.is_empty() {
            break;
        }
        let f = files[rng.below(files.len())].clone();
        match rng.below(3) {
            0 => {
                let _ = fs.write(&f, 0, &content(&mut rng));
            }
            1 => {
                let _ = fs.remove(&f);
            }
            _ => {
                let _ = fs.create(&alloc::format!("/d{}/n{}", rng.below(5), rng.below(60)), Kind::File);
            }
        }
    }
    let sel: Vec<String> = alloc::vec![String::from("/d1"), String::from("/d3")];
    set_reference_mode(reference);
    let r = fs.aside_apply("/w", if select { Some(&sel) } else { None });
    set_reference_mode(false);
    let r = r.unwrap();
    assert_clean(&mut fs);
    let mut d = dump(&mut fs, "/");
    d.retain(|(p, _)| !p.starts_with("/w"));
    (r, d)
}

#[test]
fn aside_apply_matches_full_index() {
    for seed in 1..=super::cases(10) as u64 {
        for select in [false, true] {
            let r = aside_scenario(seed, true, select);
            let f = aside_scenario(seed, false, select);
            assert_eq!(f.0, r.0, "seed {seed}: report");
            assert_eq!(f.1, r.1, "seed {seed}: vysledny strom");
        }
    }
}

/// Merge vetve: zmeny v obou vetvich, slouceni (konflikty se resi kopiemi).
fn merge_scenario(seed: u64, reference: bool) -> (crate::MergeReport, Vec<(String, Vec<u8>)>) {
    let mut rng = Rng(seed.wrapping_mul(0xA076_1D64_78BD_642F));
    let mut fs = new_fs(32768);
    fs.create_dataset("/r").unwrap();
    populate(&mut fs, "/r", &mut rng, 4, 30);
    fs.commit_version("/r", "zaklad").unwrap();
    fs.branch("/r", "x", "/x").unwrap();
    random_ops(&mut fs, "/x", &mut rng, 30);
    fs.commit_version("/x", "vetev").unwrap();
    random_ops(&mut fs, "/r", &mut rng, 15);
    fs.commit_version("/r", "nase").unwrap();
    set_reference_mode(reference);
    let r = fs.merge("/r", "/x");
    set_reference_mode(false);
    let mut r = r.unwrap();
    // Commit slouceni nese cas a ID - porovnava se jen, zda vznikl.
    let had_commit = r.commit.take().is_some();
    assert_eq!(had_commit, r.conflicts.is_empty());
    assert_clean(&mut fs);
    (r, dump(&mut fs, "/r"))
}

#[test]
fn branch_merge_matches_full_index() {
    for seed in 1..=super::cases(8) as u64 {
        let r = merge_scenario(seed, true);
        let f = merge_scenario(seed, false);
        assert_eq!(f.0, r.0, "seed {seed}: report");
        assert_eq!(f.1, r.1, "seed {seed}: vysledny strom");
    }
}

/// Strom s `dirs` x 100 soubory, klon "bokem" korene, zmena 2 souboru;
/// po odpojeni (prazdna cache) zmeri (navstivene uzly, cteni z disku) pro
/// `diff_clone` a `aside_apply`.
fn aside_cost(dirs: usize, reference: bool) -> [(u64, u64); 2] {
    let mut rng = Rng(7);
    let mut fs = new_fs(65536);
    populate(&mut fs, "/", &mut rng, dirs, 100);
    fs.commit().unwrap();
    fs.aside_begin("/", "/w").unwrap();
    fs.truncate("/w/d7/f42", 0).unwrap();
    fs.write("/w/d7/f42", 0, b"zmeneno").unwrap();
    fs.create("/w/d3/novy", Kind::File).unwrap();
    fs.write("/w/d3/novy", 0, b"novy").unwrap();
    fs.commit().unwrap();
    let mut fs = super::remount(fs);
    set_reference_mode(reference);
    let c0 = fs.node_counters();
    let d = fs.diff_clone("/w").unwrap();
    let cd = fs.node_counters();
    let diff_cost = (cd.0 - c0.0, cd.1 - c0.1);
    set_reference_mode(false);
    fs.commit().unwrap();
    let mut fs = super::remount(fs);
    set_reference_mode(reference);
    let c1 = fs.node_counters();
    let r = fs.aside_apply("/w", None).unwrap();
    let c2 = fs.node_counters();
    set_reference_mode(false);
    assert_eq!(d.len(), 2, "{d:?}");
    assert!(r.conflicts.is_empty() && r.applied.len() == 2, "{r:?}");
    assert_eq!(fs.read("/d7/f42", 0, 100).unwrap(), b"zmeneno");
    assert_eq!(fs.read("/d3/novy", 0, 100).unwrap(), b"novy");
    [diff_cost, (c2.0 - c1.0, c2.1 - c1.1)]
}

/// 5000 souboru, zmena 2: rozdil a prevzeti sahnou jen na malo uzlu
/// a cena skoro neroste s poctem souboru (uplny pruchod: vsechny listy).
#[test]
fn aside_cost_scales_with_change() {
    let full = aside_cost(50, true);
    let small = aside_cost(10, false);
    let fast = aside_cost(50, false);
    std::eprintln!("(navstivene uzly, cteni z disku) diff_clone / aside_apply:");
    std::eprintln!("  uplny pruchod, 5000 souboru: {:?} / {:?}", full[0], full[1]);
    std::eprintln!("  rychle,        1000 souboru: {:?} / {:?}", small[0], small[1]);
    std::eprintln!("  rychle,        5000 souboru: {:?} / {:?}", fast[0], fast[1]);
    // Horni mez umerna zmene (cesty k nekolika listum, vypisy dvou slozek
    // po 100 polozkach a korene, zapis a commit), ne poctu souboru. Ukazatel
    // s kopii uzlu (FS faze 3, 48 B) dava mensi vetveni uzlu, a proto o neco
    // vyssi strom nez puvodni 40 B ukazatel - meze maji rezervu na uroven.
    assert!(fast[0].0 <= 90 && fast[0].1 <= 40, "diff_clone: {:?}", fast[0]);
    assert!(fast[1].1 <= 80, "aside_apply: {:?}", fast[1]);
    assert!(fast[0].0 <= small[0].0 + 30 && fast[1].0 <= small[1].0 + 80);
    // Rychla cesta je vzdy vyrazne levnejsi nez uplny pruchod.
    assert!(fast[0].0 * 3 <= full[0].0 && fast[1].0 * 2 <= full[1].0);
    assert!(full[0].1 > 5 * fast[0].1 && full[1].1 > 3 * fast[1].1);
    assert!(full[0].0 > 4 * fast[0].0 && full[1].0 > 2 * fast[1].0);
}
