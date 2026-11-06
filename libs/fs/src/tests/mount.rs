//! Pripojeni: vyber superbloku, navrat o generaci, rezim jen pro cteni.

use std::cell::Cell;
use std::rc::Rc;

use super::{assert_clean, new_fs};
use crate::layout::{SB_COUNT, SB_FIRST, Superblock};
use crate::{BLOCK_SIZE, BlockDevice, Durability, Error, Fs, Kind, MemDevice, Result};

/// Pamet se zvolitelnymi chybami cteni a zarukou trvanlivosti.
struct TestDevice {
    inner: MemDevice,
    /// Cteni tohoto bloku selze.
    bad_read: Option<u64>,
    /// Kazde cteni mimo superbloky selze (prechodna chyba disku).
    fail_reads: Rc<Cell<bool>>,
    durability: Durability,
}

impl TestDevice {
    fn new(inner: MemDevice) -> Self {
        Self {
            inner,
            bad_read: None,
            fail_reads: Rc::new(Cell::new(false)),
            durability: Durability::Flush,
        }
    }
}

impl BlockDevice for TestDevice {
    fn block_count(&self) -> u64 {
        self.inner.block_count()
    }

    fn read_block(&mut self, block: u64, buf: &mut [u8]) -> Result<()> {
        let sb = (SB_FIRST..SB_FIRST + SB_COUNT).contains(&block);
        if self.bad_read == Some(block) || (!sb && self.fail_reads.get()) {
            return Err(Error::IoError);
        }
        self.inner.read_block(block, buf)
    }

    fn write_block(&mut self, block: u64, data: &[u8]) -> Result<()> {
        self.inner.write_block(block, data)
    }

    fn flush(&mut self) -> Result<()> {
        Ok(())
    }

    fn durability(&self) -> Durability {
        self.durability
    }
}

/// Svazek se dvema commity; vraci zarizeni a slot nejnovejsiho superbloku.
fn two_generations() -> (MemDevice, u64) {
    let mut fs = new_fs(1024);
    fs.create("/stary", Kind::File).unwrap();
    fs.commit().unwrap();
    fs.create("/novy", Kind::File).unwrap();
    fs.commit().unwrap();
    let generation = fs.info().generation;
    (fs.into_device(), SB_FIRST + generation % SB_COUNT)
}

#[test]
fn unreadable_slot_is_skipped() {
    let (dev, newest) = two_generations();
    let mut dev = TestDevice::new(dev);
    dev.bad_read = Some(newest);
    // Nejnovejsi slot nejde precist: predchozi generace jen pro cteni.
    let mut fs = Fs::mount(dev).unwrap();
    assert!(fs.info().read_only);
    assert!(fs.stat("/stary").is_ok());
    assert_eq!(fs.stat("/novy").err(), Some(Error::NotFound));
}

#[test]
fn transient_error_does_not_fall_back() {
    let (dev, _) = two_generations();
    let dev = TestDevice::new(dev);
    dev.fail_reads.set(true);
    // Prechodna chyba I/O: zadny navrat na starsi stav, ale chyba.
    assert_eq!(Fs::mount(dev).err(), Some(Error::IoError));
}

#[test]
fn fallback_invalidates_newer_slots() {
    let (mut dev, newest) = two_generations();
    let off = newest as usize * BLOCK_SIZE;
    let sb = Superblock::decode(&dev.data[off..off + BLOCK_SIZE]).unwrap();
    let root = sb.root.block as usize * BLOCK_SIZE;
    dev.data[root + 100] ^= 0xFF;
    let mut fs = Fs::mount(dev).unwrap();
    assert_eq!(fs.info().fallback, 1);
    fs.create("/po-navratu", Kind::File).unwrap();
    fs.commit().unwrap();
    // Poskozeny novejsi stav je pryc (slot prepsal novy commit nebo se
    // vynuloval): dalsi pripojeni uz se nevraci.
    let dev = fs.into_device();
    for slot in SB_FIRST..SB_FIRST + SB_COUNT {
        let o = slot as usize * BLOCK_SIZE;
        if let Ok(other) = Superblock::decode(&dev.data[o..o + BLOCK_SIZE]) {
            assert_ne!(other.root, sb.root, "slot {slot} stale ukazuje na poskozeny koren");
        }
    }
    let mut fs = Fs::mount(dev).unwrap();
    assert_eq!(fs.info().fallback, 0);
    assert!(fs.stat("/po-navratu").is_ok());
    assert_clean(&mut fs);
}

