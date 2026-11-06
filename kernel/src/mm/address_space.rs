//! Adresni prostor procesu: vlastni spodni polovina, sdilene jadro.
//!
//! Spodni polovina obsahuje jen mapovani objektu pameti ([`MemoryObject`]).
//! Jadro do pameti procesu nikdy nepristupuje pres adresy userspace: pro
//! kopirovani dat (argumenty syscallu) prelozi adresu podle seznamu
//! mapovani na fyzickou stranku objektu a kopiruje pres primou mapu. Diky
//! tomu nemuze vzniknout page fault v jadre a SMAP muze byt trvale zapnuty.

use alloc::collections::BTreeMap;
use alloc::sync::Arc;
use alloc::vec::Vec;

use abi::vm::{
    MAP_EXECUTE, MAP_FIXED, MAP_READ, MAP_WRITE, PAGE_SIZE, USER_ALLOC_BASE, USER_MAX, USER_MIN,
};

use super::{PhysAddr, VirtAddr, new_address_space, tlb};
use crate::arch::paging::{MapFlags, PageSize, PageTable};
use crate::error::Error;
use crate::object::vmo::MemoryObject;
use crate::sync::SpinLock;

struct Mapping {
    len: u64,
    vmo: Arc<MemoryObject>,
    vmo_offset: u64,
    flags: u32,
}

struct Inner {
    table: Option<PageTable>,
    /// Mapovani podle pocatecni adresy (neprekryvaji se).
    mappings: BTreeMap<u64, Mapping>,
}

pub struct AddressSpace {
    root: u64,
    inner: SpinLock<Inner>,
}

fn page_aligned(v: u64) -> bool {
    v.is_multiple_of(PAGE_SIZE)
}

impl Inner {
    fn overlaps(&self, start: u64, len: u64) -> bool {
        let end = start + len;
        // Mapovani zacinajici pred `end`; staci posledni z nich.
        self.mappings
            .range(..end)
            .next_back()
            .is_some_and(|(&s, m)| s + m.len > start)
    }

    fn find_free(&self, len: u64, hint: u64) -> Option<u64> {
        let mut candidate = if hint >= USER_MIN && page_aligned(hint) {
            hint
        } else {
            USER_ALLOC_BASE
        };
        loop {
            let end = candidate.checked_add(len)?;
            if end > USER_MAX {
                return None;
            }
            if !self.overlaps(candidate, len) {
                return Some(candidate);
            }
            // Za konec prvniho prekryvajiciho mapovani.
            let (&s, m) = self.mappings.range(..end).next_back()?;
            candidate = s + m.len;
        }
    }

    /// Mapovani obsahujici adresu `addr`.
    fn mapping_at(&self, addr: u64) -> Option<(u64, &Mapping)> {
        let (&start, m) = self.mappings.range(..=addr).next_back()?;
        (addr < start + m.len).then_some((start, m))
    }
}

impl AddressSpace {
    pub fn new() -> Option<Self> {
        let table = new_address_space()?;
        Some(Self {
            root: table.root(),
            inner: SpinLock::new(Inner {
                table: Some(table),
                mappings: BTreeMap::new(),
            }),
        })
    }

    /// Fyzicka adresa korenove tabulky (pro prepnuti CR3).
    pub fn root(&self) -> u64 {
        self.root
    }

