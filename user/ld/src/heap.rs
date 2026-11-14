//! Alokator zavadece: `linked_list_allocator` (first-fit, uvolnuje -
//! `dlopen`, bloky dynamickeho TLS, chyby `dlerror`) nad rezervaci adres,
//! kterou vybere jadro (nepotka se s heapem programu na pevne adrese
//! libos). Pamet se do rezervace dava po kusech, az je potreba.

use core::alloc::{GlobalAlloc, Layout};
use core::ptr::NonNull;
use core::sync::atomic::{AtomicU64, Ordering};

use abi::vm::{MAP_READ, MAP_WRITE};
use linked_list_allocator::LockedHeap;

/// Rezervace adres heapu (jen adresy, pamet az pri rustu).
const RESERVE: u64 = 1 << 30;
/// Nejmensi prirustek.
const GROW: u64 = 256 * 1024;

struct LdHeap {
    heap: LockedHeap,
    /// Konec rezervace (0 = heap jeste nevznikl).
    limit: AtomicU64,
}

#[global_allocator]
static HEAP: LdHeap = LdHeap { heap: LockedHeap::empty(), limit: AtomicU64::new(0) };

// SAFETY: vse pod zamkem heapu; pridana pamet je cerstve namapovana
// v rezervaci, navazuje na konec heapu a nikdo jiny ji nepouziva.
unsafe impl GlobalAlloc for LdHeap {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let mut h = self.heap.lock();
        if let Ok(p) = h.allocate_first_fit(layout) {
            return p.as_ptr();
        }
        // `page_size` nealokuje (smi se volat v alokatoru).
        let grow = ((layout.size() + layout.align() + 64) as u64).max(GROW).next_multiple_of(libos::vm::page_size());
        let rw = MAP_READ | MAP_WRITE;
        if self.limit.load(Ordering::Relaxed) == 0 {
            let Ok(base) = libos::vm::reserve(RESERVE, 0) else { return core::ptr::null_mut() };
            if libos::vm::commit(base, grow, rw).is_err() {
                return core::ptr::null_mut();
            }
            self.limit.store(base + RESERVE, Ordering::Relaxed);
            // SAFETY: `base..base + grow` je prave namapovana pamet RW.
            unsafe { h.init(base as *mut u8, grow as usize) };
        } else {
            let top = h.top() as u64;
            if top + grow > self.limit.load(Ordering::Relaxed) || libos::vm::commit(top, grow, rw).is_err() {
                return core::ptr::null_mut();
            }
            // SAFETY: `top..top + grow` je prave namapovany a navazuje na heap.
            unsafe { h.extend(grow as usize) };
        }
        h.allocate_first_fit(layout).map_or(core::ptr::null_mut(), |p| p.as_ptr())
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        if let Some(p) = NonNull::new(ptr) {
            // SAFETY: `ptr` vratila `alloc` s timtez `layout`.
            unsafe { self.heap.lock().deallocate(p, layout) };
        }
    }
}
