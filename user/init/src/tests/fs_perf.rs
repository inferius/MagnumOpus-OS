//! Vykon FS: radek shellu se zapisem (docs/cs/souborovy-system.md, "Vykon
//! radku shellu") a cena commitu na svazku po cele sade testu.
//!
//! Sada bezi az po ostatnich testech FS (svazek ma mnoho datasetu, klonu,
//! snapshotu a souboru) a meri pres skutecne fsd a disk:
//! - commit bez zmen a s malou zmenou,
//! - radek shellu: `ASIDE_BEGIN` (snapshot puvodu = pohled pro undo),
//!   podminene ulozeni v klonu, `ASIDE_CHANGES`, `ASIDE_APPLY` s `KEEP_VIEW`
//!   (`ASIDE_DISCARD` a `VIEW_RELEASE` az kdyz je shell volny, mimo radek),
//! - samostatny `VIEW` (programy mimo shell).
//!
//! Casove meze jsou s rezervou pro pomaly stroj (TCG, soubezne behy):
//! hlidaji, ze cena neroste se stavem FS (radove sekundy pred opravou).

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use fsproto::FsClient;
use libos::{Startup, println, time};

use super::{Test, TestResult, devices, fail};

pub const TESTS: &[Test] = &[("radek shellu se zapisem", shell_line)];

/// Kolik radku zmerit (bere se median).
const LINES: usize = 5;
/// Mez pro cely radek (median) a pro jeden commit. Namereno po cele sade
/// (s alokatorem po velikostnich tridach): x86_64 174 ms, aarch64 313 ms;
/// mez s rezervou hlida zpomaleni radove na sekundy.
const LINE_LIMIT_MS: u64 = 500;
const COMMIT_LIMIT_MS: u64 = 80;

const DIR: &str = "/f-vykon";
const ASIDE: &str = "/tmp/.aside";

fn ms(ns: u64) -> u64 {
    ns / 1_000_000
}

/// Zmeri jednu operaci (ns).
fn timed<T>(f: impl FnOnce() -> libos::Result<T>, what: &str) -> Result<(T, u64), String> {
    let t = time::now();
    let r = f().map_err(|e| fail(what, e))?;
    Ok((r, time::now() - t))
}

fn median(mut v: Vec<u64>) -> u64 {
    v.sort_unstable();
    v[v.len() / 2]
}

fn ensure_dir(fs: &FsClient, p: &str) -> TestResult {
    match fs.create(p, fsproto::kind::DIR) {
        Ok(()) | Err(libos::Error::AlreadyExists) => Ok(()),
        Err(e) => Err(fail(p, e)),
    }
}

fn shell_line(_: &Startup) -> TestResult {
    let fs = devices().and_then(|d| d.fs.as_ref()).ok_or("sluzba fsd nebezi")?;
    ensure_dir(fs, DIR)?;
    ensure_dir(fs, "/tmp")?;
    ensure_dir(fs, ASIDE)?;
    let file = format!("{DIR}/a.txt");
    match fs.create(&file, fsproto::kind::FILE) {
        Ok(()) | Err(libos::Error::AlreadyExists) => {}
        Err(e) => return Err(fail("soubor", e)),
    }
    fs.write(&file, 0, b"zacatek\n").map_err(|e| fail("zapis", e))?;
    fs.commit().map_err(|e| fail("commit", e))?;

    let (_, idle) = timed(|| fs.commit(), "commit bez zmen")?;
    // Stejna delka jako "zacatek\n" (zapis od zacatku bez zkraceni).
    fs.write(&file, 0, b"zmena..\n").map_err(|e| fail("zapis", e))?;
    let (_, small) = timed(|| fs.commit(), "commit male zmeny")?;
    println!("vykon FS: commit bez zmen {} ms, s malou zmenou {} ms", ms(idle), ms(small));

    let mut lines = Vec::new();
    let mut parts: [Vec<u64>; 6] = Default::default();
    for n in 0..LINES {
        let clone = format!("{ASIDE}/vykon-{n}");
        let start = time::now();
        let (ds, t0) = timed(|| fs.dataset_of(DIR), "DATASET_OF")?;
        let (b, t2) = timed(|| fs.aside_begin_ex(&ds, &clone), "ASIDE_BEGIN")?;
        // Zapis v klonu jako shell: revize, pak podminene ulozeni.
        let inner = format!("{clone}{}", &file[ds.trim_end_matches('/').len()..]);
        let (_, t3) = timed(
            || {
                let o = fs.open(&inner, fsproto::v2::access::READ, fsproto::v2::share::ALL, 0)?;
                let _ = fs.close(o.handle);
                fs.save_if(&inner, Some(o.revision), format!("radek {n}\n").as_bytes())
            },
            "REPLACE",
        )?;
        let (changes, t4) = timed(|| fs.aside_changes(&clone), "ASIDE_CHANGES")?;
        let (_, t5) = timed(|| fs.aside_apply_keep(&clone, None), "ASIDE_APPLY")?;
        lines.push(time::now() - start);
        // Pohled pro undo: stav pred radkem.
        let before = fs.snapshot_read_of(&ds, &b.snapshot, &file[ds.trim_end_matches('/').len()..], 0, 64).map_err(|e| fail("pohled", e))?;
        let expected = if n == 0 { String::from("zmena..\n") } else { format!("radek {}\n", n - 1) };
        if before != expected.as_bytes() {
            return Err(format!("pohled po radku {n}: {:?}", String::from_utf8_lossy(&before)));
        }
        // Samostatny pohled (programy mimo shell): bez zmen skoro zadarmo.
        let (v, t1) = timed(|| fs.view(&ds), "VIEW")?;
        fs.release_view(&ds, &v.snapshot).map_err(|e| fail("VIEW_RELEASE", e))?;
        for (i, t) in [t0, t1, t2, t3, t4, t5].into_iter().enumerate() {
            parts[i].push(t);
        }
        // Ulozeni pres docasny soubor: novy objekt na miste stareho.
        if changes.is_empty() || changes.iter().any(|c| c.path != file) {
            return Err(format!("zmeny klonu: {changes:?}"));
        }
        // Mimo radek (shell je volny).
        fs.aside_discard(&clone).map_err(|e| fail("ASIDE_DISCARD", e))?;
        fs.release_view(&ds, &b.snapshot).map_err(|e| fail("VIEW_RELEASE", e))?;
    }
    let names = ["DATASET_OF", "VIEW", "ASIDE_BEGIN", "REPLACE", "ASIDE_CHANGES", "ASIDE_APPLY"];
    let mut detail = String::new();
    for (i, n) in names.iter().enumerate() {
        detail.push_str(&format!(" {n} {}", ms(median(parts[i].clone()))));
    }
    let line = median(lines);
    println!("vykon FS: radek shellu {} ms (median z {LINES}):{detail}", ms(line));
    let data = fs.read(&file, 0, 64).map_err(|e| fail("cteni", e))?;
    if data != format!("radek {}\n", LINES - 1).as_bytes() {
        return Err(format!("obsah po radcich: {:?}", String::from_utf8_lossy(&data)));
    }
    if ms(small) > COMMIT_LIMIT_MS {
        return Err(format!("commit s malou zmenou {} ms (mez {COMMIT_LIMIT_MS} ms)", ms(small)));
    }
    if ms(line) > LINE_LIMIT_MS {
        return Err(format!("radek shellu {} ms (mez {LINE_LIMIT_MS} ms):{detail}", ms(line)));
    }
    Ok(())
}
