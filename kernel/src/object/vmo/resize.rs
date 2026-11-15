//! Zvetsitelny objekt pameti (`VMO_RESIZABLE`, `VMO_SET_SIZE`; C `memfd`
//! a `ftruncate`, docs/cs/sprava-pameti.md).
//!
//! Seznam stranek ma od vytvoreni delku kapacity (polozky za koncem jsou
//! 0), takze ctenari bez zamku nikdy nevidi presun seznamu. Slib (`phys`)
//! a rozpocet uzivatelske pameti jobu plati jen za aktualni velikost
//! (`len`): zvetseni je hned slibi (bez OOM pri vypadku, jako vytvoreni
//! objektu), zmenseni vrati.
//!
//! Zmenseni (poradi kvuli soubeznym vypadkum a mapovanim):
//! 1. zverejnit novou velikost (SeqCst): dalsi vypadek za koncem selze,
//!    vypadek, ktery uz stranku vytvoril, ji po kontrole vrati sam
//!    (`commit_page_fast`);
//! 2. odmapovat rozsah u vsech mapperu (`drop_frame_ptes`, mapovani samo
//!    zustane - dalsi pristup = vypadek za koncem = vyjimka) a vyprazdnit
//!    TLB vsech CPU;
//! 3. teprve pak uvolnit ramce a slib stranek za koncem.
//!
//! Zmeny velikosti jednoho objektu jdou za sebou (`resizing`); zamek se
//! nedrzi pres shootdown (jen priznak a vzdani se CPU).

use core::sync::atomic::Ordering;

use super::{ENTRY_FLAGS, Kind, MemoryObject, PAGE, PINNED, SHARED};
use crate::error::Error;
use crate::mm::{PhysAddr, phys};

/// Zamek zmen velikosti: priznak `resizing` (drzi se i pres shootdown,
/// proto ne `SpinLock`).
struct Resizing<'a>(&'a MemoryObject);

impl<'a> Resizing<'a> {
    fn take(o: &'a MemoryObject) -> Self {
        while o.resizing.compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire).is_err() {
            crate::sched::yield_now();
        }
        Resizing(o)
    }
}

impl Drop for Resizing<'_> {
    fn drop(&mut self) {
        self.0.resizing.store(false, Ordering::Release);
    }
}

impl MemoryObject {
    /// Zmeni velikost zvetsitelneho objektu na `new` bajtu (nejvys kapacita).
    pub fn set_size(&self, new: u64) -> Result<(), Error> {
        if !self.resizable || !matches!(self.kind, Kind::Pages) {
            return Err(Error::NotSupported);
        }
        if new > self.size {
            return Err(Error::OutOfRange);
        }
        if self.is_sealed() {
            return Err(Error::AccessDenied);
        }
        let _g = Resizing::take(self);
        let old = self.len.load(Ordering::SeqCst);
        let (old_pages, new_pages) = (old.div_ceil(PAGE), new.div_ceil(PAGE));
        if new_pages > old_pages {
            return self.grow(new, new_pages - old_pages);
        }
        if new_pages < old_pages {
            return self.shrink(new, new_pages, old_pages);
        }
        // Stejny pocet stranek: jen bajtova velikost. Pri zmenseni v posledni
        // strance cast za koncem uz neni objekt (cteni jadrem ji neprecte);
        // obsah stranky zustava jako u Linuxu az do dalsiho zvetseni.
        self.len.store(new, Ordering::SeqCst);
        Ok(())
    }

    fn grow(&self, new: u64, add: u64) -> Result<(), Error> {
        if !phys::user_room(add) {
            return Err(Error::NoMemory);
        }
        self.resize_charge.lock().grow(add * PAGE)?;
        if !phys::commit(add) {
            self.resize_charge.lock().shrink(add * PAGE);
            return Err(Error::NoMemory);
        }
        self.unbacked.fetch_add(add, Ordering::AcqRel);
        self.len.store(new, Ordering::SeqCst);
        Ok(())
    }

    fn shrink(&self, new: u64, new_pages: u64, old_pages: u64) -> Result<(), Error> {
        let (from, to) = (new_pages * PAGE, old_pages * PAGE);
        // Pripnuty ramec (ovladac zna jeho adresu) nesmi zmizet.
        for i in new_pages..old_pages {
            if self.pages.get(i as usize).is_some_and(|s| s.load(Ordering::SeqCst) & PINNED != 0) {
                return Err(Error::BadState);
            }
        }
        // 1. Konec zverejnit.
        self.len.store(new, Ordering::SeqCst);
        // 2. Odmapovat u vsech mapperu a vyprazdnit TLB.
        let mut flush = false;
        for p in self.mapper_processes() {
            flush |= p.address_space().drop_frame_ptes(self, from, to - from, None);
        }
        if flush {
            crate::mm::tlb::shootdown();
        }
        // 3. Ramce a slib stranek za koncem.
        let _cow = self.cow_lock.lock();
        let mut unpromised = 0;
        for i in new_pages..old_pages {
            let Some(slot) = self.pages.get(i as usize) else { break };
            let v = slot.swap(0, Ordering::SeqCst);
            if v == 0 {
                // Slibena stranka bez ramce.
                unpromised += 1;
                continue;
            }
            let frame = PhysAddr::new(v & !ENTRY_FLAGS);
            if v & SHARED != 0 {
                // Sdileny ramec (klon) je soucasti `unbacked`: slib zpet,
                // ramec drzi dal ostatni vlastnici.
                unpromised += 1;
                if !phys::try_unshare(frame) {
                    phys::free_pages(frame, 0);
                }
            } else {
                phys::free_pages(frame, 0);
            }
        }
        drop(_cow);
        if unpromised > 0 {
            self.unbacked.fetch_sub(unpromised, Ordering::AcqRel);
            phys::uncommit(unpromised);
        }
        self.resize_charge.lock().shrink((old_pages - new_pages) * PAGE);
        Ok(())
    }
}
