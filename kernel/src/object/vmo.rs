//! Objekt pameti (memory object): stranky fyzicke pameti, ktere lze cist,
//! zapisovat a mapovat do adresnich prostoru procesu.
//!
//! Zatim se stranky alokuji a nuluji hned pri vytvoreni a objekt nemeni
//! velikost. TODO: lazy alokace pri prvnim pristupu, copy-on-write klony.

use alloc::sync::Arc;
use alloc::vec::Vec;

use super::{Error, KernelObject, ObjectCore, ObjectType, Rights, Signals};
use crate::mm::{PhysAddr, phys};

const PAGE: u64 = abi::vm::PAGE_SIZE;

pub struct MemoryObject {
    core: ObjectCore,
    size: u64,
    pages: Vec<PhysAddr>,
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
        };
        for _ in 0..count {
            // Pri neuspechu se uz alokovane stranky uvolni v Drop.
            vmo.pages
                .push(phys::alloc_frame_zeroed().ok_or(Error::NoMemory)?);
        }
        Ok(Arc::new(vmo))
    }

    pub fn size(&self) -> u64 {
        self.size
    }

    /// Fyzicka stranka na offsetu `offset` (zarovnanem na stranku).
    pub fn page(&self, offset: u64) -> Option<PhysAddr> {
        self.pages.get((offset / PAGE) as usize).copied()
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
        // Objekt uz nikdo nema namapovany (mapovani drzi Arc), takze stranky
        // smi zpet do alokatoru.
        for &page in &self.pages {
            phys::free_pages(page, 0);
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
