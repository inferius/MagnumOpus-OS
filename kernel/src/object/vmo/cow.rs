//! Kopie pri zapisu: klon objektu pameti (`VMO_CLONE`) a rozdeleni
//! sdileneho ramce pri zapisu.
//!
//! **Sdileni:** klon dostane tytez ramce jako zdroj; obe polozky maji
//! priznak `SHARED` a ramec v `phys::shares` dalsiho vlastnika. Zapis
//! vlastnika sdileneho ramce:
//! 1. odmapuje ramec ve vsech svych mapovanich (zadny zapis pres stary
//!    zaznam TLB uz nedopadne do sdileneho ramce),
//! 2. pod `cow_lock` ramec zkopiruje a vzda se sveho dilu
//!    (`phys::try_unshare`), nebo - je-li uz jedinym vlastnikem - si ramec
//!    necha bez kopie,
//! 3. odmapuje stary ramec ostatnim svym mapovanim (cteni mezitim).
//!
//! Do ramce s dalsimi vlastniky nikdo nezapisuje (zapis vzdy jde pres krok
//! 2), takze kopie je konzistentni.
//!
//! **Slib:** klon slibi celou svou velikost; zdroj za kazdy nove sdileny
//! ramec slibi jednu stranku navic (sam muze zapisovat jako prvni). Kdo
//! si nakonec ramec necha (posledni vlastnik), slib za nej vrati. Kopie
//! tak nikdy nenarazi na nedostatek pameti.
//!
//! **Piny:** pripnuty ramec (DMA) se nikdy nesdili - klon dostane kopii
//! hned.

use alloc::sync::Arc;

use core::sync::atomic::{AtomicU64, Ordering};

use super::{ENTRY_FLAGS, Kind, MemoryObject, PAGE, PINNED, PRESENT, SHARED, charge, page_list};
use crate::error::Error;
use crate::mm::{PhysAddr, phys};
use crate::object::job::Job;

/// Zkopiruje obsah ramce `from` do `to` (prima mapa).
fn copy_frame(from: PhysAddr, to: PhysAddr) {
    // SAFETY: oba ramce jsou cele v prime mape a ruzne; do `from` nikdo
    // nezapisuje (sdileny ramec, viz modul), `to` je prave alokovany.
    unsafe {
        core::ptr::copy_nonoverlapping(
            from.to_virt().as_mut_ptr::<u8>(),
            to.to_virt().as_mut_ptr::<u8>(),
            PAGE as usize,
        )
    };
}

impl MemoryObject {
    /// Ramec pro zapis na offsetu `offset`: dosud nevznikla stranka vznikne,
    /// sdileny ramec se zkopiruje (nebo prevezme, kdyz uz nema dalsi
    /// vlastniky). Ostatnim mapovanim objektu odmapuje nahrazeny ramec
    /// (nulovou stranku nebo sdileny ramec). Vraci ramec a nahrazeny ramec.
    /// Volat bez zamku adresnich prostoru.
    pub fn writable_page(&self, offset: u64) -> Result<(PhysAddr, Option<PhysAddr>), Error> {
        let page = offset & !(PAGE - 1);
        if let Some((frame, fresh)) = self.commit_page_fast(page)? {
            if fresh {
                self.forget_zero(page, PAGE);
                return Ok((frame, Some(crate::mm::zero_frame())));
            }
            return Ok((frame, None));
        }
        let slot = &self.pages[(page / PAGE) as usize];
        loop {
            let cur = slot.load(Ordering::SeqCst);
            if cur & SHARED == 0 {
                // Mezitim vyreseno jinym vlaknem.
                return Ok((PhysAddr::new(cur & !ENTRY_FLAGS), None));
            }
            let old = PhysAddr::new(cur & !ENTRY_FLAGS);
            // 1. Zadny zapisovatelny zaznam (ani stary v TLB) na sdileny ramec.
            self.forget_frame(page, PAGE, old);
            // 2. Kopie nebo prevzeti pod zamkem.
            let new = {
                let _g = self.cow_lock.lock();
                if slot.load(Ordering::SeqCst) != cur {
                    continue;
                }
                let mut copied = None;
                if phys::shares(old) != 0 {
                    let new = phys::alloc_committed_zeroed().ok_or(Error::NoMemory)?;
                    copy_frame(old, new);
                    if phys::try_unshare(old) {
                        copied = Some(new);
                    } else {
                        // Ostatni vlastnici mezitim odesli.
                        phys::free_committed(new);
                    }
                }
                match copied {
                    Some(new) => slot.store(new.as_u64() | PRESENT, Ordering::SeqCst),
                    None => {
                        // Posledni vlastnik: ramec si necha, slib za kopii
                        // vrati.
                        slot.store(cur & !SHARED, Ordering::SeqCst);
                        phys::uncommit(1);
                    }
                }
                copied
            };
            self.unbacked.fetch_sub(1, Ordering::AcqRel);
            return match new {
                Some(new) => {
                    // 3. Cteni mezitim mohla namapovat stary ramec.
                    self.forget_frame(page, PAGE, old);
                    Ok((new, Some(old)))
                }
                None => Ok((old, None)),
            };
        }
    }

