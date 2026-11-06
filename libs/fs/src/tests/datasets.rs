//! Datasety: vlastni stromy, snapshoty, klony s vazbou na puvodni snapshot,
//! mazani, intent log pres vic datasetu, vypadek behem mazani.

use std::cell::Cell;
use std::rc::Rc;

use abi::Error;

use super::{assert_clean, new_fs, opts, remount};
use crate::{BlockDevice, Fs, Kind, MemDevice, Result};

#[test]
fn dataset_is_folder_with_own_tree() {
    let mut fs = new_fs(4096);
    let id = fs.create_dataset("/projekt").unwrap();
    assert!(id > 1);
    fs.create("/projekt/src", Kind::Dir).unwrap();
    fs.create("/projekt/src/main.rs", Kind::File).unwrap();
    fs.write("/projekt/src/main.rs", 0, b"fn main() {}").unwrap();
    let list = fs.list("/").unwrap();
    assert_eq!(list.iter().find(|e| e.name == "projekt").unwrap().kind, Kind::Dataset);
    let st = fs.stat("/projekt").unwrap();
    assert_eq!(st.kind, Kind::Dir);
    assert_eq!(st.dataset, id);
    assert_eq!(fs.stat("/projekt/src/main.rs").unwrap().dataset, id);
    assert_eq!(fs.stat("/").unwrap().dataset, 1);
    // Bez rucniho commitu: zapis v datasetu ulozi commit vseho.
    fs.commit().unwrap();
    let mut fs = remount(fs);
    assert_eq!(fs.read("/projekt/src/main.rs", 0, 100).unwrap(), b"fn main() {}");
    assert_eq!(fs.datasets().unwrap().len(), 1);
    assert_clean(&mut fs);
}

#[test]
fn dataset_rules() {
    let mut fs = new_fs(4096);
    fs.create_dataset("/a").unwrap();
    fs.create("/a/f", Kind::File).unwrap();
    fs.create("/b", Kind::Dir).unwrap();
    // Presun mezi datasety = kopie dat, ne prejmenovani.
    assert_eq!(fs.rename("/a/f", "/b/f").err(), Some(Error::NotSupported));
    // Dataset se nemaze jako soubor.
    assert_eq!(fs.remove("/a").err(), Some(Error::WrongType));
    assert_eq!(fs.create("/c", Kind::Dataset).err(), Some(Error::InvalidArgs));
    // Prejmenovani datasetu (polozky) je v poradku.
    fs.rename("/a", "/b/a2").unwrap();
    assert!(fs.stat("/b/a2/f").is_ok());
    // Vnoreny dataset brani smazani rodice.
    fs.create_dataset("/b/a2/vnoreny").unwrap();
    assert_eq!(fs.delete_dataset("/b/a2").err(), Some(Error::NotEmpty));
    fs.delete_dataset("/b/a2/vnoreny").unwrap();
    fs.delete_dataset("/b/a2").unwrap();
    assert_eq!(fs.stat("/b/a2").err(), Some(Error::NotFound));
    assert!(fs.datasets().unwrap().is_empty());
    assert_clean(&mut fs);
}

#[test]
fn dataset_snapshots_are_independent() {
    let mut fs = new_fs(4096);
    fs.create_dataset("/p").unwrap();
    fs.create("/p/f", Kind::File).unwrap();
    fs.write("/p/f", 0, b"verze 1").unwrap();
    fs.create("/korenovy", Kind::File).unwrap();
    fs.snapshot_of("/p", "v1").unwrap();
    fs.write("/p/f", 0, b"verze 2").unwrap();
    assert_eq!(fs.snapshot_read_of("/p", "v1", "/f", 0, 100).unwrap(), b"verze 1");
    assert_eq!(fs.read("/p/f", 0, 100).unwrap(), b"verze 2");
    // Snapshot datasetu neni snapshot korene a naopak.
    assert!(fs.snapshots().unwrap().is_empty());
    assert_eq!(fs.snapshots_of("/p").unwrap().len(), 1);
    fs.snapshot("koren").unwrap();
    // Pohled snapshotu korene do datasetu neprechazi.
    assert_eq!(fs.snapshot_stat("koren", "/p/f").err(), Some(Error::NotSupported));
    fs.delete_snapshot_of("/p", "v1").unwrap();
    assert_clean(&mut fs);
}

#[test]
fn two_clones_keep_origin_data_after_snapshot_deletion() {
    // Scenar z review: ze snapshotu S dva zapisovatelne klony, v prvnim
    // se prepise soubor, S se smaze - druhy klon musi cist puvodni data.
    let mut fs = new_fs(8192);
    fs.commit().unwrap();
    let base = fs.info().free_blocks;
    fs.create_dataset("/p").unwrap();
    fs.create("/p/data", Kind::File).unwrap();
    fs.write("/p/data", 0, &[1u8; 200_000]).unwrap();
    fs.snapshot_of("/p", "S").unwrap();
    fs.clone_snapshot("/p", "S", "/k1").unwrap();
    fs.clone_snapshot("/p", "S", "/k2").unwrap();
    assert_eq!(fs.snapshots_of("/p").unwrap()[0].clones, 2);
    fs.write("/k1/data", 0, &[2u8; 200_000]).unwrap();
    // I originalni dataset po S zmeni data.
    fs.write("/p/data", 0, &[3u8; 200_000]).unwrap();
    fs.commit().unwrap();
    fs.delete_snapshot_of("/p", "S").unwrap();
    // S je skryty (drzi bloky pro klony), ne smazany.
    assert!(fs.snapshots_of("/p").unwrap().is_empty());
    assert_clean(&mut fs);
    let mut fs = remount(fs);
    assert_eq!(fs.read("/k2/data", 0, 200_000).unwrap(), [1u8; 200_000]);
    assert_eq!(fs.read("/k1/data", 0, 200_000).unwrap(), [2u8; 200_000]);
    assert_eq!(fs.read("/p/data", 0, 200_000).unwrap(), [3u8; 200_000]);
    // Smazani klonu: po poslednim zmizi i skryty S.
    fs.delete_dataset("/k1").unwrap();
    assert_eq!(fs.read("/k2/data", 0, 4).unwrap(), [1u8; 4]);
    assert_clean(&mut fs);
    fs.delete_dataset("/k2").unwrap();
    assert_clean(&mut fs);
    fs.delete_dataset("/p").unwrap();
    for _ in 0..crate::REUSE_DELAY + 2 {
        fs.commit().unwrap();
    }
    // Ucetnictvi mista: vse se vratilo.
    let free = fs.info().free_blocks;
    assert!(free + 16 >= base, "unik {} bloku", base - free);
    assert_clean(&mut fs);
}

