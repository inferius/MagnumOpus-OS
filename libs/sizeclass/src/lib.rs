//! Velikostni tridy pro male alokace heapu.
//!
//! Hlavni alokator procesu (`linked_list_allocator`, first-fit) prochazi
//! pri kazde alokaci seznam volnych useku. Kdyz se heap roztrisi (velka
//! cache uzlu FS, hodne kratkodobych `Vec` a `String`), seznam ma tisice
//! polozek a kazda alokace stoji milisekundy: ve fsd po testech FS stalo
//! jedno vlozeni do B-stromu ~10 ms, radek shellu se zapisem sekundy.
//!
//! Male alokace (do 4 KiB, zarovnani do 16 B) proto jdou do trid pevne
//! velikosti: kazda trida ma vlastni seznam volnych objektu (O(1) alokace
//! i uvolneni). Prazdna trida si od hlavniho alokatoru vezme kus pameti
//! (`chunk_size`) a rozdeli ho na objekty. Do hlavniho alokatoru tak jdou
//! jen velke alokace a kusy pro tridy - malo polozek, seznam zustane
//! kratky.
//!
//! Uvolneny objekt zustava ve sve tride (pamet se hlavnimu alokatoru
//! nevraci). Rezie: zaokrouhleni na tridu (nejvys ~1,5x) a nejvys jeden
//! nevyuzity kus na tridu (16 trid, kusy 4-32 KiB).
//!
//! Knihovna je bez zamku: volajici (globalni alokator) ji drzi pod svym
//! zamkem.

#![no_std]

#[cfg(test)]
extern crate std;

use core::alloc::Layout;
use core::ptr;

/// Velikosti trid (nasobky 16: kazdy objekt je zarovnany na 16 B, kdyz je
/// kus zarovnany na 16 B).
pub const CLASSES: [usize; 16] = [16, 32, 48, 64, 96, 128, 192, 256, 384, 512, 768, 1024, 1536, 2048, 3072, 4096];
/// Nejvetsi zarovnani, ktere tridy zaruci.
pub const ALIGN: usize = 16;

/// Trida pro alokaci `layout`; `None` = jde primo do hlavniho alokatoru
/// (velka nebo s vetsim zarovnanim).
pub fn class_of(layout: Layout) -> Option<usize> {
    if layout.align() > ALIGN {
        return None;
    }
    let size = layout.size().max(1);
    CLASSES.iter().position(|&c| c >= size)
}

/// Velikost kusu, ktery si trida `c` bere od hlavniho alokatoru.
pub fn chunk_size(c: usize) -> usize {
    (CLASSES[c] * 16).clamp(4096, 32 * 1024)
}

/// Layout kusu pro tridu `c` (pro hlavni alokator).
pub fn chunk_layout(c: usize) -> Layout {
    // Velikost i zarovnani jsou konstanty, ktere vzdy plati.
    Layout::from_size_align(chunk_size(c), ALIGN).unwrap()
}

/// Seznamy volnych objektu (prvni slovo objektu = dalsi volny).
pub struct Classes {
    free: [*mut u8; CLASSES.len()],
    /// Statistika: kolik kusu tridy dostaly.
    pub chunks: usize,
}

// SAFETY: ukazatele vedou do pameti heapu procesu; pristup serializuje
// zamek volajiciho.
unsafe impl Send for Classes {}

impl Default for Classes {
    fn default() -> Self {
        Self::new()
    }
}

impl Classes {
    pub const fn new() -> Self {
        Self { free: [ptr::null_mut(); CLASSES.len()], chunks: 0 }
    }

    /// Objekt tridy `c`. Kdyz je trida prazdna, zavola `refill` s layoutem
    /// kusu (vrati pamet z hlavniho alokatoru, nebo null = nedostatek).
    ///
    /// # Safety
    /// `refill` musi vratit null nebo platnou, nikym jinym nepouzivanou
    /// pamet velikosti a zarovnani podle `chunk_layout(c)`.
    pub unsafe fn alloc(&mut self, c: usize, refill: impl FnOnce(Layout) -> *mut u8) -> *mut u8 {
        if self.free[c].is_null() {
            let chunk = refill(chunk_layout(c));
            if chunk.is_null() {
                return ptr::null_mut();
            }
            self.chunks += 1;
            let size = CLASSES[c];
            let n = chunk_size(c) / size;
            // Seznam od konce, aby prvni alokace dostala zacatek kusu.
            for i in (0..n).rev() {
                // SAFETY: objekt `i` lezi cely uvnitr kusu.
                let obj = unsafe { chunk.add(i * size) };
                // SAFETY: objekt ma aspon 16 B a je zarovnany na 16 B.
                unsafe { (obj as *mut *mut u8).write(self.free[c]) };
                self.free[c] = obj;
            }
        }
        let obj = self.free[c];
        // SAFETY: `obj` je volny objekt tridy, prvni slovo = dalsi volny.
        self.free[c] = unsafe { (obj as *mut *mut u8).read() };
        obj
    }

    /// Vrati objekt `p` do tridy `c`.
    ///
    /// # Safety
    /// `p` vratil `alloc` tehoz `Classes` pro tutez tridu a jeste nebyl
    /// uvolneny.
    pub unsafe fn dealloc(&mut self, c: usize, p: *mut u8) {
        // SAFETY: objekt patri volajicimu az do ted; prvni slovo pouzije
        // seznam.
        unsafe { (p as *mut *mut u8).write(self.free[c]) };
        self.free[c] = p;
    }
}

#[cfg(test)]
mod tests;

/// Pocatecni velikost heapu procesu (`libos` `rt_own`). Objekt pameti si
/// pri vytvoreni vezme slib za celou velikost, proto heap zacina maly.
pub const HEAP_INITIAL: u64 = 256 * 1024;
/// Nejmensi prirustek heapu.
pub const HEAP_GROW_MIN: u64 = 256 * 1024;

/// O kolik heap velikosti `size` naroste, kdyz se do nej nevejde alokace
/// `need` bajtu: aspon [`HEAP_GROW_MIN`] a aspon o polovinu dosavadni
/// velikosti (pocet rustu, mapovani a volani jadra je logaritmicky),
/// zarovnane na stranku `page`.
pub fn heap_grow(need: u64, size: u64, page: u64) -> u64 {
    need.max(HEAP_GROW_MIN).max(size / 2).next_multiple_of(page.max(1))
}
