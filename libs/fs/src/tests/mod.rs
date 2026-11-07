//! Testy souboroveho systemu (bezi na hostiteli: `cargo test -p fs`).
//!
//! Pocet nahodnych pripadu property testu lze zvysit promennou prostredi
//! `PROPTEST_CASES` (napr. `PROPTEST_CASES=5000` pro dukladny beh).

mod aside_crash;
mod aside_nested;
mod basic;
mod commits;
mod crash;
mod crash3;
mod data;
mod datasets;
mod errors;
mod fastdiff;
mod features;
mod handles;
mod history;
mod journal;
mod model;
mod versions;
mod mount;
mod objects;
mod ondisk;
mod perf;
mod perm;
mod scrub;
mod sharing;
mod times;
mod trash;

use crate::{Algo, BlockDevice, FormatOptions, Fs, MemDevice};

/// Pocet pripadu: `PROPTEST_CASES`, jinak `default`.
fn cases(default: u32) -> u32 {
    std::env::var("PROPTEST_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

fn opts(label: &str) -> FormatOptions {
    FormatOptions {
        label: label.into(),
        algo: Algo::Xxh3,
        ag_blocks: None,
        uuid: None,
        time: crate::layout::Timestamp::UNIX_EPOCH,
    }
}

/// Novy pripojeny system v pameti.
fn new_fs(blocks: u64) -> Fs<MemDevice> {
    let dev = Fs::format(MemDevice::new(blocks), &opts("test")).unwrap();
    Fs::mount(dev).unwrap()
}

/// Novy system s danymi parametry.
fn new_fs_with(blocks: u64, o: &FormatOptions) -> Fs<MemDevice> {
    let dev = Fs::format(MemDevice::new(blocks), o).unwrap();
    Fs::mount(dev).unwrap()
}

/// Odpoji (bez commitu) a znovu pripoji.
fn remount<D: BlockDevice>(fs: Fs<D>) -> Fs<D> {
    Fs::mount(fs.into_device()).unwrap()
}

/// Kontrola konzistence musi projit bez problemu.
fn assert_clean<D: BlockDevice>(fs: &mut Fs<D>) {
    let r = fs.check(true).unwrap();
    assert!(
        r.problems.is_empty(),
        "kontrola konzistence: {:#?}",
        r.problems
    );
}
