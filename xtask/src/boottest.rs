//! Vic startu po sobe na jednom datovem disku (`cargo xtask test --only
//! opakovany-start`, v plne sade po hlavnim behu).
//!
//! Bezne testy maji pri kazdem behu novy datovy disk, takze chyby, ktere se
//! projevi az pri dalsim startu nad stejnym diskem (sluzba zaklada dataset,
//! ktery uz existuje; obnova po restartu; prehrani logu), jim uniknou.
//! Tady se system nastartuje [`BOOTS`]x nad stejnym diskem se sadou
//! `opakovany-start` (`user/init/src/tests/reboot.rs`): kazdy start zapise
//! a commitne a overi data z minuleho. Kazdy beh musi projit sam a po
//! poslednim xtask precte disk na hostiteli (stejna knihovna `fs`):
//! pocitadlo startu = [`BOOTS`] a kontrola konzistence bez nalezu.

use std::path::Path;

use fs::{FileDevice, Fs};

use crate::{Options, Result, build, image_for, qemu, target_dir};

/// Kolikrat nastartovat (prvni start + dva dalsi nad stejnym diskem).
pub const BOOTS: u64 = 3;

pub fn run(opts: &mut Options) -> Result<()> {
    let disk = target_dir().join(format!("opakovany-start-{}-{}.img", opts.arch.name, std::process::id()));
    let _ = std::fs::remove_file(&disk);
    // Jen sada `opakovany-start` (filtr v initfs plati pro vsechny starty).
    let saved: Vec<(String, Vec<u8>)> = opts.extra_initfs.iter().filter(|(n, _)| n == "test-filter").cloned().collect();
    opts.extra_initfs.retain(|(n, _)| n != "test-filter");
    opts.extra_initfs
        .push((String::from("test-filter"), b"suites = opakovany-start\nrepeat = 1\n".to_vec()));
    opts.data_image = Some(disk.clone());
    let r = boots(opts).and_then(|()| verify(&disk));
    opts.data_image = None;
    opts.extra_initfs.retain(|(n, _)| n != "test-filter");
    opts.extra_initfs.extend(saved);
    if std::env::var_os("OS_KEEP_DATA").is_some() {
        eprintln!("xtask: disk opakovaneho startu zustava v {}", disk.display());
    } else {
        let _ = std::fs::remove_file(&disk);
    }
    r?;
    println!("xtask: opakovany start ({BOOTS}x na jednom disku) prosel");
    Ok(())
}

fn boots(opts: &Options) -> Result<()> {
    let artifacts = build(opts)?;
    let image = image_for(&artifacts, opts)?;
    for i in 1..=BOOTS {
        println!("xtask: opakovany start {i}/{BOOTS} na jednom datovem disku");
        qemu::run(&image, &artifacts, opts).map_err(|e| format!("opakovany start {i}/{BOOTS}: {e}"))?;
    }
    Ok(())
}

/// Disk po poslednim startu: pocitadlo a konzistence (na hostiteli).
fn verify(disk: &Path) -> Result<()> {
    let dev = FileDevice::open(disk)?;
    let mut fs = Fs::mount(dev).map_err(|e| format!("disk po opakovanem startu nejde pripojit: {e:?}"))?;
    let raw = fs
        .read("/opakovany-start.txt", 0, 32)
        .map_err(|e| format!("pocitadlo startu chybi: {e:?}"))?;
    let n: u64 = String::from_utf8_lossy(&raw)
        .trim()
        .parse()
        .map_err(|_| "pocitadlo startu je poskozene")?;
    if n != BOOTS {
        return Err(format!("pocitadlo startu {n}, ocekavano {BOOTS} (nektery start nezapsal)").into());
    }
    let report = fs.check(false).map_err(|e| format!("kontrola disku: {e:?}"))?;
    if !report.problems.is_empty() {
        return Err(format!("kontrola disku po opakovanem startu: {:?}", report.problems).into());
    }
    Ok(())
}
