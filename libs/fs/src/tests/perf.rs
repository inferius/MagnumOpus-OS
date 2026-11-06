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