    /// Klon `len` bajtu od `offset` (zarovnane na stranku) s kopii pri
    /// zapisu, strzeny z rozpoctu `job`. Jen bezne objekty (ne DMA, souvisle
    /// ani fyzicke). Zapisy zdroje soubezne s klonovanim muze klon videt.
    pub fn clone_cow(self: &Arc<Self>, offset: u64, len: u64, job: Option<&Arc<Job>>) -> Result<Arc<Self>, Error> {
        if !matches!(self.kind, Kind::Pages) || self.for_dma {
            return Err(Error::NotSupported);
        }
        if len == 0 || offset % PAGE != 0 || len % PAGE != 0 {
            return Err(Error::InvalidArgs);
        }
        match offset.checked_add(len) {
            Some(end) if end <= self.size => {}
            _ => return Err(Error::OutOfRange),
        }
        let count = len / PAGE;
        let first = (offset / PAGE) as usize;
        let mut charges = charge(job, count, count * PAGE)?;
        let mut pages = page_list(count)?;
        let mut eager = 0u64;
        let mut failed = false;
        {
            let _g = self.cow_lock.lock();
            // Kolik ramcu zdroje zacne sdilet poprve (zdroj za ne slibi
            // stranku navic) - pred zmenami, aby slib sel vratit celym.
            let newly_shared = (0..count as usize)
                .filter(|&i| {
                    let v = self.pages[first + i].load(Ordering::SeqCst);
                    v != 0 && v & (SHARED | PINNED) == 0
                })
                .count() as u64;
            if !phys::commit(count + newly_shared) {
                return Err(Error::NoMemory);
            }
            if let Err(e) = charges[1].grow(newly_shared * PAGE) {
                phys::uncommit(count + newly_shared);
                return Err(e);
            }
            let mut extra = 0u64;
            for i in 0..count as usize {
                let src = &self.pages[first + i];
                let v = src.load(Ordering::SeqCst);
                let frame = PhysAddr::new(v & !ENTRY_FLAGS);
                let entry = if v == 0 || failed {
                    0
                } else if v & PINNED == 0 && phys::share(frame) {
                    if src.fetch_or(SHARED, Ordering::SeqCst) & SHARED == 0 {
                        extra += 1;
                    }
                    frame.as_u64() | PRESENT | SHARED
                } else {
                    // Pripnuty (zarizeni do nej pise) nebo plne pocitadlo:
                    // klon dostane kopii hned (ze sveho slibu).
                    match phys::alloc_committed_zeroed() {
                        Some(new) => {
                            copy_frame(frame, new);
                            eager += 1;
                            new.as_u64() | PRESENT
                        }
                        // Jen kdyz jadro vycerpalo rezervu: zbytek prazdny,
                        // objekt se hned zrusi (Drop vrati sdileni i slib).
                        None => {
                            failed = true;
                            0
                        }
                    }
                };
                pages.push(AtomicU64::new(entry))?;
            }
            // Nove sdilene ramce zdroje: slib navic patri zdroji.
            self.unbacked.fetch_add(extra, Ordering::AcqRel);
            phys::uncommit(newly_shared - extra);
        }
        let clone = Arc::new(Self::with(len, pages, Kind::Pages, count - eager, charges));
        if failed {
            drop(clone);
            return Err(Error::NoMemory);
        }
        // Zapisovatelne zaznamy zdroje na nyni sdilene ramce pryc: dalsi
        // zapis zdroje projde kopii.
        let mut flush = false;
        for p in self.mapper_processes() {
            flush |= p.address_space().drop_frame_ptes(self, offset, len, None);
        }
        if flush {
            crate::mm::tlb::shootdown();
        }
        Ok(clone)
    }
}
