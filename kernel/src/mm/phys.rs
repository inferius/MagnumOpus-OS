//! Alokator fyzicke pameti (buddy).
//!
//! Pamet se spravuje v blocich o velikosti 2^order stranek, order 0..=MAX_ORDER.
//! Pro kazdy order existuje:
//! - obousmerny seznam volnych bloku; uzly seznamu lezi primo ve volnych
//!   strankach (pristup pres primou mapu), takze seznamy nestoji zadnou
//!   pamet navic,
//! - bitmapa "blok je volny" pro rychle zjisteni, zda je volny buddy.
//!
//! Bitmapy pokryvaji fyzicke adresy od 0 do konce nejvyssi oblasti RAM.
//! Diry v pameti v bitmapach nikdy nejsou oznacene jako volne, takze se pres
//! ne bloky nespoji.

use boot_info::{MemoryKind, MemoryRegion};
use spin::Mutex;

use super::PhysAddr;
use crate::arch::paging::PAGE_SIZE;

pub const MAX_ORDER: usize = 10; // nejvetsi blok 2^10 stranek = 4 MiB

/// Pamet pod 1 MiB alokator nespravuje: aplikacni procesory startuji
/// v realnem modu z adresy pod 1 MiB (milnik 3 si tam vezme trampolinu).
const LOW_MEMORY_LIMIT: u64 = 0x10_0000;

const NONE: u64 = u64::MAX;

#[repr(C)]
struct FreeBlock {
    next: u64,
    prev: u64,
}

struct Buddy {
    /// Pocet stranek pokrytych bitmapami (od fyzicke adresy 0).
    pages: u64,
    bitmaps: [*mut u64; MAX_ORDER + 1],
    free_lists: [u64; MAX_ORDER + 1],
    free_pages: u64,
    total_pages: u64,
}

// SAFETY: ukazatele do bitmap a volnych bloku vlastni vyhradne alokator,
// pristup k nemu serializuje mutex.
unsafe impl Send for Buddy {}

static ALLOCATOR: Mutex<Option<Buddy>> = Mutex::new(None);

const fn block_bytes(order: usize) -> u64 {
    PAGE_SIZE << order
}

fn blocks_at(pages: u64, order: usize) -> u64 {
    pages.div_ceil(1 << order)
}

fn bitmap_words(pages: u64, order: usize) -> u64 {
    blocks_at(pages, order).div_ceil(64)
}

fn node(addr: u64) -> *mut FreeBlock {
    PhysAddr::new(addr).to_virt().as_mut_ptr()
}

impl Buddy {
    fn bit(&self, addr: u64, order: usize) -> Option<(*mut u64, u64)> {
        let idx = (addr / PAGE_SIZE) >> order;
        if idx >= blocks_at(self.pages, order) {
            return None;
        }
        Some((
            self.bitmaps[order].wrapping_add((idx / 64) as usize),
            1 << (idx % 64),
        ))
    }

    fn is_free(&self, addr: u64, order: usize) -> bool {
        // SAFETY: `bit` vraci ukazatel jen uvnitr bitmapy daneho orderu.
        self.bit(addr, order)
            .is_some_and(|(w, m)| unsafe { *w & m != 0 })
    }

    fn set_free(&mut self, addr: u64, order: usize, free: bool) {
        let (w, m) = self.bit(addr, order).expect("blok mimo spravovanou pamet");
        // SAFETY: viz `is_free`.
        unsafe { if free { *w |= m } else { *w &= !m } };
    }

    fn push(&mut self, addr: u64, order: usize) {
        let head = self.free_lists[order];
        // SAFETY: blok `addr` je volny a patri alokatoru, `head` take.
        unsafe {
            node(addr).write(FreeBlock {
                next: head,
                prev: NONE,
            });
            if head != NONE {
                (*node(head)).prev = addr;
            }
        }
        self.free_lists[order] = addr;
        self.set_free(addr, order, true);
    }

    fn remove(&mut self, addr: u64, order: usize) {
        // SAFETY: blok je v seznamu (bit v bitmape je nastaveny), takze
        // jeho uzel i sousede jsou platne volne bloky.
        unsafe {
            let FreeBlock { next, prev } = node(addr).read();
            if prev != NONE {
                (*node(prev)).next = next;
            } else {
                self.free_lists[order] = next;
            }
            if next != NONE {
                (*node(next)).prev = prev;
            }
        }
        self.set_free(addr, order, false);
    }

    fn alloc(&mut self, order: usize) -> Option<u64> {
        let found = (order..=MAX_ORDER).find(|&k| self.free_lists[k] != NONE)?;
        let addr = self.free_lists[found];
        self.remove(addr, found);
        // Rozdeleni vetsiho bloku: horni poloviny se vraci do seznamu.
        for k in (order..found).rev() {
            self.push(addr + block_bytes(k), k);
        }
        self.free_pages -= 1 << order;
        Some(addr)
    }

