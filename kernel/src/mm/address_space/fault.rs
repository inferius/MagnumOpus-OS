//! Vypadek stranky v userspace: doplneni stranek na pozadani.
//!
//! Mapovani namapuje jen existujici stranky objektu pameti. Pristup na
//! ostatni vyvola vypadek a ten:
//! - pri cteni nebo spusteni namapuje existujici ramec, nebo sdilenou
//!   nulovou stranku jen pro cteni (stranka objektu dal nevznika),
//! - pri zapisu stranku objektu vytvori (`MemoryObject::commit_page_fast`)
//!   a namapuje ji zapisovatelne; byla-li tam nulova stranka, nejdriv ji
//!   odmapuje (break-before-make: na aarch64 `unmap_4k` zneplatni TLB vsech
//!   CPU hned, na x86_64 dobehne shootdown pred navratem do userspace),
//! - pri zapisu do sdileneho ramce (kopie pri zapisu) pusti zamek, zkopiruje
//!   ramec (`MemoryObject::writable_page`) a instrukci zopakuje; dalsi
//!   vypadek uz namapuje vlastni ramec. Sdileny ramec se mapuje jen pro
//!   cteni.
//!
//! - sleduje-li objekt zapis (`VMO_DIRTY`, `object/vmo/dirty.rs`), cista
//!   stranka se mapuje jen pro cteni a zapis do ni ji oznaci a namapuje
//!   pro zapis (i kdyz uz byla namapovana pro cteni),
//!
//! Vse se overuje znovu pod zamkem prostoru, takze soubeh dvou vlaken na
//! stejne strance, s `VM_UNMAP` nebo s koncem procesu je bezpecny: kdo
//! prijde druhy, najde stranku uz namapovanou (planny vypadek) nebo
//! mapovani pryc (chyba). Instrukce se po navratu provede znovu.

use alloc::sync::Arc;

use abi::vm::{MAP_EXECUTE, MAP_READ, MAP_WRITE};

use super::{AddressSpace, page_flags};
use crate::arch::paging::{MapError, PAGE_SIZE, PageSize, PageTableOps};
use crate::error::Error;
use crate::mm::{VirtAddr, tlb, zero_frame};

/// Druh pristupu, ktery vypadek vyvolal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Access {
    Read,
    Write,
    Exec,
}

impl AddressSpace {
    /// Doplni stranku na adrese `addr` po vypadku s pristupem `access`.
    /// `Ok` = instrukci zopakovat; chyba = pristup neni povoleny (adresa
    /// mimo mapovani, chybejici pravo) nebo dosla pamet.
    pub fn resolve_fault(&self, addr: u64, access: Access) -> Result<(), Error> {
        let va = VirtAddr::new(addr & !(PAGE_SIZE - 1));
        let mut inner = self.inner.lock();
        let (start, m) = inner.mapping_at(addr).ok_or(Error::BadAddress)?;
        // Rezervace (i ochranna) nema pamet: pristup je chyba procesu.
        let Some(vmo) = &m.vmo else {
            return Err(Error::AccessDenied);
        };
        let allowed = match access {
            Access::Read => m.flags & MAP_READ != 0,
            Access::Write => m.flags & MAP_WRITE != 0,
            Access::Exec => m.flags & MAP_EXECUTE != 0,
        };
        if !allowed {
            return Err(Error::AccessDenied);
        }
        let vmo = Arc::clone(vmo);
        let flags = m.flags;
        let off = m.vmo_offset + (va.as_u64() - start);
        // Za koncem zvetsitelneho objektu (mapovani muze sahat az do
        // kapacity): vyjimka s duvodem `BEYOND_END` (C: `SIGBUS`).
        if !vmo.in_size(off) {
            return Err(Error::OutOfRange);
        }
        let table = inner.table.as_mut().ok_or(Error::BadState)?;
        let current = table.translate(va);

        if access != Access::Write {
            if current.is_some() {
                // Jine vlakno stranku mezitim namapovalo.
                return Ok(());
            }
            // Poradi vuci vzniku stranky: nejdriv priznak, pak kontrola
            // (viz `MemoryObject::commit_page_fast`).
            let (frame, writable) = match vmo.page_state(off) {
                Some(state) => state,
                None => {
                    vmo.note_zero_mapped();
                    vmo.page_state(off).unwrap_or((zero_frame(), false))
                }
            };
            // Sledovany zapis: cista stranka jen pro cteni (prvni zapis =
            // vypadek, ktery ji oznaci).
            let writable = writable && !vmo.tracked_clean(off);
            return map_one(table, va, frame, page_flags(&vmo, flags, writable));
        }

        let Some((frame, fresh)) = vmo.commit_page_fast(off)? else {
            // Sdileny ramec: kopie bez zamku prostoru, pak znovu.
            drop(inner);
            vmo.writable_page(off)?;
            return Ok(());
        };
        // Sledovany zapis: oznacit pred namapovanim pro zapis (`dirty.rs`).
        let tracked = vmo.tracking();
        vmo.mark_dirty(off, PAGE_SIZE);
        let mut flush = false;
        match current {
            // Cista sledovana stranka je namapovana jen pro cteni: znovu
            // pro zapis (break-before-make).
            Some(p) if p == frame && tracked => {
                table.unmap_4k(va);
                flush = true;
                map_one(table, va, frame, page_flags(&vmo, flags, true))?;
            }
            Some(p) if p == frame => {} // planny vypadek, uz je namapovana
            Some(_) => {
                // Nulova stranka -> vlastni ramec: break-before-make.
                table.unmap_4k(va);
                flush = true;
                map_one(table, va, frame, page_flags(&vmo, flags, true))?;
            }
            None => map_one(table, va, frame, page_flags(&vmo, flags, true))?,
        }
        drop(inner);
        if flush {
            // Ostatni vlakna procesu mohla mit nulovou stranku v TLB; dobehne
            // pred navratem tohoto vlakna, tedy pred jeho zapisem.
            tlb::shootdown();
        }
        if fresh {
            // Stejnou stranku mohla jina mapovani (i v jinych procesech)
            // cist jako nulovou: odmapovat, at vidi zapsana data.
            vmo.forget_zero(off, PAGE_SIZE);
        }
        Ok(())
    }
}

fn map_one(
    table: &mut crate::arch::paging::PageTable,
    va: VirtAddr,
    frame: crate::mm::PhysAddr,
    flags: crate::arch::paging::MapFlags,
) -> Result<(), Error> {
    match table.map(va, frame, PageSize::Size4K, flags) {
        Ok(()) | Err(MapError::AlreadyMapped) => Ok(()),
        Err(_) => Err(Error::NoMemory),
    }
}
