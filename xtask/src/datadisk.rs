//! Datovy disk se souborovym systemem: vytvori ho na hostiteli stejna
//! knihovna `fs`, jakou pouziva sluzba fsd v systemu (overuje to i
//! kompatibilitu formatu mezi hostitelem a systemem).
//!
//! Disk se pri kazdem behu vytvori znovu (testy do nej zapisuji).
//!
//! S `--wine` je na disku i Wine v rozlozeni aplikace
//! (`/apps/org.winehq.wine`, docs/cs/windows-vrstva.md): disk je pak vetsi
//! a vznika jednou jako sablona (`target/wine/<arch>/datadisk-<klic>.img`,
//! klic = obsah stromu Wine); beh dostane jeji kopii.

use std::path::{Path, PathBuf};

use fs::{Algo, FileDevice, FormatOptions, Fs, Kind};

/// Velikost datoveho disku.
const SIZE: u64 = 64 * 1024 * 1024;
/// Velikost datoveho disku s Wine (strom ~115 MB + misto pro prefix).
const SIZE_WINE: u64 = 512 * 1024 * 1024;
/// Adresar aplikace Wine na disku.
pub const WINE_APP: &str = "/apps/org.winehq.wine";
/// Testovaci programy Windows (`target/wine/<arch>/tests`, `user/wintest`).
pub const WINE_TESTS_APP: &str = "/apps/org.magnumopus.wintest";

/// Adresare na disk s Wine: (zdroj na hostiteli, cil na disku). Testy jen
/// kdyz je sestaveni Wine vyrobilo.
fn wine_sources(tree: &Path) -> Vec<(PathBuf, &'static str)> {
    let mut v = vec![(tree.to_path_buf(), WINE_APP)];
    let tests = tree.parent().unwrap_or(tree).join("tests");
    if tests.is_dir() {
        v.push((tests, WINE_TESTS_APP));
    }
    v
}

/// Vsechny soubory zdroju: (cesta na disku, plna cesta na hostiteli).
fn wine_files(tree: &Path) -> Result<Vec<(String, PathBuf)>> {
    let mut out = Vec::new();
    for (src, target) in wine_sources(tree) {
        let mut list = Vec::new();
        walk(&src, &src, &mut list)?;
        out.extend(list.into_iter().map(|(rel, full)| (format!("{target}/{rel}"), full)));
    }
    out.sort();
    Ok(out)
}

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

fn fs_err(what: &str, e: fs::Error) -> Box<dyn std::error::Error> {
    format!("datovy disk: {what}: {e:?}").into()
}

/// Vytvori datovy disk `path` s ukazkovym obsahem; vraci cestu. `wine` =
/// strom Wine (`target/wine/<arch>/tree`), ktery se zkopiruje do
/// [`WINE_APP`] (pres sablonu, viz modul).
pub fn create_with(path: &Path, wine: Option<&Path>) -> Result<PathBuf> {
    let Some(tree) = wine else { return build(path, SIZE, None) };
    let key = tree_key(tree)?;
    let template = tree.parent().unwrap_or(tree).join(format!("datadisk-{key:016x}.img"));
    if !template.is_file() {
        println!("xtask: datovy disk s Wine (sablona {})", template.display());
        let tmp = template.with_extension("tmp");
        build(&tmp, SIZE_WINE, Some(tree))?;
        std::fs::rename(&tmp, &template)?;
    }
    std::fs::copy(&template, path)?;
    Ok(path.to_path_buf())
}

/// Klic stromu: relativni cesty, velikosti a casy zmeny vsech souboru.
fn tree_key(tree: &Path) -> Result<u64> {
    use std::hash::{Hash, Hasher};
    let files = wine_files(tree)?;
    let mut h = std::collections::hash_map::DefaultHasher::new();
    SIZE_WINE.hash(&mut h);
    for (rel, full) in &files {
        let m = std::fs::metadata(full)?;
        rel.hash(&mut h);
        m.len().hash(&mut h);
        m.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_nanos()).hash(&mut h);
    }
    Ok(h.finish())
}