#[test]
fn clone_snapshots_and_restore_copy() {
    let mut fs = new_fs(8192);
    fs.create_dataset("/p").unwrap();
    fs.create("/p/cfg.toml", Kind::File).unwrap();
    fs.write("/p/cfg.toml", 0, b"a = 1").unwrap();
    fs.snapshot_of("/p", "S").unwrap();
    fs.clone_snapshot("/p", "S", "/bokem").unwrap();
    // Klon ma vlastni snapshoty; jejich mazani respektuje hranici puvodu.
    fs.write("/bokem/cfg.toml", 0, b"a = 2").unwrap();
    fs.snapshot_of("/bokem", "K1").unwrap();
    fs.write("/bokem/cfg.toml", 0, b"a = 3").unwrap();
    fs.delete_snapshot_of("/bokem", "K1").unwrap();
    assert_eq!(fs.read("/bokem/cfg.toml", 0, 10).unwrap(), b"a = 3");
    assert_eq!(fs.snapshot_read_of("/p", "S", "/cfg.toml", 0, 10).unwrap(), b"a = 1");
    assert_clean(&mut fs);
    // Snapshot s klonem nejde smazat s datasetem.
    assert_eq!(fs.delete_dataset("/p").err(), Some(Error::BadState));
    fs.delete_dataset("/bokem").unwrap();
    fs.delete_dataset("/p").unwrap();
    assert_clean(&mut fs);
}

#[test]
fn fsync_across_datasets_survives_power_loss() {
    let mut fs = new_fs(4096);
    fs.create_dataset("/a").unwrap();
    fs.create_dataset("/b").unwrap();
    fs.commit().unwrap();
    fs.create("/a/x", Kind::File).unwrap();
    fs.write("/a/x", 0, b"A").unwrap();
    fs.create("/b/y", Kind::File).unwrap();
    fs.write("/b/y", 0, &[7u8; 50_000]).unwrap();
    fs.create("/z", Kind::File).unwrap();
    fs.fsync().unwrap();
    // Bez commitu: prehrani logu musi vratit zmeny ve vsech datasetech.
    let mut fs = remount(fs);
    assert_eq!(fs.read("/a/x", 0, 1).unwrap(), b"A");
    assert_eq!(fs.read("/b/y", 0, 50_000).unwrap(), [7u8; 50_000]);
    assert!(fs.stat("/z").is_ok());
    assert_clean(&mut fs);
}

/// Zarizeni, ktere od `cut` zahazuje zapisy a neumi flush (vypadek proudu
/// uprostred commitu).
struct CutDevice {
    inner: MemDevice,
    cut: Rc<Cell<bool>>,
}

impl BlockDevice for CutDevice {
    fn block_count(&self) -> u64 {
        self.inner.block_count()
    }
    fn read_block(&mut self, block: u64, buf: &mut [u8]) -> Result<()> {
        self.inner.read_block(block, buf)
    }
    fn write_block(&mut self, block: u64, data: &[u8]) -> Result<()> {
        if self.cut.get() {
            return Ok(());
        }
        self.inner.write_block(block, data)
    }
    fn flush(&mut self) -> Result<()> {
        if self.cut.get() { Err(Error::IoError) } else { Ok(()) }
    }
}

#[test]
fn power_loss_during_dataset_deletion() {
    let inner = Fs::format(MemDevice::new(8192), &opts("ds")).unwrap();
    let cut = Rc::new(Cell::new(false));
    let mut fs = Fs::mount(CutDevice { inner, cut: cut.clone() }).unwrap();
    fs.create_dataset("/p").unwrap();
    fs.create("/p/f", Kind::File).unwrap();
    fs.write("/p/f", 0, &[5u8; 100_000]).unwrap();
    fs.snapshot_of("/p", "S").unwrap();
    fs.clone_snapshot("/p", "S", "/k").unwrap();
    fs.write("/k/f", 0, &[6u8; 100_000]).unwrap();
    fs.commit().unwrap();
    // Vypadek behem mazani klonu: stav je pred, nebo po - nikdy napul.
    cut.set(true);
    assert!(fs.delete_dataset("/k").is_err());
    let dev = fs.into_device();
    let mut fs = Fs::mount(dev.inner).unwrap();
    assert_eq!(fs.read("/p/f", 0, 4).unwrap(), [5u8; 4]);
    if fs.stat("/k").is_ok() {
        assert_eq!(fs.read("/k/f", 0, 4).unwrap(), [6u8; 4]);
    }
    assert_clean(&mut fs);
    // A smazat jde i po obnove.
    if fs.stat("/k").is_ok() {
        fs.delete_dataset("/k").unwrap();
    }
    fs.delete_snapshot_of("/p", "S").unwrap();
    assert_clean(&mut fs);
}