    /// Namapuje `len` bajtu objektu `vmo` od `vmo_offset`. S `MAP_FIXED` presne
    /// na `addr`, jinak na volne misto (`addr` je napoveda, 0 = kdekoli).
    pub fn map(
        &self,
        vmo: Arc<MemoryObject>,
        vmo_offset: u64,
        len: u64,
        flags: u32,
        addr: u64,
    ) -> Result<u64, Error> {
        let known = MAP_READ | MAP_WRITE | MAP_EXECUTE | MAP_FIXED;
        if flags & !known != 0 || flags & MAP_READ == 0 {
            return Err(Error::InvalidArgs);
        }
        // W^X: stranka nesmi byt zaroven zapisovatelna a spustitelna.
        if flags & MAP_WRITE != 0 && flags & MAP_EXECUTE != 0 {
            return Err(Error::InvalidArgs);
        }
        if len == 0 || !page_aligned(len) || !page_aligned(vmo_offset) {
            return Err(Error::InvalidArgs);
        }
        match vmo_offset.checked_add(len) {
            Some(end) if end <= vmo.size() => {}
            _ => return Err(Error::OutOfRange),
        }

        // Tabulky stranek pro mapovani: nejvys 1 stranka na 2 MiB + 3 vyssi
        // urovne; vznikaji na pozadani programu, takze ne z rezervy jadra.
        if !super::phys::user_room(len.div_ceil(2 << 20) + 3) {
            return Err(Error::NoMemory);
        }
        let mut inner = self.inner.lock();
        if inner.mappings.len() >= abi::limits::PROCESS_MAX_MAPPINGS {
            return Err(Error::NoResources);
        }
        let start = if flags & MAP_FIXED != 0 {
            let end = addr.checked_add(len).ok_or(Error::InvalidArgs)?;
            if !page_aligned(addr) || addr < USER_MIN || end > USER_MAX {
                return Err(Error::InvalidArgs);
            }
            if inner.overlaps(addr, len) {
                return Err(Error::BadState);
            }
            addr
        } else {
            inner.find_free(len, addr).ok_or(Error::NoResources)?
        };

        let mut map_flags = MapFlags::USER;
        if vmo.is_uncached() {
            map_flags = map_flags | MapFlags::UNCACHED;
        }
        if flags & MAP_WRITE != 0 {
            map_flags = map_flags | MapFlags::WRITE;
        }
        if flags & MAP_EXECUTE != 0 {
            map_flags = map_flags | MapFlags::EXEC;
        }
        let table = inner.table.as_mut().expect("adresni prostor je zruseny");
        for off in (0..len).step_by(PAGE_SIZE as usize) {
            let phys = vmo.page(vmo_offset + off).expect("rozsah overen vyse");
            if table
                .map(
                    VirtAddr::new(start + off),
                    phys,
                    PageSize::Size4K,
                    map_flags,
                )
                .is_err()
            {
                // Nedostatek pameti na tabulky: vratit, co uz je namapovane.
                for done in (0..off).step_by(PAGE_SIZE as usize) {
                    table.unmap_4k(VirtAddr::new(start + done));
                }
                drop(inner);
                tlb::shootdown();
                return Err(Error::NoMemory);
            }
        }
        inner.mappings.insert(
            start,
            Mapping {
                len,
                vmo,
                vmo_offset,
                flags,
            },
        );
        Ok(start)
    }

    /// Zrusi mapovani. Rozsah musi presne pokryvat cela mapovani
    /// (castecne odmapovani zatim neni podporovane).
    pub fn unmap(&self, addr: u64, len: u64) -> Result<(), Error> {
        if len == 0 || !page_aligned(addr) || !page_aligned(len) {
            return Err(Error::InvalidArgs);
        }
        let end = addr.checked_add(len).ok_or(Error::InvalidArgs)?;
        let mut inner = self.inner.lock();
        // Overit, ze rozsah tvori souvislou radu celych mapovani.
        let mut pos = addr;
        let starts: Vec<u64> = inner.mappings.range(addr..end).map(|(&s, _)| s).collect();
        for &s in &starts {
            if s != pos {
                return Err(Error::NotFound);
            }
            pos += inner.mappings[&s].len;
        }
        if pos != end || inner.mapping_at(addr).is_some_and(|(s, _)| s != addr) {
            return Err(Error::NotFound);
        }
        let mut removed = Vec::new();
        for s in starts {
            let m = inner.mappings.remove(&s).unwrap();
            let table = inner.table.as_mut().expect("adresni prostor je zruseny");
            for off in (0..m.len).step_by(PAGE_SIZE as usize) {
                table.unmap_4k(VirtAddr::new(s + off));
            }
            removed.push(m);
        }
        drop(inner);
        // Stranky (v `removed`) se smi uvolnit az kdyz je zadne CPU nema v TLB.
        tlb::shootdown();
        drop(removed);
        Ok(())
    }

