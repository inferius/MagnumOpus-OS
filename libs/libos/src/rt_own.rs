//! Vlastni behove prostredi libos pro programy `no_std`: vstupni bod,
//! heap a panic handler. Na cili magnumopus je dodava `std`.

use core::alloc::{GlobalAlloc, Layout};
use core::arch::naked_asm;
use core::panic::PanicInfo;
use core::ptr::NonNull;
use core::sync::atomic::Ordering;

use abi::vm::{MAP_FIXED, MAP_READ, MAP_WRITE};
use linked_list_allocator::LockedHeap;

use crate::rt::{Startup, fail, self_process};
use crate::sys;

/// Heap procesu zacina na pevne adrese a roste smerem nahoru: kdyz alokace
/// selze, namapuje se dalsi objekt pameti hned za konec (`MAP_FIXED`).
/// Jadro pro mapovani bez pevne adresy vybira adresy od `USER_ALLOC_BASE`,
/// daleko pod timto rozsahem.
const HEAP_BASE: u64 = 0x0000_6000_0000_0000;
/// Pocatecni velikost.
const HEAP_INITIAL: u64 = 4 * 1024 * 1024;
/// Nejmensi prirustek.
const HEAP_GROW_MIN: u64 = 4 * 1024 * 1024;
/// Heap smi rust az ke konci pameti procesu (~32 TiB adres); kolik
/// skutecne dostane, urcuje rozpocet uzivatelske pameti jobu.
const HEAP_MAX: u64 = abi::vm::USER_MAX - HEAP_BASE;

/// Heap: male alokace (do 4 KiB) ve velikostnich tridach (`sizeclass`, O(1)),
/// velke a kusy pro tridy v `linked_list_allocator` (first-fit). Bez trid
/// se seznam volnych useku po roztristeni heapu (fsd s velkou cache)
/// natahl na tisice polozek a kazda alokace stala milisekundy.
struct GrowingHeap {
    big: LockedHeap,
    classes: Spin<sizeclass::Classes>,
}

/// Jednoduchy spin zamek (alokator nesmi alokovat ani cekat na jadro).
struct Spin<T> {
    locked: core::sync::atomic::AtomicBool,
    value: core::cell::UnsafeCell<T>,
}

// SAFETY: k hodnote se pristupuje jen pod zamkem (`with`).
unsafe impl<T: Send> Sync for Spin<T> {}

impl<T> Spin<T> {
    const fn new(value: T) -> Self {
        Self { locked: core::sync::atomic::AtomicBool::new(false), value: core::cell::UnsafeCell::new(value) }
    }

    fn with<R>(&self, f: impl FnOnce(&mut T) -> R) -> R {
        while self
            .locked
            .compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            core::hint::spin_loop();
        }
        // SAFETY: zamek je nas, nikdo jiny k hodnote nesaha.
        let r = f(unsafe { &mut *self.value.get() });
        self.locked.store(false, Ordering::Release);
        r
    }
}

#[global_allocator]
static HEAP: GrowingHeap = GrowingHeap { big: LockedHeap::empty(), classes: Spin::new(sizeclass::Classes::new()) };

impl GrowingHeap {
    /// Velka alokace (a kus pro tridu): first-fit, pri nedostatku heap
    /// naroste.
    fn alloc_big(&self, layout: Layout) -> *mut u8 {
        let mut heap = self.big.lock();
        if let Ok(p) = heap.allocate_first_fit(layout) {
            return p.as_ptr();
        }
        let need = (layout.size() + layout.align() + 64) as u64;
        // Rust o polovinu (mene volani pri postupnem rustu), zarovnane na stranky.
        // `page_size` tady nesmi alokovat (jsme uvnitr alokatoru) - nealokuje.
        let grow = need
            .max(HEAP_GROW_MIN)
            .max(heap.size() as u64 / 2)
            .next_multiple_of(crate::vm::page_size());
        let top = heap.top() as u64;
        if top + grow > HEAP_BASE + HEAP_MAX || !map_heap(self_process(), top, grow) {
            return core::ptr::null_mut();
        }
        // SAFETY: `top..top + grow` je prave namapovany a navazuje na heap.
        unsafe { heap.extend(grow as usize) };
        heap.allocate_first_fit(layout)
            .map_or(core::ptr::null_mut(), |p| p.as_ptr())
    }
}

// SAFETY: male alokace jdou pres tridy pod zamkem `classes` (kusy dava
// `alloc_big`), velke pres zamceny `linked_list_allocator::Heap`; trida se
// urcuje z layoutu stejne pri alokaci i uvolneni. Pridana pamet heapu je
// cerstve namapovana, navazuje na konec a nikdo jiny ji nepouziva.
unsafe impl GlobalAlloc for GrowingHeap {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        match sizeclass::class_of(layout) {
            // SAFETY: kus vraci `alloc_big` (null nebo volna pamet podle layoutu).
            Some(c) => self.classes.with(|cl| unsafe { cl.alloc(c, |l| self.alloc_big(l)) }),
            None => self.alloc_big(layout),
        }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        let Some(p) = NonNull::new(ptr) else { return };
        match sizeclass::class_of(layout) {
            // SAFETY: `ptr` vratila `alloc` pro tutez tridu (stejny layout).
            Some(c) => self.classes.with(|cl| unsafe { cl.dealloc(c, ptr) }),
            // SAFETY: `ptr` vratila `alloc_big` s timtez `layout`.
            None => unsafe { self.big.lock().deallocate(p, layout) },
        }
    }
}