#[test]
fn no_durability_mounts_read_only() {
    let mut fs = new_fs(1024);
    fs.create("/f", Kind::File).unwrap();
    fs.write("/f", 0, b"ahoj").unwrap();
    fs.commit().unwrap();
    fs.write("/f", 0, b"AHOJ").unwrap();
    fs.fsync().unwrap();
    let mut dev = TestDevice::new(fs.into_device());
    dev.durability = Durability::None;
    let mut fs = Fs::mount(dev).unwrap();
    assert!(fs.info().read_only);
    // Cteni vidi i zmeny z intent logu (prehrane jen v pameti).
    assert_eq!(fs.read("/f", 0, 4).unwrap(), b"AHOJ");
    assert_eq!(fs.create("/g", Kind::File).err(), Some(Error::ReadOnly));
    assert_eq!(fs.write("/f", 0, b"x").err(), Some(Error::ReadOnly));
    assert_eq!(fs.commit().err(), Some(Error::ReadOnly));
    // Write-through zarizeni je zapisovatelne.
    let mut dev = TestDevice::new(fs.into_device().inner);
    dev.durability = Durability::WriteThrough;
    let mut fs = Fs::mount(dev).unwrap();
    assert!(!fs.info().read_only);
    fs.create("/g", Kind::File).unwrap();
}

#[test]
fn fallback_by_two_generations_zeroes_newest_slot() {
    let mut fs = new_fs(1024);
    for name in ["/a", "/b", "/c"] {
        fs.create(name, Kind::File).unwrap();
        fs.commit().unwrap();
    }
    let g = fs.info().generation;
    let mut dev = fs.into_device();
    // Poskodit koreny dvou nejnovejsich generaci (g a g-1).
    let mut roots = alloc::vec::Vec::new();
    for generation in [g, g - 1] {
        let o = (SB_FIRST + generation % SB_COUNT) as usize * BLOCK_SIZE;
        let sb = Superblock::decode(&dev.data[o..o + BLOCK_SIZE]).unwrap();
        roots.push(sb.root);
        dev.data[sb.root.block as usize * BLOCK_SIZE + 100] ^= 0xFF;
    }
    let mut fs = Fs::mount(dev).unwrap();
    assert_eq!(fs.info().fallback, 2);
    assert!(fs.stat("/a").is_ok());
    // Commit po navratu zapise generaci g-1; slot generace g musi zmizet,
    // jinak by pri dalsim pripojeni vyhral.
    let dev = fs.into_device();
    for slot in SB_FIRST..SB_FIRST + SB_COUNT {
        let o = slot as usize * BLOCK_SIZE;
        if let Ok(sb) = Superblock::decode(&dev.data[o..o + BLOCK_SIZE]) {
            assert!(!roots.contains(&sb.root), "slot {slot} ukazuje na poskozeny koren");
            assert!(sb.generation < g);
        }
    }
    let mut fs = Fs::mount(dev).unwrap();
    assert_eq!(fs.info().fallback, 0);
    assert_clean(&mut fs);
}

/// Prepise nejnovejsi superblok funkci `f` nad jeho bajty a prepocita soucet.
fn patch_newest(dev: &mut MemDevice, newest: u64, f: impl FnOnce(&mut [u8])) {
    let o = newest as usize * BLOCK_SIZE;
    let b = &mut dev.data[o..o + BLOCK_SIZE];
    f(b);
    let c = crate::hash::xxh3(&b[..BLOCK_SIZE - 16]);
    b[BLOCK_SIZE - 16..].copy_from_slice(&c.0);
}

#[test]
fn unknown_incompat_refuses_mount() {
    // Scenar z review: generace 2 s neznamym bitem incompat nesmi vest
    // k tichemu pripojeni generace 1 (a zapisum do stareho stavu).
    let (mut dev, newest) = two_generations();
    patch_newest(&mut dev, newest, |b| b[72..80].copy_from_slice(&(1u64 << 63).to_le_bytes()));
    assert_eq!(Fs::mount(dev).err(), Some(Error::NotSupported));
}

#[test]
fn newer_format_version_refuses_mount() {
    let (mut dev, newest) = two_generations();
    patch_newest(&mut dev, newest, |b| {
        b[7] = b'4';
        b[8..12].copy_from_slice(&4u32.to_le_bytes());
    });
    assert_eq!(Fs::mount(dev).err(), Some(Error::NotSupported));
}

