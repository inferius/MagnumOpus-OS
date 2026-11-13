//! Chyba zapisu uprostred commitu (napr. `NO_MEMORY`, kdyz systemu dojde
//! pamet a pozadavek pro ovladac disku nejde poslat): neuspesny commit
//! nesmi nechat stav v pameti nekonzistentni. Dalsi commit ho musi bud
//! dokoncit, nebo vratit chybu - nikdy panika (driv `write_tree` vyjmul
//! uzly z `dirty` pred zapisem a pristi commit spadl na chybejicim klici).
//! Po novem pripojeni musi byt svazek konzistentni.

use std::cell::Cell;
use std::rc::Rc;

use super::{assert_clean, opts};
use crate::{BlockDevice, Error, Fs, Kind, MemDevice, Result};

/// Zarizeni, ktere selze (jednou) na `fail_at`-tem zapisu bloku od zapnuti.
struct OneShotDevice {
    inner: MemDevice,
    writes: Rc<Cell<u64>>,
    fail_at: Rc<Cell<u64>>,
}

impl BlockDevice for OneShotDevice {
    fn block_count(&self) -> u64 {
        self.inner.block_count()
    }

    fn read_block(&mut self, block: u64, buf: &mut [u8]) -> Result<()> {
        self.inner.read_block(block, buf)
    }

    fn write_block(&mut self, block: u64, data: &[u8]) -> Result<()> {
        let n = self.writes.get() + 1;
        self.writes.set(n);
        if n == self.fail_at.get() {
            return Err(Error::NoMemory);
        }
        self.inner.write_block(block, data)
    }

    fn flush(&mut self) -> Result<()> {
        Ok(())
    }
}

/// Svazek s datasetem a obsahem; vraci citace pro injekci chyby.
fn setup() -> (Fs<OneShotDevice>, Rc<Cell<u64>>, Rc<Cell<u64>>) {
    let inner = Fs::format(MemDevice::new(4096), &opts("pamet")).unwrap();
    let writes = Rc::new(Cell::new(0));
    let fail_at = Rc::new(Cell::new(0));
    let dev = OneShotDevice {
        inner,
        writes: writes.clone(),
        fail_at: fail_at.clone(),
    };
    let mut fs = Fs::mount(dev).unwrap();
    fs.create_dataset("/ds").unwrap();
    fs.create("/ds/a", Kind::File).unwrap();
    fs.write("/ds/a", 0, b"puvodni").unwrap();
    fs.commit().unwrap();
    (fs, writes, fail_at)
}

/// Zmeny pro commit: metadata i data v datasetu a v korenu.
fn change(fs: &mut Fs<OneShotDevice>) {
    for i in 0..40 {
        fs.create(&std::format!("/ds/d{i}"), Kind::Dir).unwrap();
    }
    fs.write("/ds/a", 0, b"nove v datasetu").unwrap();
    fs.create("/k", Kind::File).unwrap();
    fs.write("/k", 0, &[7u8; 20000]).unwrap();
}

fn check_after(fs: &mut Fs<OneShotDevice>) {
    assert!(fs.stat("/ds/d39").is_ok(), "slozka z neuspesneho commitu");
    assert_eq!(fs.read("/ds/a", 0, 64).unwrap(), b"nove v datasetu");
    assert_eq!(fs.read("/k", 0, 20000).unwrap(), std::vec![7u8; 20000]);
}

#[test]
fn commit_survives_write_failure_at_every_point() {
    // Kolik zapisu commit udela (beh bez chyby).
    let (mut fs, writes, _) = setup();
    change(&mut fs);
    let start = writes.get();
    fs.commit().unwrap();
    let total = writes.get() - start;
    assert!(total > 5, "commit ma zapisovat");
    for k in 1..=total {
        let (mut fs, writes, fail_at) = setup();
        change(&mut fs);
        fail_at.set(writes.get() + k);
        let first = fs.commit();
        assert!(first.is_err(), "zapis c. {k}: commit mel selhat");
        fail_at.set(0);
        // Zopakovany commit: projde (stav v pameti cely), nebo chyba
        // (svazek ve stavu chyby) - ale nikdy panika.
        match fs.commit() {
            Ok(()) => {
                check_after(&mut fs);
                assert_clean(&mut fs);
            }
            Err(e) => assert_eq!(e, Error::IoError, "zapis c. {k}: opakovany commit"),
        }
        // Po novem pripojeni konzistentni (posledni potvrzeny stav).
        let dev = fs.into_device();
        let mut fs = Fs::mount(dev).unwrap();
        assert_clean(&mut fs);
        assert!(fs.read("/ds/a", 0, 64).is_ok(), "zapis c. {k}: data datasetu po pripojeni");
    }
}

/// Chyba hned v zapisu stromu datasetu (prvni zapis uzlu): pristi commit
/// musi projit a nic se neztrati (presne pripad z plne sady: commit
/// selhal s `NO_MEMORY`, dalsi za 5 s spadl panikou).
#[test]
fn retry_after_tree_write_failure_keeps_everything() {
    let (mut fs, writes, fail_at) = setup();
    // Jen metadata v datasetu: prvni zapis commitu je strom datasetu.
    for i in 0..40 {
        fs.create(&std::format!("/ds/d{i}"), Kind::Dir).unwrap();
    }
    fail_at.set(writes.get() + 1);
    assert_eq!(fs.commit(), Err(Error::NoMemory));
    fail_at.set(0);
    fs.commit().unwrap();
    assert!(fs.stat("/ds/d39").is_ok());
    assert_clean(&mut fs);
    let dev = fs.into_device();
    let mut fs = Fs::mount(dev).unwrap();
    assert!(fs.stat("/ds/d0").is_ok(), "po pripojeni");
    assert_clean(&mut fs);
}