/// Namapuje novou pamet heapu na `addr`.
fn map_heap(process: u32, addr: u64, len: u64) -> bool {
    let Ok(vmo) = sys::vmo_create(len, 0) else {
        return false;
    };
    let ok = sys::vm_map(process, vmo, 0, len, MAP_READ | MAP_WRITE | MAP_FIXED, addr).is_ok();
    // Mapovani drzi objekt pameti, handle uz neni potreba.
    let _ = sys::handle_close(vmo);
    ok
}

unsafe extern "Rust" {
    /// Definuje program makrem `libos::entry!`.
    fn __libos_main(startup: Startup) -> i64;
}

/// Vstupni bod procesu. Jadro skoci sem s handlem bootstrap kanalu v prvnim
/// argumentu (x86_64 `rdi`, aarch64 `x0`) a zasobnikem zarovnanym na 16 B;
/// vlastni kod chce zarovnani jako po instrukci `call` a nulovy ukazatel
/// ramce (konec vypisu zasobniku), proto tenky asembler.
#[cfg(target_arch = "x86_64")]
#[unsafe(naked)]
#[unsafe(no_mangle)]
extern "C" fn _start() -> ! {
    naked_asm!("xor ebp, ebp", "and rsp, -16", "call {}", "ud2", sym start_rust)
}

#[cfg(target_arch = "aarch64")]
#[unsafe(naked)]
#[unsafe(no_mangle)]
extern "C" fn _start() -> ! {
    naked_asm!(
        "mov x9, sp",
        "and x9, x9, #-16",
        "mov sp, x9",
        "mov x29, xzr",
        "mov x30, xzr",
        "bl {}",
        "brk #1",
        sym start_rust
    )
}

/// `sp` a `magic` preda jen zavadec `ld.so` (`abi::loader`); bez nej jsou
/// registry nulove nebo nahodne a `magic` nesedi.
extern "C" fn start_rust(bootstrap: u64, sp: u64, magic: u64) -> ! {
    // SAFETY: registry ze vstupu procesu (zavadec nebo jadro).
    unsafe { crate::loader::init(sp, magic) };
    let startup = crate::rt::startup(bootstrap, init_heap);
    // Konstruktory modulu (pod zavadecem) az s pripravenym heapem.
    crate::loader::run_init();
    // SAFETY: funkci definuje program makrem `entry!` se spravnou signaturou.
    let code = unsafe { __libos_main(startup) };
    // Normalni konec: finalizatory modulu (pod zavadecem).
    crate::loader::run_fini();
    sys::process_exit(code)
}

fn init_heap(process: u32) {
    if !map_heap(process, HEAP_BASE, HEAP_INITIAL) {
        fail("libos: nedostatek pameti pro heap");
    }
    // SAFETY: rozsah je prave namapovany pro cteni i zapis, nikdo jiny ho
    // nepouziva a zustane namapovany do konce procesu.
    unsafe {
        HEAP.big
            .lock()
            .init(HEAP_BASE as *mut u8, HEAP_INITIAL as usize)
    };
}

/// Buffer na zasobniku pro text paniky (panika nesmi alokovat: muze vzniknout
/// prave kvuli nedostatku pameti nebo pri drzenem zamku heapu).
struct StackText {
    buf: [u8; 512],
    len: usize,
}

impl core::fmt::Write for StackText {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        // Zkratit na hranici znaku, zbytek zahodit.
        let mut n = s.len().min(self.buf.len() - self.len);
        while !s.is_char_boundary(n) {
            n -= 1;
        }
        self.buf[self.len..self.len + n].copy_from_slice(&s.as_bytes()[..n]);
        self.len += n;
        Ok(())
    }
}

static PANICKING: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    // Panika behem paniky (napr. pri formatovani) nebo soubezna panika
    // jineho vlakna: chvili pockat, at prvni vlakno stihne vypsat zpravu,
    // pak ukoncit proces.
    if PANICKING.swap(true, Ordering::AcqRel) {
        crate::time::sleep(core::time::Duration::from_millis(100));
        sys::process_exit(abi::process::EXIT_PANIC)
    }
    let mut text = StackText {
        buf: [0; 512],
        len: 0,
    };
    let _ = core::fmt::write(&mut text, format_args!("PANIC: {info}"));
    sys::debug_log(core::str::from_utf8(&text.buf[..text.len]).unwrap_or("PANIC"));
    sys::process_exit(abi::process::EXIT_PANIC)
}
