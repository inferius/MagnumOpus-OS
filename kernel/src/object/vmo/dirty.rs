//! Sledovani zapisu do objektu pameti (`VMO_DIRTY`, docs/cs/syscally.md):
//! sdilene mapovani souboru (fsd) potrebuje vedet, ktere stranky zapsat
//! zpet do FS.
//!
//! Prvni `VMO_DIRTY` sledovani zapne. Pak plati: cista stranka se mapuje
//! jen pro cteni (pri vytvoreni mapovani i pri vypadku pro cteni), prvni
//! zapis vyvola vypadek, ten stranku oznaci jako spinavou a namapuje ji
//! pro zapis. `VMO_DIRTY` vrati spinave stranky, oznaci je jako ciste
//! a odmapuje je u vsech mapperu (dalsi zapis = znovu vypadek). Stejne na
//! x86_64 i aarch64 (bez dirty bitu PTE). Zapis bez vypadku (`VMO_WRITE`,
//! kopie jadra do pameti procesu) stranky oznaci primo.
//!
//! Poradi `take_dirty`: nejdriv vynulovat bity, pak odmapovat a vyprazdnit
//! TLB, teprve pak vratit seznam; zapis mezi vynulovanim a odmapovanim
//! skonci v ramci driv, nez volajici obsah precte, zapis po odmapovani
//! stranku oznaci znovu.

use alloc::boxed::Box;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU64, Ordering};

use super::{MemoryObject, PAGE};
use crate::error::Error;

impl MemoryObject {
    /// Sleduje se zapis (`VMO_DIRTY` aspon jednou)?
    pub fn tracking(&self) -> bool {
        self.dirty.get().is_some()
    }

    /// Stranka na `offset` se sleduje a je cista: mapovat jen pro cteni.
    pub fn tracked_clean(&self, offset: u64) -> bool {
        let Some(bits) = self.dirty.get() else { return false };
        let i = (offset / PAGE) as usize;
        bits.get(i / 64).is_some_and(|w| w.load(Ordering::SeqCst) & (1 << (i % 64)) == 0)
    }

    /// Oznaci stranky `offset..+len` jako spinave (jen pri sledovani).
    pub fn mark_dirty(&self, offset: u64, len: u64) {
        let Some(bits) = self.dirty.get() else { return };
        if len == 0 {
            return;
        }
        let first = (offset / PAGE) as usize;
        let last = ((offset + len - 1) / PAGE) as usize;
        for i in first..=last {
            if let Some(w) = bits.get(i / 64) {
                w.fetch_or(1 << (i % 64), Ordering::SeqCst);
            }
        }
    }

    /// `VMO_DIRTY`: nejvys `cap` indexu spinavych stranek (vzestupne), ty
    /// oznaci jako ciste a odmapuje je u vsech mapperu. Prvni volani zapne
    /// sledovani a vsechny existujici stranky bere jako spinave (zapis pred
    /// sledovanim nikdo nevidel).
    pub fn take_dirty(&self, cap: usize) -> Result<Vec<u64>, Error> {
        if !self.tracking() {
            let words = (self.capacity().div_ceil(PAGE) as usize).div_ceil(64);
            let mut v = Vec::new();
            v.try_reserve_exact(words).map_err(|_| Error::NoMemory)?;
            v.extend((0..words).map(|_| AtomicU64::new(0)));
            let bits: Box<[AtomicU64]> = v.into_boxed_slice();
            let bits = self.dirty.call_once(|| bits);
            for i in 0..(self.capacity().div_ceil(PAGE) as usize) {
                if self.page_state(i as u64 * PAGE).is_some() {
                    bits[i / 64].fetch_or(1 << (i % 64), Ordering::SeqCst);
                }
            }
        }
        let bits = self.dirty.get().ok_or(Error::BadState)?;
        let mut out = Vec::new();
        for (wi, w) in bits.iter().enumerate() {
            if out.len() >= cap {
                break;
            }
            let mut cur = w.load(Ordering::SeqCst);
            while cur != 0 && out.len() < cap {
                let b = cur.trailing_zeros() as usize;
                let mask = 1u64 << b;
                // Vynulovat jen tento bit (soubezny zapis muze nastavit dalsi).
                w.fetch_and(!mask, Ordering::SeqCst);
                out.try_reserve(1).map_err(|_| Error::NoMemory)?;
                out.push((wi * 64 + b) as u64);
                cur &= !mask;
            }
        }
        // Odmapovat ciste stranky u vsech mapperu (po souvislych usecich).
        let mut flush = false;
        let mappers = self.mapper_processes();
        let mut i = 0;
        while i < out.len() {
            let mut j = i;
            while j + 1 < out.len() && out[j + 1] == out[j] + 1 {
                j += 1;
            }
            let (from, len) = (out[i] * PAGE, (out[j] - out[i] + 1) * PAGE);
            for p in &mappers {
                flush |= p.address_space().drop_frame_ptes(self, from, len, None);
            }
            i = j + 1;
        }
        if flush {
            crate::mm::tlb::shootdown();
        }
        Ok(out)
    }
}