/// Soubory stromu: (relativni cesta s `/`, plna cesta).
fn walk(root: &Path, dir: &Path, out: &mut Vec<(String, PathBuf)>) -> Result<()> {
    for e in std::fs::read_dir(dir)? {
        let e = e?;
        let p = e.path();
        if e.file_type()?.is_dir() {
            walk(root, &p, out)?;
        } else {
            let rel = p.strip_prefix(root).map_err(|_| "cesta mimo strom")?.to_string_lossy().replace('\\', "/");
            out.push((rel, p));
        }
    }
    Ok(())
}

fn build(path: &Path, size: u64, wine: Option<&Path>) -> Result<PathBuf> {
    let path = path.to_path_buf();
    let file = std::fs::File::create(&path)?;
    file.set_len(size)?;
    drop(file);
    let dev = FileDevice::open(&path)?;
    let opts = FormatOptions {
        label: "data".into(),
        algo: Algo::Xxh3,
        ag_blocks: None,
        uuid: Some(random_uuid()),
        time: fs::Timestamp::UNIX_EPOCH,
    };
    let dev = Fs::format(dev, &opts).map_err(|e| fs_err("formatovani", e))?;
    let mut fs = Fs::mount(dev).map_err(|e| fs_err("pripojeni", e))?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| fs::Timestamp::new(d.as_secs() as i64, d.subsec_nanos() as i64))
        .unwrap_or(fs::Timestamp::UNIX_EPOCH);
    fs.set_time(now);
    let files: [(&str, &[u8]); 2] = [
        ("/vitejte.txt", b"Ahoj z hostitele!\n"),
        (
            "/dokumenty/navod.md",
            b"# Navod\n\nTento soubor vytvoril xtask na hostiteli.\n",
        ),
    ];
    fs.create("/dokumenty", Kind::Dir)
        .map_err(|e| fs_err("adresar", e))?;
    for (p, data) in files {
        fs.create(p, Kind::File).map_err(|e| fs_err(p, e))?;
        fs.write(p, 0, data).map_err(|e| fs_err(p, e))?;
    }
    if let Some(tree) = wine {
        let list = wine_files(tree)?;
        let mut dirs = std::collections::BTreeSet::new();
        for (target, full) in &list {
            // Adresare cesty (od korene) jednou.
            let mut at = 0;
            while let Some(i) = target[at + 1..].find('/') {
                let d = &target[..at + 1 + i];
                if dirs.insert(d.to_string()) {
                    match fs.create(d, Kind::Dir) {
                        Ok(()) | Err(fs::Error::AlreadyExists) => {}
                        Err(e) => return Err(fs_err(d, e)),
                    }
                }
                at += 1 + i;
            }
            let data = std::fs::read(full)?;
            fs.create(target, Kind::File).map_err(|e| fs_err(target, e))?;
            fs.write(target, 0, &data).map_err(|e| fs_err(target, e))?;
        }
        println!("xtask: Wine na datovem disku: {} souboru ({WINE_APP}, testy {WINE_TESTS_APP})", list.len());
    }
    fs.commit().map_err(|e| fs_err("commit", e))?;
    let report = fs.check(true).map_err(|e| fs_err("kontrola", e))?;
    if !report.problems.is_empty() {
        return Err(format!("datovy disk: kontrola konzistence: {:?}", report.problems).into());
    }
    Ok(path)
}

/// Nahodne UUID verze 4. Nahodu dava `RandomState` ze std (klice SipHash
/// seedovane z generatoru nahody operacniho systemu), takze neni potreba
/// dalsi zavislost.
fn random_uuid() -> [u8; 16] {
    use std::hash::{BuildHasher, Hasher};
    let mut u = [0u8; 16];
    for half in u.chunks_mut(8) {
        let mut h = std::collections::hash_map::RandomState::new().build_hasher();
        h.write_u64(0);
        half.copy_from_slice(&h.finish().to_le_bytes());
    }
    u[6] = (u[6] & 0x0F) | 0x40; // verze 4
    u[8] = (u[8] & 0x3F) | 0x80; // varianta RFC 4122
    u
}
