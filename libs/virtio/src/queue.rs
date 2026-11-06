//! Split virtqueue: tabulka deskriptoru, fronta "available" (ovladac ->
//! zarizeni) a fronta "used" (zarizeni -> ovladac), vse v DMA pameti.
//!
//! Ovladac v tehle verzi posila pozadavky synchronne (jeden retezec a ceka
//! na jeho dokonceni), coz na blokove zarizeni zatim staci.

use core::sync::atomic::{Ordering, fence};

use libos::abi::vm::{MAP_READ, MAP_WRITE};
use libos::hw::Resource;
use libos::{Error, Mapping, Result, Vmo};

const DESC_SIZE: u64 = 16;
const DESC_F_NEXT: u16 = 1;
const DESC_F_WRITE: u16 = 2;

/// Jeden kus retezce pozadavku: fyzicka adresa, delka, zapisuje-li do nej
/// zarizeni.
#[derive(Debug, Clone, Copy)]
pub struct Buffer {
    pub phys: u64,
    pub len: u32,
    pub device_writes: bool,
}

pub struct Virtqueue {
    _vmo: Vmo,
    map: Mapping,
    phys: u64,
    size: u16,
    index: u16,
    notify_offset: u64,
    avail_off: u64,
    used_off: u64,
    avail_idx: u16,
    last_used: u16,
}

impl Virtqueue {
    pub(crate) fn new(
        size: u16,
        resource: &Resource,
        index: u16,
        notify_offset: u64,
    ) -> Result<Self> {
        let n = size as u64;
        let avail_off = n * DESC_SIZE;
        // avail: flags, idx, ring[n], used_event; used zarovnany na 4 B.
        let used_off = (avail_off + 4 + 2 * n + 2).next_multiple_of(4);
        let total = used_off + 4 + 8 * n + 2;
        let vmo = resource.dma_memory(total)?;
        let phys = resource.phys_addr(&vmo, 0)?;
        let map = Mapping::new(&vmo, 0, total, MAP_READ | MAP_WRITE)?;
        Ok(Self {
            _vmo: vmo,
            map,
            phys,
            size,
            index,
            notify_offset,
            avail_off,
            used_off,
            avail_idx: 0,
            last_used: 0,
        })
    }

    pub(crate) fn phys_addresses(&self) -> (u64, u64, u64) {
        (
            self.phys,
            self.phys + self.avail_off,
            self.phys + self.used_off,
        )
    }

    pub fn index(&self) -> u16 {
        self.index
    }

    pub(crate) fn notify_offset(&self) -> u64 {
        self.notify_offset
    }

    /// Vlozi retezec bufferu (deskriptory 0..n) do fronty. Pred dalsim
    /// pozadavkem je treba pockat na dokonceni (`take_used`).
    pub fn submit(&mut self, chain: &[Buffer]) -> Result<()> {
        if chain.is_empty() || chain.len() > self.size as usize {
            return Err(Error::InvalidArgs);
        }
        for (i, b) in chain.iter().enumerate() {
            let d = i as u64 * DESC_SIZE;
            let mut flags = if b.device_writes { DESC_F_WRITE } else { 0 };
            if i + 1 < chain.len() {
                flags |= DESC_F_NEXT;
            }
            self.map.write::<u64>(d, b.phys);
            self.map.write::<u32>(d + 8, b.len);
            self.map.write::<u16>(d + 12, flags);
            self.map.write::<u16>(d + 14, (i + 1) as u16);
        }
        let slot = self.avail_off + 4 + 2 * (self.avail_idx % self.size) as u64;
        self.map.write::<u16>(slot, 0); // hlava retezce = deskriptor 0
        // Deskriptory a polozka fronty musi byt viditelne driv nez novy index.
        fence(Ordering::SeqCst);
        self.avail_idx = self.avail_idx.wrapping_add(1);
        self.map.write::<u16>(self.avail_off + 2, self.avail_idx);
        fence(Ordering::SeqCst);
        Ok(())
    }

    /// Vyjme dokonceny pozadavek, pokud nejaky je: (id hlavy, zapsana delka).
    /// Vyzvedne dokonceny pozadavek: `Ok(None)` = zatim nic. Zarizeni, ktere
    /// posune index o vic nez velikost fronty nebo vrati neznamy retezec,
    /// je vadne (`CORRUPTED`) - driver ho pak nesmi dal pouzivat (DMA
    /// buffery by mohly byt jeste prepisovany).
    pub fn take_used(&mut self) -> Result<Option<(u32, u32)>> {
        fence(Ordering::SeqCst);
        let used_idx: u16 = self.map.read(self.used_off + 2);
        if used_idx == self.last_used {
            return Ok(None);
        }
        if used_idx.wrapping_sub(self.last_used) > self.size {
            return Err(Error::Corrupted);
        }
        fence(Ordering::SeqCst);
        let e = self.used_off + 4 + 8 * (self.last_used % self.size) as u64;
        let id: u32 = self.map.read(e);
        let len: u32 = self.map.read(e + 4);
        self.last_used = self.last_used.wrapping_add(1);
        // Fronta ma jediny retezec s hlavou v deskriptoru 0.
        if id != 0 {
            return Err(Error::Corrupted);
        }
        Ok(Some((id, len)))
    }
}
