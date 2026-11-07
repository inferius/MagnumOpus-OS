//! Mereni vykonu a rezie (`cargo test -p fs --release -- --ignored --nocapture perf`).

use std::time::Instant;

use super::new_fs;
use crate::{BLOCK_SIZE, Kind};

#[test]
#[ignore]
fn perf_sequential_and_overhead() {
    let mib = 512u64;
    let mut fs = new_fs(mib * 256 + 40_000);
    let free0 = fs.info().free_blocks;
    fs.create("/velky", Kind::File).unwrap();
    let chunk = vec![0xA5u8; 1 << 20];
    let t = Instant::now();
    for i in 0..mib {
        fs.write("/velky", i << 20, &chunk).unwrap();
    }
    fs.commit().unwrap();
    let w = t.elapsed();
    let t = Instant::now();
    for i in 0..mib {
        assert_eq!(fs.read("/velky", i << 20, 1 << 20).unwrap().len(), 1 << 20);
    }
    let r = t.elapsed();
    let used = free0 - fs.info().free_blocks;
    let data = mib * 256;
    std::eprintln!(
        "zapis {:.0} MiB/s, cteni {:.0} MiB/s, rezie metadat {:.3} % ({} bloku na {} bloku dat), fragmentu {}",
        mib as f64 / w.as_secs_f64(),
        mib as f64 / r.as_secs_f64(),
        (used - data) as f64 * 100.0 / data as f64,
        used - data,
        data,
        fs.fragments("/velky").unwrap()
    );
    // Male soubory: vytvoreni + zapis 4 KiB.
    let t = Instant::now();
    fs.create("/d", Kind::Dir).unwrap();
    for i in 0..20_000 {
        let p = format!("/d/soubor-{i}.txt");
        fs.create(&p, Kind::File).unwrap();
        fs.write(&p, 0, &chunk[..BLOCK_SIZE]).unwrap();
    }
    fs.commit().unwrap();
    std::eprintln!(
        "20 000 malych souboru: {:.0} souboru/s",
        20_000.0 / t.elapsed().as_secs_f64()
    );
    let t = Instant::now();
    let st = fs.stat("/d").unwrap();
    std::eprintln!(
        "velikost adresare {} B, {} souboru za {:?}",
        st.tree_bytes,
        st.tree_files,
        t.elapsed()
    );
}

