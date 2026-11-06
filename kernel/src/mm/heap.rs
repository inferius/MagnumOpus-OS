//! Kernel heap (globalni alokator pro `alloc`).
//!
//! - Male alokace (do 2 KiB): tridy velikosti mocnin dvou od 16 B. Kazda
//!   trida ma seznam volnych objektu; kdyz dojde, vezme se nova stranka
//!   a rozreze se. Objekty jsou zarovnane na svou velikost, takze staci
//!   tridu zvolit podle max(size, align).
//! - Velke alokace: primo bloky z buddy alokatoru (zarovnane na svou velikost).
//!
//! `dealloc` dostane stejny `Layout`, takze zadne hlavicky nejsou potreba.
//!
//! TODO: vracet prazdne stranky trid zpet buddy alokatoru, per-CPU cache.
//! Pozor: zamek neni bezpecny vuci prerusenim - obsluha preruseni nesmi
//! alokovat, dokud se to nevyresi.

use core::alloc::{GlobalAlloc, Layout};

use spin::Mutex;

use super::{PhysAddr, VirtAddr, phys};
use crate::arch::paging::PAGE_SIZE;

const MIN_CLASS_SHIFT: u32 = 4; // 16 B
const MAX_CLASS_SHIFT: u32 = 11; // 2 KiB
const CLASSES: usize = (MAX_CLASS_SHIFT - MIN_CLASS_SHIFT + 1) as usize;

/// Hlavicky seznamu volnych objektu (virtualni adresy, 0 = prazdny).
struct Classes([usize; CLASSES]);

static CLASSES_FREE: Mutex<Classes> = Mutex::new(Classes([0; CLASSES]));

pub struct KernelHeap;

#[global_allocator]
static HEAP: KernelHeap = KernelHeap;

fn class_of(layout: &Layout) -> Option<usize> {
    let size = layout.size().max(layout.align()).max(1 << MIN_CLASS_SHIFT);
    let shift = size.next_power_of_two().trailing_zeros();
    (shift <= MAX_CLASS_SHIFT).then(|| (shift - MIN_CLASS_SHIFT) as usize)
}

fn order_of(layout: &Layout) -> usize {
    let size = layout.size().max(layout.align()) as u64;
    let pages = size.div_ceil(PAGE_SIZE).next_power_of_two();
    pages.trailing_zeros() as usize
}

// SAFETY: alloc vraci bud null, nebo pamet o velikosti alespon `layout.size()`
// zarovnanou na `layout.align()`, kterou nikdo jiny nepouziva (vyjmuta ze
// seznamu volnych objektu nebo nove alokovana z buddy alokatoru).
unsafe impl GlobalAlloc for KernelHeap {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let Some(class) = class_of(&layout) else {
            return phys::alloc_pages(order_of(&layout))
                .map_or(core::ptr::null_mut(), |p| p.to_virt().as_mut_ptr());
        };
        let mut free = CLASSES_FREE.lock();
        if free.0[class] == 0 {
            let Some(page) = phys::alloc_pages(0) else {
                return core::ptr::null_mut();
            };
            let obj_size = 1usize << (class as u32 + MIN_CLASS_SHIFT);
            let base = page.to_virt().as_u64() as usize;
            // Rozrezani stranky na objekty spojene do seznamu.
            for offset in (0..PAGE_SIZE as usize).step_by(obj_size).rev() {
                let obj = (base + offset) as *mut usize;
                // SAFETY: obj lezi uvnitr prave alokovane stranky a je
                // zarovnany alespon na 16 B.
                unsafe { obj.write(free.0[class]) };
                free.0[class] = obj as usize;
            }
        }
        let obj = free.0[class] as *mut usize;
        // SAFETY: obj je hlava seznamu volnych objektu teto tridy, prvni
        // slovo volneho objektu obsahuje odkaz na dalsi.
        free.0[class] = unsafe { obj.read() };
        obj.cast()
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        match class_of(&layout) {
            Some(class) => {
                let mut free = CLASSES_FREE.lock();
                // SAFETY: podle kontraktu GlobalAlloc ptr pochazi z `alloc`
                // se stejnym layoutem, takze jde o objekt teto tridy, ktery
                // uz nikdo nepouziva.
                unsafe { ptr.cast::<usize>().write(free.0[class]) };
                free.0[class] = ptr as usize;
            }
            None => {
                let virt = VirtAddr::new(ptr as u64);
                phys::free_pages(PhysAddr::from_direct_map(virt), order_of(&layout));
            }
        }
    }
}
