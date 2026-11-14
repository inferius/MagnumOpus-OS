//! DMA po kusech pro ovladace: mapovani pripnute pameti do domeny zarizeni.
//!
//! ```ignore
//! let buf = Vmo::create_pinned(64 << 10)?;            // nesouvisle stranky
//! let map = DmaMap::new(&resource, &buf, 0, 64 << 10, map::DEVICE_WRITE)?;
//! for seg in map.segments()? {                         // s IOMMU jeden usek
//!     // zapsat seg.addr / seg.len do deskriptoru zarizeni
//! }
//! map.sync(0, 64 << 10, sync::FOR_CPU)?;               // po prenosu
//! drop(map);                                           // odmapovani + IOTLB
//! ```
//!
//! Buffer jineho procesu (aplikace) jde namapovat jen s handlem, ktery ma
//! pravo `DMA` (odesilatel rozhoduje). Bez IOMMU jsou useky fyzicke adresy
//! a zarizeni smi kamkoli. Popis: `docs/cs/dma.md`.

use alloc::vec::Vec;

pub use abi::dma::{Segment, map, sync};
use abi::Result;

use crate::handle::object_type;
use crate::hw::Resource;
use crate::{Handle, Vmo, sys};

/// Mapovani pameti pro zarizeni; zavreni handlu = odmapovani (jadro
/// vyprazdni IOTLB drive, nez `drop` skonci).
#[derive(Debug)]
pub struct DmaMap(Handle);
object_type!(DmaMap);

impl DmaMap {
    /// Zpristupni `len` bajtu objektu `vmo` od `offset` (zarovnane na
    /// stranky) zarizeni prostredku `resource`. `flags`: [`map::DEVICE_READ`]
    /// a/nebo [`map::DEVICE_WRITE`]. Adresu vybere jadro.
    pub fn new(resource: &Resource, vmo: &Vmo, offset: u64, len: u64, flags: u64) -> Result<Self> {
        Self::at(resource, vmo, offset, len, flags, 0)
    }

    /// Jako [`DmaMap::new`], ale na adrese `iova` zvolene ovladacem (jen
    /// s IOMMU; jadro overi, ze se neprekryva s jinym mapovanim).
    pub fn at(resource: &Resource, vmo: &Vmo, offset: u64, len: u64, flags: u64, iova: u64) -> Result<Self> {
        sys::dma_map(resource.handle().raw(), vmo.handle().raw(), offset, len, flags, iova)
            .map(|h| Self(Handle::from_raw(h)))
    }

    /// Vsechny useky z pohledu zarizeni (s IOMMU jeden souvisly).
    pub fn segments(&self) -> Result<Vec<Segment>> {
        let mut out = alloc::vec![Segment::default(); 1];
        loop {
            let n = sys::dma_segments(self.0.raw(), &mut out)?;
            if n <= out.len() {
                out.truncate(n);
                return Ok(out);
            }
            out.resize(n, Segment::default());
        }
    }

    /// Adresa zacatku pro zarizeni (prvni usek).
    pub fn device_addr(&self) -> Result<u64> {
        let mut one = [Segment::default()];
        sys::dma_segments(self.0.raw(), &mut one)?;
        Ok(one[0].addr)
    }

    /// Udrzba cache rozsahu mapovani ([`sync::FOR_DEVICE`] pred prenosem
    /// do zarizeni, [`sync::FOR_CPU`] po prenosu do pameti). Na koherentnich
    /// platformach nedela nic; `FOR_CPU` navic vybere hlaseni chyb IOMMU.
    pub fn sync(&self, offset: u64, len: u64, direction: u64) -> Result<()> {
        sys::dma_sync(self.0.raw(), offset, len, direction)
    }
}
