//! Vypadek stranky v userspace: doplneni stranek na pozadani.
//!
//! Mapovani namapuje jen existujici stranky objektu pameti. Pristup na
//! ostatni vyvola vypadek a ten:
//! - pri cteni nebo spusteni namapuje existujici ramec, nebo sdilenou
//!   nulovou stranku jen pro cteni (stranka objektu dal nevznika),
//! - pri zapisu stranku objektu vytvori (`MemoryObject::commit_page`)
//!   a namapuje ji zapisovatelne; byla-li tam nulova stranka, nejdriv ji
//!   odmapuje (break-before-make: na aarch64 `unmap_4k` zneplatni TLB vsech
//!   CPU hned, na x86_64 dobehne shootdown pred navratem do userspace).
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
        let table = inner.table.as_mut().ok_or(Error::BadState)?;
        let current = table.translate(va);

        if access != Access::Write {
            if current.is_some() {
                // Jine vlakno stranku mezitim namapovalo.
                return Ok(());
            }
            // Poradi vuci vzniku stranky: nejdriv priznak, pak kontrola
            // (viz `MemoryObject::commit_page`).
            let (frame, writable) = match vmo.page(off) {
                Some(frame) => (frame, true),
                None => {
                    vmo.note_zero_mapped();
                    match vmo.page(off) {
                        Some(frame) => (frame, true),
                        None => (zero_frame(), false),
                    }
                }
            };
            return map_one(table, va, frame, page_flags(&vmo, flags, writable));
        }

        let (frame, fresh) = vmo.commit_page(off)?;
        let mut flush = false;
        match current {
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
