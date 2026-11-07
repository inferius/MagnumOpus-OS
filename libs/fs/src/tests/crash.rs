//! Simulace vypadku proudu: zarizeni drzi "trvaly" obraz a zapisy od
//! posledniho flush. Pri vypadku se z neflushnutych zapisu provede jen
//! nahodna cast (v nahodnem poradi). Po pripojeni musi system odpovidat
//! stavu po poslednim dokoncenem commitu - nebo stavu po commitu, ktery
//! vypadek prerusil (pokud se jeho superblok stihl zapsat cely). Totez pro
//! vypadek behem `fsync` (intent log).

use alloc::collections::BTreeMap;
use alloc::vec::Vec;

use proptest::prelude::*;

use super::{assert_clean, opts};
use crate::{BlockDevice, Fs, Kind, MemDevice, Result};

pub(super) struct CrashDevice {
    pub(super) durable: MemDevice,
    pub(super) pending: Vec<(u64, Vec<u8>)>,
    /// Po kolika zapisech "vypadne proud": dalsi flush uz nic neulozi.
    pub(super) fail_after: Option<usize>,
    pub(super) writes: usize,
}

impl BlockDevice for CrashDevice {
    fn block_count(&self) -> u64 {
        self.durable.block_count()
    }

    fn read_block(&mut self, block: u64, buf: &mut [u8]) -> Result<()> {
        if let Some((_, d)) = self.pending.iter().rev().find(|(b, _)| *b == block) {
            buf.copy_from_slice(d);
            return Ok(());
        }
        self.durable.read_block(block, buf)
    }

    fn write_block(&mut self, block: u64, data: &[u8]) -> Result<()> {
        // Zapis jde do cache zarizeni; trvalym se stane az pri flush.
        self.writes += 1;
        self.pending.push((block, data.to_vec()));
        Ok(())
    }

    fn flush(&mut self) -> Result<()> {
        // Po "vypadku" flush uz nikdy neuspeje (zarizeni je bez proudu).
        if self.fail_after.is_some_and(|n| self.writes > n) {
            return Err(crate::Error::IoError);
        }
        for (b, d) in self.pending.drain(..) {
            self.durable.write_block(b, &d)?;
        }
        Ok(())
    }
}

/// Stav systemu jako mapa cesta -> obsah.
pub(super) fn snapshot_state(fs: &mut Fs<impl BlockDevice>) -> BTreeMap<alloc::string::String, Vec<u8>> {
    let mut out = BTreeMap::new();
    for e in fs.list("/").unwrap() {
        let path = alloc::format!("/{}", e.name);
        let data = fs.read(&path, 0, 1 << 20).unwrap();
        out.insert(path, data);
    }
    out
}

fn scenario(fail_after: usize, keep: Vec<bool>, via_fsync: bool) {
    let dev = Fs::format(MemDevice::new(1024), &opts("crash")).unwrap();
    let dev = CrashDevice {
        durable: dev,
        pending: Vec::new(),
        fail_after: None,
        writes: 0,
    };
    let mut fs = Fs::mount(dev).unwrap();

    // Ulozeny stav A.
    for i in 0..5 {
        let p = alloc::format!("/s{i}");
        fs.create(&p, Kind::File).unwrap();
        fs.write(&p, 0, &alloc::vec![i as u8; 3000 + i * 9000])
            .unwrap();
    }
    fs.commit().unwrap();
    let state_a = snapshot_state(&mut fs);

    // Rozpracovany stav B; behem jeho commitu vypadne proud.
    let mut dev = fs.into_device();
    dev.writes = 0;
    dev.fail_after = Some(fail_after);
    let mut fs = Fs::mount(dev).unwrap();
    for i in 0..5 {
        let p = alloc::format!("/s{i}");
        fs.write(&p, 100, &alloc::vec![0xEE; 20_000]).unwrap();
    }
    fs.remove("/s0").unwrap();
    fs.create("/novy", Kind::File).unwrap();
    fs.write("/novy", 0, b"B").unwrap();
    let state_b = snapshot_state(&mut fs);
    let _ = if via_fsync { fs.fsync() } else { fs.commit() };

    // Vypadek: z neflushnutych zapisu prezije jen nahodna cast.
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
        "po vypadku neni stav A ani B (fail_after = {fail_after}, fsync = {via_fsync})"
    );
    assert_clean(&mut fs);
}

proptest! {
    #![proptest_config(ProptestConfig { cases: super::cases(48), ..ProptestConfig::default() })]

    #[test]
    fn crash_during_commit(fail_after in 0usize..120, keep in proptest::collection::vec(any::<bool>(), 0..120)) {
        scenario(fail_after, keep, false);
    }

    #[test]
    fn crash_during_fsync(fail_after in 0usize..120, keep in proptest::collection::vec(any::<bool>(), 0..120)) {
        scenario(fail_after, keep, true);
    }
}