/// Radek shellu se zapisem nad FS s mnoha datasety a snapshoty (jako po
/// testech systemu): pohled pro undo, klon bokem, zapis, zmeny, prevzeti,
/// zahozeni. Vypise cas kroku a pocet commitu.
#[test]
#[ignore]
fn perf_shell_line_on_busy_fs() {
    let mut fs = new_fs(200_000);
    let chunk = vec![0x5Au8; 8192];
    for d in 0..60 {
        let ds = format!("/ds{d}");
        fs.create_dataset(&ds).unwrap();
        for f in 0..40 {
            let p = format!("{ds}/f{f}");
            fs.create(&p, Kind::File).unwrap();
            fs.write(&p, 0, &chunk).unwrap();
        }
        fs.commit().unwrap();
        for s in 0..3 {
            fs.write(&format!("{ds}/f0"), 0, &chunk[..100 + s]).unwrap();
            fs.commit_version(&ds, &format!("v{s}")).unwrap();
        }
    }
    fs.create("/home", Kind::Dir).unwrap();
    for f in 0..200 {
        let p = format!("/home/f{f}");
        fs.create(&p, Kind::File).unwrap();
        fs.write(&p, 0, &chunk[..1000]).unwrap();
    }
    fs.commit().unwrap();
    for round in 0..3 {
        let t = Instant::now();
        let d0 = fs.device();
        let mut last = (d0.written, d0.write_calls, d0.flushes);
        let mut lap = |what: &str, t: &mut Instant, fs: &crate::Fs<crate::MemDevice>| {
            let d = fs.device();
            let now = (d.written, d.write_calls, d.flushes);
            std::eprintln!(
                "  {what}: {:?}, bloku {}, zapisu {}, flush {}",
                t.elapsed(),
                now.0 - last.0,
                now.1 - last.1,
                now.2 - last.2
            );
            last = now;
            *t = Instant::now();
        };
        let mut tt = Instant::now();
        let v = fs.view("/").unwrap();
        lap("view", &mut tt, &fs);
        fs.create("/tmp", Kind::Dir).ok();
        fs.aside_begin("/", &format!("/tmp/a{round}")).unwrap();
        lap("aside_begin", &mut tt, &fs);
        fs.write(&format!("/tmp/a{round}/home/f1"), 0, format!("zmena {round}").as_bytes()).unwrap();
        lap("zapis", &mut tt, &fs);
        let d = fs.diff_clone(&format!("/tmp/a{round}")).unwrap();
        assert_eq!(d.len(), 1);
        lap("diff_clone", &mut tt, &fs);
        fs.aside_apply(&format!("/tmp/a{round}"), None).unwrap();
        lap("aside_apply", &mut tt, &fs);
        fs.aside_discard(&format!("/tmp/a{round}")).unwrap();
        lap("aside_discard", &mut tt, &fs);
        fs.release_view("/", &v.snapshot).unwrap();
        lap("release_view", &mut tt, &fs);
        fs.commit().unwrap();
        lap("commit (prazdny)", &mut tt, &fs);
        std::eprintln!("radek {round}: {:?}", t.elapsed());
    }
}

/// Radek shellu nad skutecnym obrazem disku po behu systemu
/// (`OS_KEEP_DATA=1 cargo xtask test`, pak
/// `OS_FS_IMAGE=target/data-N.img cargo test -p fs -- --ignored --nocapture perf_image`).
#[test]
#[ignore]
fn perf_image_shell_line() {
    let Some(path) = std::env::var_os("OS_FS_IMAGE") else {
        std::eprintln!("OS_FS_IMAGE neni nastavena");
        return;
    };
    let data = std::fs::read(&path).unwrap();
    let mut dev = crate::MemDevice::new(0);
    dev.data = data;
    let t = Instant::now();
    let mut fs = crate::Fs::mount(dev).unwrap();
    std::eprintln!("pripojeni: {:?}", t.elapsed());
    std::eprintln!("datasety: {}", fs.datasets().unwrap().len());
    fs.create("/perf", Kind::Dir).ok();
    fs.create("/perf/a.txt", Kind::File).ok();
    fs.create("/tmp", Kind::Dir).ok();
    fs.commit().unwrap();
    for round in 0..3 {
        let mut tt = Instant::now();
        let lap = |what: &str, tt: &mut Instant| {
            std::eprintln!("  {what}: {:?}", tt.elapsed());
            *tt = Instant::now();
        };
        let v = fs.view("/").unwrap();
        lap("view", &mut tt);
        let clone = format!("/tmp/perf{round}");
        fs.aside_begin("/", &clone).unwrap();
        lap("aside_begin", &mut tt);
        fs.create(&format!("{clone}/perf/t{round}"), Kind::File).unwrap();
        lap("create", &mut tt);
        fs.write(&format!("{clone}/perf/a.txt"), 0, format!("zmena {round}").as_bytes()).unwrap();
        lap("write", &mut tt);
        let d = fs.diff_clone(&clone).unwrap();
        lap(&format!("diff_clone ({} zmen)", d.len()), &mut tt);
        fs.aside_apply(&clone, None).unwrap();
        lap("aside_apply", &mut tt);
        fs.aside_discard(&clone).unwrap();
        lap("aside_discard", &mut tt);
        fs.release_view("/", &v.snapshot).unwrap();
        lap("release_view", &mut tt);
        fs.commit().unwrap();
        lap("commit", &mut tt);
    }
}
