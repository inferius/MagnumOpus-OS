//! Alokator zavadece: kusy pameti od jadra (adresu vybere jadro, takze se
//! nepotka s heapem programu na pevne adrese libos) a postupne prirazovani.
//! Uvolneni nic nedela: zavadec alokuje malo a jen pri startu programu.

use core::alloc::{GlobalAlloc, Layout};
use core::sync::atomic::{AtomicU64, Ordering};

/// Velikost kusu od jadra.
const CHUNK: u64 = 1 << 20;

struct Bump {
    next: AtomicU64,
    end: AtomicU64,
}

#[global_allocator]
static HEAP: Bump = Bump { next: AtomicU64::new(0), end: AtomicU64::new(0) };

// SAFETY: zavadec bezi v jednom vlakne (pred startem programu); kazda
// alokace dostane vlastni rozsah z namapovane pameti RW.
unsafe impl GlobalAlloc for Bump {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let align = layout.align() as u64;
        let size = layout.size() as u64;
        let mut at = self.next.load(Ordering::Relaxed).next_multiple_of(align);
        if at == 0 || at + size > self.end.load(Ordering::Relaxed) {
            let len = (size + align).next_multiple_of(CHUNK);
            let Ok(base) = libos::vm::alloc(len, abi::vm::MAP_READ | abi::vm::MAP_WRITE) else {
                return core::ptr::null_mut();
            };
            self.end.store(base + len, Ordering::Relaxed);
            at = base.next_multiple_of(align);
        }
        self.next.store(at + size, Ordering::Relaxed);
        at as *mut u8
    }

    unsafe fn dealloc(&self, _: *mut u8, _: Layout) {}
}
