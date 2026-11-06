//! Objekt pameti (memory object): stranky fyzicke pameti, ktere lze cist,
//! zapisovat a mapovat do adresnich prostoru procesu.
//!
//! Druhy objektu:
//! - bezny: stranky RAM alokovane a vynulovane hned pri vytvoreni,
//! - souvisly: fyzicky souvisla RAM pro DMA (ovladace zjisti fyzickou adresu),
//! - fyzicky: okno na registry zarizeni (MMIO); stranky nepatri alokatoru
//!   a mapuji se bez cache.
//!
//! Objekt nemeni velikost. TODO: lazy alokace pri prvnim pristupu,
//! copy-on-write klony.

use alloc::sync::Arc;
use alloc::vec::Vec;

use super::{Error, KernelObject, ObjectCore, ObjectType, Rights, Signals};
use crate::mm::{PhysAddr, phys};

const PAGE: u64 = abi::vm::PAGE_SIZE;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// Jednotlive stranky z alokatoru.
    Pages,
    /// Jeden souvisly blok z alokatoru (2^order stranek).
    Contiguous { order: usize },
    /// Fyzicky rozsah mimo RAM (registry zarizeni).
    Physical,
}

pub struct MemoryObject {
    core: ObjectCore,
    size: u64,
    pages: Vec<PhysAddr>,
    kind: Kind,
    /// Domena IOMMU, ve ktere je pamet namapovana pro DMA zarizeni
    /// (odmapuje se pred vracenim stranek alokatoru).
    dma: Option<Arc<crate::arch::iommu::Domain>>,
}

impl MemoryObject {
    /// Novy objekt o velikosti `size` (zaokrouhleno nahoru na stranky),
    /// vyplneny nulami.
    pub fn new(size: u64) -> Result<Arc<Self>, Error> {
        if size == 0 || size > abi::vm::VMO_MAX_SIZE {
            return Err(Error::InvalidArgs);
        }
        let count = size.div_ceil(PAGE) as usize;
        let mut pages = Vec::new();
        pages
            .try_reserve_exact(count)
            .map_err(|_| Error::NoMemory)?;
        let mut vmo = Self {
            core: ObjectCore::new(Signals::NONE),
            size: count as u64 * PAGE,
            pages,
            kind: Kind::Pages,
            dma: None,
        };
        for _ in 0..count {
            // Pri neuspechu se uz alokovane stranky uvolni v Drop.
            vmo.pages
                .push(phys::alloc_user_frame_zeroed().ok_or(Error::NoMemory)?);
        }
        Ok(Arc::new(vmo))
    }

    /// Fyzicky souvisly objekt pro DMA (max. 4 MiB), vyplneny nulami; s
    /// domenou IOMMU ho zarizeni smi cist i zapisovat (identicky).
    pub fn new_contiguous_dma(
        size: u64,
        domain: Option<Arc<crate::arch::iommu::Domain>>,
    ) -> Result<Arc<Self>, Error> {
        let vmo = Self::new_contiguous(size)?;
        let Some(domain) = domain else { return Ok(vmo) };
        let mut vmo = Arc::try_unwrap(vmo).map_err(|_| Error::BadState)?;
        domain.map(vmo.pages[0].as_u64(), vmo.allocated_bytes() / PAGE)?;
        vmo.dma = Some(domain);
        Ok(Arc::new(vmo))
    }

    /// Fyzicky souvisly objekt pro DMA (max. 4 MiB), vyplneny nulami.
    pub fn new_contiguous(size: u64) -> Result<Arc<Self>, Error> {
        if size == 0 || size > (PAGE << phys::MAX_ORDER) {
            return Err(Error::InvalidArgs);
        }
        let count = size.div_ceil(PAGE);
        let order = count.next_power_of_two().trailing_zeros() as usize;
        let base = phys::alloc_user_pages(order).ok_or(Error::NoMemory)?;
        // SAFETY: prave alokovany blok 2^order stranek v prime mape.
        unsafe {
            base.to_virt()
                .as_mut_ptr::<u8>()
                .write_bytes(0, (PAGE << order) as usize)
        };
        let pages = (0..count)
            .map(|i| PhysAddr::new(base.as_u64() + i * PAGE))
            .collect();
        Ok(Arc::new(Self {
            core: ObjectCore::new(Signals::NONE),
            size: count * PAGE,
            pages,
            kind: Kind::Contiguous { order },
            dma: None,
        }))
    }

