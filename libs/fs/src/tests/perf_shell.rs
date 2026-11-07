//! Cena radku shellu se zapisem a commitu na svazku s mnoha nactenymi
//! datasety: pocet pozadavku na disk (zapisy, flush) nesmi rust s poctem
//! datasetu. V QEMU stoji kazdy pozadavek cestu k ovladaci a zpet, proto se
//! hlida pocet, ne cas (cas meri QEMU sada `vykon FS`).
//!
//! Casy na hostiteli: `cargo test -p fs --release -- --nocapture perf_shell`.

use std::time::Instant;

use super::opts;
use crate::{BlockDevice, Fs, Kind, MemDevice};
use abi::Result;

#[derive(Default, Clone, Copy, Debug, PartialEq, Eq)]
struct Counts {
    writes: u64,
    flushes: u64,
}

struct Counting {
    inner: MemDevice,
    c: Counts,
}

impl BlockDevice for Counting {
    fn block_count(&self) -> u64 {
        self.inner.block_count()
    }
    fn read_block(&mut self, block: u64, buf: &mut [u8]) -> Result<()> {
        self.inner.read_block(block, buf)
    }
    fn write_block(&mut self, block: u64, data: &[u8]) -> Result<()> {
        self.c.writes += 1;
        self.inner.write_block(block, data)
    }
    fn read_blocks(&mut self, start: u64, buf: &mut [u8]) -> Result<()> {
        self.inner.read_blocks(start, buf)
    }
    fn write_blocks(&mut self, start: u64, data: &[u8]) -> Result<()> {
        self.c.writes += 1;
        self.inner.write_blocks(start, data)
    }
    fn flush(&mut self) -> Result<()> {
        self.c.flushes += 1;
        self.inner.flush()
    }
}

/// Provede `f` a vrati pocet pozadavku na disk.
fn io<T>(fs: &mut Fs<Counting>, label: &str, f: impl FnOnce(&mut Fs<Counting>) -> T) -> Counts {
    let before = fs.test_store().dev.c;
    let s = Instant::now();
    f(fs);
    let e = s.elapsed();
    let c = fs.test_store().dev.c;
    let d = Counts { writes: c.writes - before.writes, flushes: c.flushes - before.flushes };
    std::eprintln!("{label}: {e:?}, zapisy {}, flush {}", d.writes, d.flushes);
    d
}

/// Svazek s `n` nactenymi datasety (kazdy se souborem a snapshotem).
fn volume(n: usize) -> Fs<Counting> {
    let dev = Fs::format(Counting { inner: MemDevice::new(60_000), c: Counts::default() }, &opts("t")).unwrap();
    let mut fs = Fs::mount(dev).unwrap();
    for d in ["/tmp", "/tmp/.aside", "/home"] {
        fs.create(d, Kind::Dir).unwrap();
    }
    fs.create("/home/a", Kind::File).unwrap();
    for i in 0..n {
        let d = format!("/ds{i}");
        fs.create_dataset(&d).unwrap();
        fs.create(&format!("{d}/f"), Kind::File).unwrap();
        fs.write(&format!("{d}/f"), 0, b"x").unwrap();
        fs.snapshot_of(&d, "s").unwrap();
    }
    fs.commit().unwrap();
    fs
}

/// Jeden radek shellu: (commit male zmeny, view, aside_begin, aside_changes, aside_apply).
fn line(fs: &mut Fs<Counting>, n: usize) -> [Counts; 5] {
    fs.write("/home/a", 0, b"x").unwrap();
    let commit = io(fs, "commit male zmeny", |fs| fs.commit().unwrap());
    let mut v = None;
    let view = io(fs, "view", |fs| v = Some(fs.view("/").unwrap()));
    let path = format!("/tmp/.aside/s-{n}");
    let begin = io(fs, "aside_begin", |fs| fs.aside_begin_ex("/", &path).unwrap());
    fs.write(&format!("{path}/home/a"), 0, b"novy").unwrap();
    let changes = io(fs, "aside_changes", |fs| fs.diff_clone(&path).unwrap());
    let apply = io(fs, "aside_apply", |fs| fs.aside_apply_with(&path, None, true).unwrap());
    fs.aside_discard(&path).unwrap();
    fs.release_view("/", &v.unwrap().snapshot).unwrap();
    [commit, view, begin, changes, apply]
}

#[test]
fn shell_line_io_independent_of_loaded_datasets() {
    let mut few = volume(2);
    let mut many = volume(300);
    for round in 0..3 {
        let a = line(&mut few, round);
        let b = line(&mut many, round);
        let [commit, view, begin, changes, apply] = b;
        // Commit: strom, misto, superblok a dva flush (poradi zapisu).
        assert_eq!(commit.flushes, 2, "commit male zmeny: {commit:?}");
        // Pohled: jen commit stavu (zaznam az s dalsim commitem).
        assert!(view.flushes <= 2, "view: {view:?}");
        // Zacatek bokem: stav + (zaznam snapshotu a klon) = dva commity.
        assert!(begin.flushes <= 4, "aside_begin: {begin:?}");
        // Rozdil z pameti: zadny zapis.
        assert_eq!(changes, Counts::default(), "aside_changes");
        assert!(apply.flushes <= 4, "aside_apply: {apply:?}");
        // Pocet pozadavku nezavisi na poctu nactenych datasetu.
        for (i, (x, y)) in a.iter().zip(b.iter()).enumerate() {
            assert_eq!(x.flushes, y.flushes, "krok {i}: flush 2 vs 300 datasetu");
            // Hlubsi strom korene (vic zaznamu datasetu) = o par uzlu vic,
            // ne linearne s poctem datasetu.
            assert!(y.writes <= x.writes + 4,"krok {i}: zapisy {x:?} (2 datasety) vs {y:?} (300)");
        }
    }
}