#[test]
fn unknown_ro_compat_mounts_read_only_without_writes() {
    let (mut dev, newest) = two_generations();
    patch_newest(&mut dev, newest, |b| b[64..72].copy_from_slice(&(1u64 << 62).to_le_bytes()));
    let before = dev.data.clone();
    let mut fs = Fs::mount(dev).unwrap();
    let info = fs.info();
    assert!(info.read_only);
    assert_eq!(info.fallback, 0);
    // Nejnovejsi generace je videt (zadny tichy navrat).
    assert!(fs.stat("/novy").is_ok());
    assert_eq!(fs.create("/x", Kind::File).err(), Some(Error::ReadOnly));
    assert_eq!(fs.fsync().err(), Some(Error::ReadOnly));
    // Na disk se nezapsalo nic (ani replay, ani commit).
    assert!(fs.into_device().data == before);
}

#[test]
fn bad_values_in_newest_superblock_count_as_fallback() {
    // Platny soucet, ale nesmyslny obsah (ag_blocks = 0): poskozeni, navrat
    // se ohlasi (fallback = 1), ne tise preskoci.
    let (mut dev, newest) = two_generations();
    patch_newest(&mut dev, newest, |b| b[32..40].fill(0));
    let mut fs = Fs::mount(dev).unwrap();
    assert_eq!(fs.info().fallback, 1);
    assert!(fs.stat("/stary").is_ok());
}

#[test]
fn unreadable_newest_slot_is_read_only_and_keeps_newest() {
    // Scenar z review: necitelny slot generace 3 nesmi vest k zapisu nad
    // generaci 2, ktery by generaci 3 prepsal.
    let mut fs = new_fs(1024);
    for name in ["/g1", "/g2", "/g3"] {
        fs.create(name, Kind::File).unwrap();
        fs.commit().unwrap();
    }
    let g = fs.info().generation;
    let mut dev = TestDevice::new(fs.into_device());
    dev.bad_read = Some(SB_FIRST + g % SB_COUNT);
    let mut fs = Fs::mount(dev).unwrap();
    let info = fs.info();
    assert!(info.read_only);
    assert_eq!(info.unreadable_superblocks, 1);
    assert_eq!(fs.create("/x", Kind::File).err(), Some(Error::ReadOnly));
    assert_eq!(fs.commit().err(), Some(Error::ReadOnly));
    // Chyba odezni: nejnovejsi generace je porad cela.
    let mut dev = fs.into_device();
    dev.bad_read = None;
    let mut fs = Fs::mount(dev).unwrap();
    assert!(!fs.info().read_only);
    assert!(fs.stat("/g3").is_ok());
}

#[test]
fn unreadable_log_keeps_confirmed_fsync() {
    // Scenar z review: chyba cteni druhe davky logu nesmi zahodit potvrzeny
    // fsync (dalsi commit by log zahodil natrvalo).
    let mut fs = new_fs(1024);
    fs.commit().unwrap();
    fs.create("/prvni", Kind::File).unwrap();
    fs.fsync().unwrap();
    let second = fs.test_zil_next();
    fs.create("/druhy", Kind::File).unwrap();
    fs.fsync().unwrap();
    let mut dev = TestDevice::new(fs.into_device());
    dev.bad_read = Some(second);
    let mut fs = Fs::mount(dev).unwrap();
    let info = fs.info();
    assert!(info.read_only && info.log_unreadable);
    assert!(fs.stat("/prvni").is_ok());
    assert_eq!(fs.create("/x", Kind::File).err(), Some(Error::ReadOnly));
    // Po odezneni chyby se prehraji obe davky.
    let mut dev = fs.into_device();
    dev.bad_read = None;
    let mut fs = Fs::mount(dev).unwrap();
    assert!(!fs.info().read_only);
    assert!(fs.stat("/prvni").is_ok());
    assert!(fs.stat("/druhy").is_ok());
    assert_eq!(fs.info().replayed, 2);
    assert_clean(&mut fs);
}