    /// Okno na fyzicky rozsah registru zarizeni. Volajici ruci (pres
    /// `platform::is_device_memory`), ze rozsah nelezi v RAM.
    pub fn new_physical(phys_start: u64, size: u64) -> Result<Arc<Self>, Error> {
        if size == 0 || !phys_start.is_multiple_of(PAGE) || size > abi::vm::VMO_MAX_SIZE {
            return Err(Error::InvalidArgs);
        }
        let count = size.div_ceil(PAGE);
        let pages = (0..count)
            .map(|i| PhysAddr::new(phys_start + i * PAGE))
            .collect();
        Ok(Arc::new(Self {
            core: ObjectCore::new(Signals::NONE),
            size: count * PAGE,
            pages,
            kind: Kind::Physical,
            dma: None,
        }))
    }

    /// Kolik fyzicke pameti objekt skutecne drzi (souvisly objekt zabira
    /// cely blok 2^order stranek).
    pub fn allocated_bytes(&self) -> u64 {
        match self.kind {
            Kind::Contiguous { order } => PAGE << order,
            _ => self.size,
        }
    }

    pub fn size(&self) -> u64 {
        self.size
    }

    /// Mapovat bez cache (registry zarizeni)?
    pub fn is_uncached(&self) -> bool {
        self.kind == Kind::Physical
    }

    /// Fyzicka stranka na offsetu `offset` (zarovnanem na stranku).
    pub fn page(&self, offset: u64) -> Option<PhysAddr> {
        self.pages.get((offset / PAGE) as usize).copied()
    }

    /// Registry zarizeni nejsou v prime mape a cteni by mohlo mit vedlejsi
    /// efekty: jde jen mapovat, ne cist a zapisovat pres jadro.
    fn check_access(&self) -> Result<(), Error> {
        if self.kind == Kind::Physical {
            Err(Error::NotSupported)
        } else {
            Ok(())
        }
    }

    fn check_range(&self, offset: u64, len: u64) -> Result<(), Error> {
        match offset.checked_add(len) {
            Some(end) if end <= self.size => Ok(()),
            _ => Err(Error::OutOfRange),
        }
    }

    /// Projde rozsah po kouscich v ramci stranek: `f(ukazatel do prime mapy,
    /// offset v bufferu, delka)`.
    fn for_each_chunk(&self, offset: u64, len: usize, mut f: impl FnMut(*mut u8, usize, usize)) {
        let mut done = 0;
        while done < len {
            let pos = offset + done as u64;
            let in_page = (pos % PAGE) as usize;
            let n = (PAGE as usize - in_page).min(len - done);
            let page = self.pages[(pos / PAGE) as usize];
            let ptr = page.to_virt().as_mut_ptr::<u8>().wrapping_add(in_page);
            f(ptr, done, n);
            done += n;
        }
    }

    pub fn read(&self, offset: u64, buf: &mut [u8]) -> Result<(), Error> {
        self.check_access()?;
        self.check_range(offset, buf.len() as u64)?;
        self.for_each_chunk(offset, buf.len(), |ptr, at, n| {
            // SAFETY: `ptr..ptr+n` lezi v jedne strance tohoto objektu (prima
            // mapa); buffer je mimo pamet objektu. Soubezny zapis jineho
            // vlakna muze dat smichana data, ale ne nedefinovane chovani
            // jadra (kopirujeme bajty).
            unsafe { core::ptr::copy_nonoverlapping(ptr, buf.as_mut_ptr().add(at), n) }
        });
        Ok(())
    }

    pub fn write(&self, offset: u64, data: &[u8]) -> Result<(), Error> {
        self.check_access()?;
        self.check_range(offset, data.len() as u64)?;
        self.for_each_chunk(offset, data.len(), |ptr, at, n| {
            // SAFETY: viz `read`.
            unsafe { core::ptr::copy_nonoverlapping(data.as_ptr().add(at), ptr, n) }
        });
        Ok(())
    }
}

impl Drop for MemoryObject {
    fn drop(&mut self) {
        // Zarizeni nejdriv ztrati pristup (IOMMU), az pak se pamet vrati.
        if let Some(d) = self.dma.take() {
            d.unmap(self.pages[0].as_u64(), self.allocated_bytes() / PAGE);
        }
        // Objekt uz nikdo nema namapovany (mapovani drzi Arc), takze stranky
        // smi zpet do alokatoru. Fyzicke okno alokatoru nepatri.
        match self.kind {
            Kind::Pages => {
                for &page in &self.pages {
                    phys::free_pages(page, 0);
                }
            }
            Kind::Contiguous { order } => phys::free_pages(self.pages[0], order),
            Kind::Physical => {}
        }
    }
}

impl KernelObject for MemoryObject {
    fn core(&self) -> &ObjectCore {
        &self.core
    }

    fn object_type(&self) -> ObjectType {
        ObjectType::MemoryObject
    }

    fn default_rights(&self) -> Rights {
        Rights::BASIC | Rights::READ | Rights::WRITE | Rights::MAP | Rights::EXECUTE
    }
}
