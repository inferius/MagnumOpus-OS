//! Behove prostredi procesu: vstupni bod, heap, panic handler a startovni
//! zprava.

use alloc::vec::Vec;
use core::arch::naked_asm;
use core::panic::PanicInfo;

use abi::bootstrap::{self, Entry, Header};
use abi::vm::{MAP_READ, MAP_WRITE};
use linked_list_allocator::LockedHeap;

use crate::{Channel, Handle, Process, Vmo, println, sys};

/// Velikost heapu procesu. TODO: rust podle potreby (vice objektu pameti).
const HEAP_SIZE: u64 = 4 * 1024 * 1024;

#[global_allocator]
static HEAP: LockedHeap = LockedHeap::empty();

/// Co proces dostal pri startu.
pub struct Startup {
    /// Handle na vlastni proces (mapovani pameti, vlakna).
    pub process: Process,
    /// Obraz initfs (dostava jen init).
    pub initfs: Option<Vmo>,
    /// Handly predane rodicem (`kind::USER`), v poradi zpravy.
    pub handles: Vec<Handle>,
    /// Bootstrap kanal: spojeni s rodicem.
    pub bootstrap: Channel,
}

unsafe extern "Rust" {
    /// Definuje program makrem `libos::entry!`.
    fn __libos_main(startup: Startup) -> i64;
}

/// Vstupni bod procesu. Jadro skoci sem s handlem bootstrap kanalu v `rdi`
/// a zasobnikem zarovnanym na 16 B; vlastni kod chce zarovnani jako po
/// instrukci `call`, proto tenky asembler.
#[unsafe(naked)]
#[unsafe(no_mangle)]
extern "C" fn _start() -> ! {
    naked_asm!("xor ebp, ebp", "and rsp, -16", "call {}", "ud2", sym start_rust)
}

extern "C" fn start_rust(bootstrap: u64) -> ! {
    let bootstrap = Channel::from_handle(Handle::from_raw(bootstrap as u32));
    // Startovni zprava ceka ve fronte uz pri startu; heap jeste neni, takze
    // ji cteme do bufferu na zasobniku.
    let mut data = [0u8; 512];
    let mut raw = [0u32; 16];
    let (r, (dlen, hlen)) = sys::channel_read(bootstrap.handle().raw(), &mut data, &mut raw);
    if r.is_err() {
        sys::debug_log("libos: chybi nebo je prilis velka startovni zprava");
        sys::process_exit(abi::process::EXIT_PANIC);
    }
    let Some(header) = Header::from_bytes(&data[..dlen]) else {
        sys::debug_log("libos: neplatna startovni zprava");
        sys::process_exit(abi::process::EXIT_PANIC);
    };

    let mut process = None;
    let mut initfs = None;
    let mut user = [0u32; 16];
    let mut user_count = 0;
    let count = (header.count as usize).min(hlen);
    for (i, &handle) in raw[..count].iter().enumerate() {
        let off = bootstrap::HEADER_SIZE + i * bootstrap::ENTRY_SIZE;
        // Poskozena zprava (kratsi nez pocet polozek) nesmi shodit start.
        let kind = data
            .get(off..dlen)
            .and_then(Entry::from_bytes)
            .map_or(0, |e| e.kind);
        match kind {
            bootstrap::kind::SELF_PROCESS => process = Some(handle),
            bootstrap::kind::INITFS => initfs = Some(handle),
            _ => {
                user[user_count] = handle;
                user_count += 1;
            }
        }
    }
    let Some(process) = process else {
        sys::debug_log("libos: startovni zprava neobsahuje handle procesu");
        sys::process_exit(abi::process::EXIT_PANIC);
    };

    init_heap(process);
    let startup = Startup {
        process: Process::from_handle(Handle::from_raw(process)),
        initfs: initfs.map(|h| Vmo::from_handle(Handle::from_raw(h))),
        handles: user[..user_count]
            .iter()
            .map(|&h| Handle::from_raw(h))
            .collect(),
        bootstrap,
    };
    // SAFETY: funkci definuje program makrem `entry!` se spravnou signaturou.
    let code = unsafe { __libos_main(startup) };
    sys::process_exit(code)
}

fn init_heap(process: u32) {
    let Ok(vmo) = sys::vmo_create(HEAP_SIZE) else {
        sys::debug_log("libos: nedostatek pameti pro heap");
        sys::process_exit(abi::process::EXIT_PANIC);
    };
    let Ok(addr) = sys::vm_map(process, vmo, 0, HEAP_SIZE, MAP_READ | MAP_WRITE, 0) else {
        sys::debug_log("libos: heap nelze namapovat");
        sys::process_exit(abi::process::EXIT_PANIC);
    };
    // Mapovani drzi objekt pameti, handle uz neni potreba.
    let _ = sys::handle_close(vmo);
    // SAFETY: rozsah je prave namapovany pro cteni i zapis, nikdo jiny ho
    // nepouziva a zustane namapovany do konce procesu.
    unsafe { HEAP.lock().init(addr as *mut u8, HEAP_SIZE as usize) };
}

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    println!("PANIC: {info}");
    sys::process_exit(abi::process::EXIT_PANIC)
}