#[test]
fn check_works_read_only_without_writes() {
    // Scenar z review: pri nejiste obnove zjistit, co je citelne, bez
    // zapisu na medium. Kontroluje se ulozena generace.
    let mut fs = new_fs(1024);
    fs.create("/f", Kind::File).unwrap();
    fs.write("/f", 0, &[5u8; 30_000]).unwrap();
    fs.commit().unwrap();
    let committed = fs.info().generation;
    fs.write("/f", 0, b"po commitu").unwrap();
    fs.fsync().unwrap();
    let mut dev = TestDevice::new(fs.into_device());
    dev.durability = Durability::None;
    let before = dev.inner.data.clone();
    let mut fs = Fs::mount(dev).unwrap();
    assert!(fs.info().read_only);
    let r = fs.check(true).unwrap();
    assert!(r.problems.is_empty(), "{:#?}", r.problems);
    assert_eq!(r.generation, committed);
    assert!(r.live_not_checked);
    assert!(r.data_blocks > 0);
    assert!(fs.into_device().inner.data == before);
}

#[test]
fn bad_old_slot_keeps_volume_writable() {
    // Vadny sektor ve slotu se starou generaci nezpochybnuje nejnovejsi
    // stav: svazek musi zustat zapisovatelny (jinak navzdy jen pro cteni).
    let mut fs = new_fs(1024);
    for name in ["/g1", "/g2", "/g3"] {
        fs.create(name, Kind::File).unwrap();
        fs.commit().unwrap();
    }
    let g = fs.info().generation;
    let mut dev = TestDevice::new(fs.into_device());
    dev.bad_read = Some(SB_FIRST + (g - 2) % SB_COUNT);
    let mut fs = Fs::mount(dev).unwrap();
    let info = fs.info();
    assert!(!info.read_only && !info.newest_uncertain);
    assert_eq!(info.unreadable_superblocks, 1);
    fs.create("/dalsi", Kind::File).unwrap();
    fs.commit().unwrap();
}

#[test]
fn discard_unreadable_log_on_request() {
    let mut fs = new_fs(1024);
    fs.commit().unwrap();
    fs.create("/prvni", Kind::File).unwrap();
    fs.fsync().unwrap();
    let second = fs.test_zil_next();
    fs.create("/druhy", Kind::File).unwrap();
    fs.fsync().unwrap();
    let mut dev = TestDevice::new(fs.into_device());
    dev.bad_read = Some(second);
    // Vedoma volba: zbytek logu zahodit a pokracovat zapisovatelne.
    let opts = crate::MountOptions { discard_unreadable_log: true, ..Default::default() };
    let mut fs = Fs::mount_with(dev, opts).unwrap();
    let info = fs.info();
    assert!(!info.read_only && info.log_discarded);
    assert!(fs.stat("/prvni").is_ok());
    assert_eq!(fs.stat("/druhy").err(), Some(Error::NotFound));
    fs.create("/pokracuji", Kind::File).unwrap();
    fs.commit().unwrap();
    let mut dev = fs.into_device();
    dev.bad_read = None;
    let mut fs = Fs::mount(dev).unwrap();
    assert!(fs.stat("/pokracuji").is_ok());
    assert_clean(&mut fs);
}

#[test]
fn forced_read_only_mount_writes_nothing() {
    let (dev, _) = two_generations();
    let before = dev.data.clone();
    let opts = crate::MountOptions { read_only: true, ..Default::default() };
    let mut fs = Fs::mount_with(dev, opts).unwrap();
    assert!(fs.info().read_only);
    assert!(fs.check(true).unwrap().problems.is_empty());
    assert!(fs.into_device().data == before);
}

#[test]
fn damaged_next_slot_and_unreadable_after_is_uncertain() {
    // Slot pro newest + 1 ma spatny soucet (mohl to byt dokonceny commit
    // s pozdeji poskozenym superblokem) a slot pro newest + 2 je necitelny:
    // novejsi stav nejde vyloucit, jen pro cteni.
    let mut fs = new_fs(1024);
    for name in ["/g1", "/g2", "/g3", "/g4"] {
        fs.create(name, Kind::File).unwrap();
        fs.commit().unwrap();
    }
    let g = fs.info().generation;
    let mut inner = fs.into_device();
    // "Nejnovejsi citelna" bude g - 2: slot g - 1 poskozeny, slot g necitelny.
    let o = (SB_FIRST + (g - 1) % SB_COUNT) as usize * BLOCK_SIZE;
    inner.data[o + 100] ^= 0xFF;
    let mut dev = TestDevice::new(inner);
    dev.bad_read = Some(SB_FIRST + g % SB_COUNT);
    let fs = Fs::mount(dev).unwrap();
    assert!(fs.info().newest_uncertain && fs.info().read_only);
}