    /// Prelozi rozsah userspace na kousky fyzicke pameti a zavola na ne `f`
    /// (adresa v prime mape, offset v rozsahu, delka). Overi, ze je cely
    /// rozsah namapovany s pozadovanymi pravy; jinak `BadAddress`.
    fn with_user_range(
        &self,
        addr: u64,
        len: usize,
        write: bool,
        mut f: impl FnMut(*mut u8, usize, usize),
    ) -> Result<(), Error> {
        if len == 0 {
            return Ok(());
        }
        let end = addr.checked_add(len as u64).ok_or(Error::BadAddress)?;
        if addr < USER_MIN || end > USER_MAX {
            return Err(Error::BadAddress);
        }
        let inner = self.inner.lock();
        // Nejdriv overit cely rozsah, at se pri chybe nic neprovede napul.
        let mut pos = addr;
        while pos < end {
            let (start, m) = inner.mapping_at(pos).ok_or(Error::BadAddress)?;
            // Registry zarizeni nejsou v prime mape: jako buffer syscallu
            // je nelze pouzit.
            if (write && m.flags & MAP_WRITE == 0) || m.vmo.is_uncached() {
                return Err(Error::BadAddress);
            }
            pos = start + m.len;
        }
        let mut done = 0usize;
        while done < len {
            let pos = addr + done as u64;
            let (start, m) = inner.mapping_at(pos).expect("overeno vyse");
            let in_page = (pos % PAGE_SIZE) as usize;
            let n = (PAGE_SIZE as usize - in_page).min(len - done);
            let page: PhysAddr = m
                .vmo
                .page(m.vmo_offset + (pos - start))
                .expect("overeno vyse");
            f(
                page.to_virt().as_mut_ptr::<u8>().wrapping_add(in_page),
                done,
                n,
            );
            done += n;
        }
        Ok(())
    }

    /// Zkopiruje data z pameti procesu.
    pub fn copy_from_user(&self, addr: u64, buf: &mut [u8]) -> Result<(), Error> {
        let dst = buf.as_mut_ptr();
        self.with_user_range(addr, buf.len(), false, |src, at, n| {
            // SAFETY: `src..src+n` je namapovana stranka objektu pameti (prima
            // mapa), cil je buffer jadra; zamek adresniho prostoru brani
            // uvolneni stranek behem kopirovani.
            unsafe { core::ptr::copy_nonoverlapping(src, dst.add(at), n) }
        })
    }

    /// Zkopiruje data do pameti procesu (jen do zapisovatelnych mapovani).
    pub fn copy_to_user(&self, addr: u64, data: &[u8]) -> Result<(), Error> {
        let src = data.as_ptr();
        self.with_user_range(addr, data.len(), true, |dst, at, n| {
            // SAFETY: viz `copy_from_user`.
            unsafe { core::ptr::copy_nonoverlapping(src.add(at), dst, n) }
        })
    }

    /// Pocet mapovani (pro testy a ladeni).
    pub fn mapping_count(&self) -> usize {
        self.inner.lock().mappings.len()
    }

    /// Prelozi adresu podle tabulek stranek (pro testy).
    pub fn translate(&self, virt: VirtAddr) -> Option<PhysAddr> {
        self.inner.lock().table.as_ref()?.translate(virt)
    }
}

impl Drop for AddressSpace {
    fn drop(&mut self) {
        let inner = self.inner.get_mut();
        if let Some(table) = inner.table.take() {
            // SAFETY: AddressSpace se rusi az s posledni referenci na proces;
            // jeho vlakna uz nebezi a planovac pri prepnuti na jine vlakno
            // vzdy prepne CR3, takze tento adresni prostor neni aktivni na
            // zadnem CPU. TLB zaznamy byly zahozeny prepnutim CR3.
            unsafe { table.destroy() };
        }
        // Mapovani (Arc na objekty pameti) se uvolni s `inner`.
    }
}
