//! Vypadek proudu se strukturami faze 3: komprimovane kusy, sdilene bloky
//! (kopie, deduplikace), predalokace se zapisem na misto, kopie dat.
//! Po pripojeni musi byt stav A (posledni commit) nebo B (rozpracovany)
//! a kontrola konzistence (vcetne tabulky sdileni) cista.

use alloc::vec::Vec;

use proptest::prelude::*;

use super::crash::{CrashDevice, snapshot_state};
use super::data::{noise, text};
use super::{assert_clean, opts};
use crate::datarec::{DataSettings, DedupMode};
use crate::layout::Compress;
use crate::{BlockDevice, Fs, Kind, MemDevice};

fn scenario(fail_after: usize, keep: Vec<bool>, via_fsync: bool, copies: u8) {
    let dev = Fs::format(MemDevice::new(4096), &opts("crash3")).unwrap();
    let dev = CrashDevice {
        durable: dev,
        pending: Vec::new(),
        fail_after: None,
        writes: 0,
    };
    let mut fs = Fs::mount(dev).unwrap();
    fs.set_data_settings(
        "/",
        &DataSettings {
            compress: Some(Compress::Lz4),
            copies,
            dedup: DedupMode::Background,
        },
    )
    .unwrap();
    // Stav A: komprimovany soubor, jeho kopie, predalokace, duplikaty.
    fs.create("/t", Kind::File).unwrap();
    fs.write("/t", 0, &text(300_000, 1)).unwrap();
    fs.clone_path("/t", "/t2").unwrap();
    fs.create("/db", Kind::File).unwrap();
    fs.fallocate("/db", 0, 512 * 1024, false).unwrap();
    for p in ["/d1", "/d2"] {
        fs.create(p, Kind::File).unwrap();
        fs.write(p, 0, &noise(256 * 1024, 5)).unwrap();
    }
    fs.commit().unwrap();
    while !fs.dedup_step(64).unwrap().done {}
    fs.commit().unwrap();
    let state_a = snapshot_state(&mut fs);

    // Stav B: zmeny nad sdilenymi, komprimovanymi a predalokovanymi bloky.
    let mut dev = fs.into_device();
    dev.writes = 0;
    dev.fail_after = Some(fail_after);
    let mut fs = Fs::mount(dev).unwrap();
    fs.write("/t", 10_000, &[0xEE; 5000]).unwrap();
    fs.write("/t2", 200_000, &noise(70_000, 9)).unwrap();
    fs.remove("/d1").unwrap();
    fs.write("/d2", 4096, b"zmena").unwrap();
    fs.write("/db", 64 * 1024, &text(40_000, 3)).unwrap();
    fs.truncate("/t", 150_001).unwrap();
    let state_b = snapshot_state(&mut fs);
    let _ = if via_fsync { fs.fsync() } else { fs.commit() };

    let mut dev = fs.into_device();
    let pending = core::mem::take(&mut dev.pending);
    for (i, (b, d)) in pending.into_iter().enumerate() {
        if keep.get(i).copied().unwrap_or(false) {
            dev.durable.write_block(b, &d).unwrap();
        }
    }
    let mut fs = Fs::mount(dev.durable).expect("system po vypadku nejde pripojit");
    let after = snapshot_state(&mut fs);
    assert!(
        after == state_a || after == state_b,
        "po vypadku neni stav A ani B (fail_after = {fail_after}, fsync = {via_fsync}, kopie = {copies})"
    );
    assert_clean(&mut fs);
}

proptest! {
    #![proptest_config(ProptestConfig { cases: super::cases(24), ..ProptestConfig::default() })]

    #[test]
    fn crash3_during_commit(fail_after in 0usize..200, keep in proptest::collection::vec(any::<bool>(), 0..200), copies in 1u8..=2) {
        scenario(fail_after, keep, false, copies);
    }

    #[test]
    fn crash3_during_fsync(fail_after in 0usize..200, keep in proptest::collection::vec(any::<bool>(), 0..200), copies in 1u8..=2) {
        scenario(fail_after, keep, true, copies);
    }
}