    fn free(&mut self, mut addr: u64, mut order: usize) {
        assert!(
            addr.is_multiple_of(block_bytes(order)),
            "free: nezarovnany blok {addr:#x}"
        );
        assert!(
            !self.is_free(addr, order),
            "free: blok {addr:#x} uz je volny"
        );
        self.free_pages += 1 << order;
        while order < MAX_ORDER {
            let buddy = addr ^ block_bytes(order);
            if !self.is_free(buddy, order) {
                break;
            }
            self.remove(buddy, order);
            addr = addr.min(buddy);
            order += 1;
        }
        self.push(addr, order);
    }

    /// Prida rozsah fyzicke pameti jako volny.
    fn add_range(&mut self, start: u64, end: u64) {
        let mut addr = start.max(LOW_MEMORY_LIMIT).next_multiple_of(PAGE_SIZE);
        let end = end.min(self.pages * PAGE_SIZE) & !(PAGE_SIZE - 1);
        while addr < end {
            let order = (0..=MAX_ORDER)
                .rev()
                .find(|&k| addr.is_multiple_of(block_bytes(k)) && addr + block_bytes(k) <= end)
                .unwrap();
            self.total_pages += 1 << order;
            self.free(addr, order);
            addr += block_bytes(order);
        }
    }
}

/// Inicializuje alokator z oblasti `Usable`. Bitmapy si vezme ze zacatku
/// prvni dost velke volne oblasti.
pub fn init(regions: &[MemoryRegion]) {
    let is_ram = |r: &&MemoryRegion| matches!(r.kind, MemoryKind::Usable | MemoryKind::Reclaimable);
    let max_addr = regions
        .iter()
        .filter(is_ram)
        .map(|r| r.end())
        .max()
        .expect("zadna RAM");
    let pages = max_addr / PAGE_SIZE;

    let words: u64 = (0..=MAX_ORDER).map(|k| bitmap_words(pages, k)).sum();
    let bitmap_bytes = (words * 8).next_multiple_of(PAGE_SIZE);
    let bitmap_region = regions
        .iter()
        .filter(|r| r.kind == MemoryKind::Usable && r.start >= LOW_MEMORY_LIMIT)
        .find(|r| r.len >= bitmap_bytes)
        .expect("neni misto pro bitmapy alokatoru");
    let bitmap_start = bitmap_region.start;
    let bitmap_end = bitmap_start + bitmap_bytes;

    let base: *mut u64 = PhysAddr::new(bitmap_start).to_virt().as_mut_ptr();
    // SAFETY: oblast je Usable (nikdo ji nepouziva), namapovana v prime mape
    // a ma alespon `bitmap_bytes` bajtu.
    unsafe { base.write_bytes(0, (bitmap_bytes / 8) as usize) };
    let mut bitmaps = [core::ptr::null_mut(); MAX_ORDER + 1];
    let mut offset = 0;
    for (k, bitmap) in bitmaps.iter_mut().enumerate() {
        *bitmap = base.wrapping_add(offset as usize);
        offset += bitmap_words(pages, k);
    }

    let mut buddy = Buddy {
        pages,
        bitmaps,
        free_lists: [NONE; MAX_ORDER + 1],
        free_pages: 0,
        total_pages: 0,
    };
    for r in regions.iter().filter(|r| r.kind == MemoryKind::Usable) {
        if r.start == bitmap_start {
            buddy.add_range(bitmap_end, r.end());
        } else {
            buddy.add_range(r.start, r.end());
        }
    }
    *ALLOCATOR.lock() = Some(buddy);
}

/// Prida do alokatoru vsechny oblasti daneho druhu (napr. pamet loaderu
/// a boot services, kdyz uz ji jadro nepotrebuje).
pub fn add_regions(regions: &[MemoryRegion], kind: MemoryKind) {
    let mut guard = ALLOCATOR.lock();
    let buddy = guard.as_mut().expect("alokator neni inicializovany");
    for r in regions.iter().filter(|r| r.kind == kind) {
        buddy.add_range(r.start, r.end());
    }
}

/// Alokuje 2^order souvislych stranek zarovnanych na svou velikost.
pub fn alloc_pages(order: usize) -> Option<PhysAddr> {
    if order > MAX_ORDER {
        return None;
    }
    ALLOCATOR.lock().as_mut()?.alloc(order).map(PhysAddr::new)
}

/// Uvolni blok alokovany `alloc_pages` se stejnym `order`.
pub fn free_pages(addr: PhysAddr, order: usize) {
    ALLOCATOR
        .lock()
        .as_mut()
        .expect("alokator neni inicializovany")
        .free(addr.as_u64(), order);
}

/// Alokuje jednu vynulovanou stranku.
pub fn alloc_frame_zeroed() -> Option<PhysAddr> {
    let frame = alloc_pages(0)?;
    // SAFETY: prave alokovana stranka, dostupna pres primou mapu.
    unsafe {
        frame
            .to_virt()
            .as_mut_ptr::<u8>()
            .write_bytes(0, PAGE_SIZE as usize)
    };
    Some(frame)
}

/// (volne stranky, celkem spravovane stranky)
pub fn stats() -> (u64, u64) {
    ALLOCATOR
        .lock()
        .as_ref()
        .map_or((0, 0), |b| (b.free_pages, b.total_pages))
}
