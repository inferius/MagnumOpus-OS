//! Alokator `std` (`System`): dlmalloc nad objekty pameti.
//!
//! dlmalloc si od systemu bere velke useky (segmenty) a deli je sam. Kazdy
//! segment je vlastni objekt pameti namapovany kamkoli (jadro vybere
//! adresu); uvolneny segment se odmapuje, takze pamet se vraci systemu
//! (na rozdil od heapu libos, ktery jen roste). Zamek: futex
//! (`libos::sync::Mutex`).

use core::ptr;

use abi::vm::{MAP_READ, MAP_WRITE};
use dlmalloc::Dlmalloc;
use libos::sync::Mutex;
use libos::sys;

/// Segmenty dlmallocu nad objekty pameti.
pub struct Segments;

// SAFETY: `alloc` vraci cerstve namapovanou, vynulovanou pamet zarovnanou
// na stranku, ktera patri jen dlmallocu, dokud ji nevrati `free`.
unsafe impl dlmalloc::Allocator for Segments {
    fn alloc(&self, size: usize) -> (*mut u8, usize, u32) {
        let page = libos::vm::page_size() as usize;
        let size = size.next_multiple_of(page);
        let Ok(vmo) = sys::vmo_create(size as u64, 0) else {
            return (ptr::null_mut(), 0, 0);
        };
        let r = sys::vm_map(libos::rt::self_process(), vmo, 0, size as u64, MAP_READ | MAP_WRITE, 0);
        // Mapovani drzi objekt pameti, handle uz neni potreba.
        let _ = sys::handle_close(vmo);
        match r {
            Ok(addr) => (addr as *mut u8, size, 0),
            Err(_) => (ptr::null_mut(), 0, 0),
        }
    }

    fn remap(&self, _ptr: *mut u8, _old: usize, _new: usize, _can_move: bool) -> *mut u8 {
        ptr::null_mut()
    }

    fn free_part(&self, _ptr: *mut u8, _old: usize, _new: usize) -> bool {
        false
    }

    fn free(&self, ptr: *mut u8, size: usize) -> bool {
        sys::vm_unmap(libos::rt::self_process(), ptr as u64, size as u64).is_ok()
    }

    fn can_release_part(&self, _flags: u32) -> bool {
        false
    }

    fn allocates_zeros(&self) -> bool {
        true
    }

    fn page_size(&self) -> usize {
        libos::vm::page_size() as usize
    }
}

static HEAP: Mutex<Dlmalloc<Segments>> = Mutex::new(Dlmalloc::new_with_allocator(Segments));

/// # Safety
///
/// Jako `GlobalAlloc::alloc` (`align` mocnina dvou, `size` nenulova).
pub unsafe fn alloc(size: usize, align: usize) -> *mut u8 {
    // SAFETY: predano volajicim.
    unsafe { HEAP.lock().malloc(size, align) }
}

/// # Safety
///
/// Jako `GlobalAlloc::alloc_zeroed`.
pub unsafe fn alloc_zeroed(size: usize, align: usize) -> *mut u8 {
    // SAFETY: predano volajicim.
    unsafe { HEAP.lock().calloc(size, align) }
}

/// # Safety
///
/// Jako `GlobalAlloc::dealloc` (`ptr` z teto knihovny se stejnym layoutem).
pub unsafe fn dealloc(ptr: *mut u8, size: usize, align: usize) {
    // SAFETY: predano volajicim.
    unsafe { HEAP.lock().free(ptr, size, align) }
}

/// # Safety
///
/// Jako `GlobalAlloc::realloc`.
pub unsafe fn realloc(ptr: *mut u8, size: usize, align: usize, new_size: usize) -> *mut u8 {
    // SAFETY: predano volajicim.
    unsafe { HEAP.lock().realloc(ptr, size, align, new_size) }
}
