//! Chyby zarizeni: zadna chyba zapisu nesmi ztratit predana data ani vest
//! k falesnemu potvrzeni trvanlivosti.

use std::cell::Cell;
use std::rc::Rc;

use super::{assert_clean, opts};
use crate::{BlockDevice, Error, Fs, Kind, MemDevice, Result};

/// Zarizeni, kteremu jde zvenku "vypnout" zapis nebo flush.
struct FlakyDevice {
    inner: MemDevice,
    fail_writes: Rc<Cell<bool>>,
    fail_flush: Rc<Cell<bool>>,
}

impl BlockDevice for FlakyDevice {
    fn block_count(&self) -> u64 {
        self.inner.block_count()
    }

    fn read_block(&mut self, block: u64, buf: &mut [u8]) -> Result<()> {
        self.inner.read_block(block, buf)
    }

    fn write_block(&mut self, block: u64, data: &[u8]) -> Result<()> {
        if self.fail_writes.get() {
            return Err(Error::IoError);
        }
        self.inner.write_block(block, data)
    }

    fn flush(&mut self) -> Result<()> {
        if self.fail_flush.get() {
            return Err(Error::IoError);
        }
        Ok(())
    }
}

struct Setup {
    fs: Fs<FlakyDevice>,
    writes: Rc<Cell<bool>>,
    flush: Rc<Cell<bool>>,
}

fn setup() -> Setup {
    let inner = Fs::format(MemDevice::new(2048), &opts("chyby")).unwrap();
    let writes = Rc::new(Cell::new(false));
    let flush = Rc::new(Cell::new(false));
    let dev = FlakyDevice {
        inner,
        fail_writes: writes.clone(),
        fail_flush: flush.clone(),
    };
    Setup {
        fs: Fs::mount(dev).unwrap(),
        writes,
        flush,
    }
}

#[test]
fn failed_data_write_keeps_data_and_retries() {
    let Setup { mut fs, writes, .. } = setup();
    fs.create("/f", Kind::File).unwrap();
    fs.write("/f", 0, &[1u8; 20_000]).unwrap();
    fs.commit().unwrap();
    fs.write("/f", 0, &[2u8; 20_000]).unwrap();
    // Zapis dat pri fsync selze: chyba, ale bez ztraty a bez stavu chyby.
    writes.set(true);
    assert_eq!(fs.fsync(), Err(Error::IoError));
    writes.set(false);
    assert_eq!(fs.read("/f", 0, 20_000).unwrap(), [2u8; 20_000]);
    // Druhy pokus projde a data jsou po pripojeni (bez commitu) trvala.
    fs.fsync().unwrap();
    let mut fs = Fs::mount(fs.into_device()).unwrap();
    assert_eq!(fs.read("/f", 0, 20_000).unwrap(), [2u8; 20_000]);
    assert_clean(&mut fs);
}

#[test]
fn failed_overwrite_keeps_old_mapping() {
    let Setup { mut fs, writes, .. } = setup();
    fs.create("/f", Kind::File).unwrap();
    fs.write("/f", 0, &[1u8; 20_000]).unwrap();
    fs.commit().unwrap();
    fs.write("/f", 0, &[2u8; 20_000]).unwrap();
    writes.set(true);
    assert!(fs.fsync().is_err());
    writes.set(false);
    // Bez commitu po pripojeni: stara data cela (zadna dira po prepsani).
    let mut fs = Fs::mount(fs.into_device()).unwrap();
    assert_eq!(fs.read("/f", 0, 20_000).unwrap(), [1u8; 20_000]);
    assert_clean(&mut fs);
}

#[test]
fn failed_flush_never_confirms_later() {
    let Setup { mut fs, flush, .. } = setup();
    fs.commit().unwrap();
    fs.create("/a", Kind::File).unwrap();
    flush.set(true);
    assert_eq!(fs.fsync(), Err(Error::IoError));
    flush.set(false);
    // Po nejistem zapisu logu uz nic dalsiho nepotvrdi (falesne "trvale").
    assert_eq!(fs.fsync(), Err(Error::IoError));
    assert_eq!(fs.create("/b", Kind::File).err(), Some(Error::IoError));
    // Znovu pripojit: stav je konzistentni (davka cela, nebo vubec).
    let mut fs = Fs::mount(fs.into_device()).unwrap();
    assert_clean(&mut fs);
}

#[test]
fn write_result_means_accepted_not_durable() {
    // Scenar z review: write pres 16 MiB pri selhavajicim zapisu vratil
    // chybu, a presto se data ulozila. Ted: Ok = prijato, chyba = nic.
    let Setup { mut fs, writes, .. } = setup_big();
    fs.create("/v", Kind::File).unwrap();
    writes.set(true);
    let chunk = [7u8; 1 << 20];
    let mut accepted = 0u64;
    let mut refused = false;
    for i in 0..48u64 {
        match fs.write("/v", i << 20, &chunk) {
            Ok(()) => accepted += 1,
            Err(e) => {
                assert_eq!(e, Error::IoError);
                refused = true;
                break;
            }
        }
    }
    // Buffer se zastavil na tvrdem limitu (32 MiB), nic za nim neprijato.
    assert!(refused);
    assert_eq!(fs.stat("/v").unwrap().size, accepted << 20);
    assert_eq!(fs.fsync(), Err(Error::IoError));
    // Zarizeni se vzpamatuje: ulozi se presne prijata data.
    writes.set(false);
    fs.commit().unwrap();
    let mut fs = Fs::mount(fs.into_device()).unwrap();
    assert_eq!(fs.stat("/v").unwrap().size, accepted << 20);
    assert_eq!(fs.read("/v", (accepted - 1) << 20, 4).unwrap(), [7u8; 4]);
    assert_clean(&mut fs);
}

fn setup_big() -> Setup {
    let inner = Fs::format(MemDevice::new(32_768), &opts("velky")).unwrap();
    let writes = Rc::new(Cell::new(false));
    let flush = Rc::new(Cell::new(false));
    let dev = FlakyDevice {
        inner,
        fail_writes: writes.clone(),
        fail_flush: flush.clone(),
    };
    Setup {
        fs: Fs::mount(dev).unwrap(),
        writes,
        flush,
    }
}
