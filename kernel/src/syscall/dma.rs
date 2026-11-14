//! DMA po kusech: `DMA_MAP`, `DMA_SEGMENTS`, `DMA_SYNC` (`object/dma.rs`,
//! `abi::dma`).

use alloc::vec::Vec;

use abi::dma::{Segment, map};

use super::handle::arg_handle;
use super::{Args, current_process, user};
use crate::arch::iommu::DmaAccess;
use crate::error::{Error, Result};
use crate::object::dma::DmaMap;
use crate::object::resource::Resource;
use crate::object::vmo::MemoryObject;
use crate::object::{KernelObject, Rights};

/// Nejvic useku v jednom `DMA_SEGMENTS` (bez IOMMU stranka na usek; vic
/// se cte po castech).
const SEGMENTS_MAX: u64 = 4096;

/// `DMA_MAP(resource, vmo, offset, len, flags, iova_hint) -> handle`.
pub fn dma_map(a: Args) -> Result<u64> {
    let p = current_process();
    let flags = a[4];
    if flags & !map::KNOWN != 0 || flags & map::ACCESS == 0 {
        return Err(Error::InvalidArgs);
    }
    let access = DmaAccess {
        read: flags & map::DEVICE_READ != 0,
        write: flags & map::DEVICE_WRITE != 0,
    };
    // Zarizeni cte = cteni pameti, zapisuje = zapis; vzdy pravo DMA (majitel
    // bufferu tak rozhoduje, jestli k nemu smi zarizeni vubec).
    let mut need = Rights::DMA;
    if access.read {
        need = need | Rights::READ;
    }
    if access.write {
        need = need | Rights::WRITE;
    }
    let res = p.handles().get_as::<Resource>(arg_handle(a[0])?, Rights::MANAGE)?;
    let vmo = p.handles().get_as::<MemoryObject>(arg_handle(a[1])?, need)?;
    if flags & map::REQUIRE_IOMMU != 0 && res.device_domain().is_none() {
        return Err(Error::NotSupported);
    }
    let m = DmaMap::new(&res, vmo, a[2], a[3], access, a[5], p.job())?;
    let rights = m.default_rights();
    Ok(p.handles().insert(m, rights)?.0 as u64)
}

/// `DMA_SEGMENTS(map, out_ptr, max) -> pocet`.
pub fn dma_segments(a: Args) -> Result<u64> {
    let p = current_process();
    let m = p.handles().get_as::<DmaMap>(arg_handle(a[0])?, Rights::READ)?;
    if a[2] > SEGMENTS_MAX {
        return Err(Error::InvalidArgs);
    }
    let segs = m.segments();
    let n = segs.len().min(a[2] as usize);
    if n > 0 {
        let mut buf = Vec::new();
        buf.try_reserve_exact(n * Segment::SIZE).map_err(|_| Error::NoMemory)?;
        for s in &segs[..n] {
            buf.extend_from_slice(&s.encode());
        }
        user::write_bytes(&p, a[1], &buf)?;
    }
    Ok(segs.len() as u64)
}

/// `DMA_SYNC(map, offset, len, direction) -> 0`.
pub fn dma_sync(a: Args) -> Result<u64> {
    let p = current_process();
    let m = p.handles().get_as::<DmaMap>(arg_handle(a[0])?, Rights::READ)?;
    m.sync(a[1], a[2], a[3])?;
    Ok(0)
}
