//! Behove prostredi procesu: vstupni bod, heap, panic handler a startovni
//! zprava.

use alloc::vec::Vec;
use core::arch::naked_asm;
use core::panic::PanicInfo;
use core::sync::atomic::{AtomicU32, Ordering};

use abi::bootstrap::{self, Entry, Header};
use abi::vm::{MAP_READ, MAP_WRITE};
use linked_list_allocator::LockedHeap;

use crate::hw::Resource;
use crate::{Channel, Handle, Process, Vmo, println, sys};

/// Velikost heapu procesu. TODO: rust podle potreby (vice objektu pameti).
const HEAP_SIZE: u64 = 4 * 1024 * 1024;

#[global_allocator]
static HEAP: LockedHeap = LockedHeap::empty();

/// Handle vlastniho procesu pro knihovnu (mapovani pameti).
static SELF_PROCESS: AtomicU32 = AtomicU32::new(0);

/// Syrovy handle vlastniho procesu (platny po startu).
pub fn self_process() -> u32 {
    SELF_PROCESS.load(Ordering::Relaxed)
}

/// Co proces dostal pri startu.
pub struct Startup {
    /// Handle na vlastni proces (mapovani pameti, vlakna).
    pub process: Process,
    /// Obraz initfs (dostava jen init).
    pub initfs: Option<Vmo>,
    /// Handly predane rodicem (`kind::USER`), v poradi zpravy.
    pub handles: Vec<Handle>,
    /// Pristup k hardwaru (dostava init, ten ho preda spravci zarizeni).
    pub root_resource: Option<Resource>,
    /// Okna ECAM (konfigurace PCI Express): objekt pameti a `arg` ze zpravy
    /// (prvni sbernice | posledni << 8 | segment << 16).
    pub pci_ecam: Vec<(Vmo, u32)>,
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

fn fail(msg: &str) -> ! {
    sys::debug_log(msg);
    sys::process_exit(abi::process::EXIT_PANIC)
}

/// Handly ze startovni zpravy roztridene podle druhu (bez heapu).
struct Received {
    process: Option<u32>,
    initfs: Option<u32>,
    resource: Option<u32>,
    ecam: [(u32, u32); 4],
    ecam_count: usize,
    user: [u32; 16],
    user_count: usize,
}

extern "C" fn start_rust(bootstrap: u64) -> ! {
    let bootstrap = Channel::from_handle(Handle::from_raw(bootstrap as u32));
    // Startovni zprava ceka ve fronte uz pri startu; heap jeste neni, takze
    // ji cteme do bufferu na zasobniku.
    let mut data = [0u8; 512];
    let mut raw = [0u32; 16];
    let (r, (dlen, hlen)) = sys::channel_read(bootstrap.handle().raw(), &mut data, &mut raw);
    if r.is_err() {
        fail("libos: chybi nebo je prilis velka startovni zprava");
    }
    let Some(header) = Header::from_bytes(&data[..dlen]) else {
        fail("libos: neplatna startovni zprava");
    };

    let mut got = Received {
        process: None,
        initfs: None,
        resource: None,
        ecam: [(0, 0); 4],
        ecam_count: 0,
        user: [0; 16],
        user_count: 0,
    };
    let count = (header.count as usize).min(hlen);
    for (i, &handle) in raw[..count].iter().enumerate() {
        let off = bootstrap::HEADER_SIZE + i * bootstrap::ENTRY_SIZE;
        // Poskozena zprava (kratsi nez pocet polozek) nesmi shodit start.
        let (kind, arg) = data
            .get(off..dlen)
            .and_then(Entry::from_bytes)
            .map_or((0, 0), |e| (e.kind, e.arg));
        match kind {
            bootstrap::kind::SELF_PROCESS => got.process = Some(handle),
            bootstrap::kind::INITFS => got.initfs = Some(handle),
            bootstrap::kind::ROOT_RESOURCE => got.resource = Some(handle),
            bootstrap::kind::PCI_ECAM if got.ecam_count < got.ecam.len() => {
                got.ecam[got.ecam_count] = (handle, arg);
                got.ecam_count += 1;
            }
            _ if got.user_count < got.user.len() => {
                got.user[got.user_count] = handle;
                got.user_count += 1;
            }
            _ => {}
        }
    }
    let Some(process) = got.process else {
        fail("libos: startovni zprava neobsahuje handle procesu");
    };

    SELF_PROCESS.store(process, Ordering::Relaxed);
    init_heap(process);
    let startup = Startup {
        process: Process::from_handle(Handle::from_raw(process)),
        initfs: got.initfs.map(|h| Vmo::from_handle(Handle::from_raw(h))),
        handles: got.user[..got.user_count]
            .iter()
            .map(|&h| Handle::from_raw(h))
            .collect(),
        root_resource: got
            .resource
            .map(|h| Resource::from_handle(Handle::from_raw(h))),
        pci_ecam: got.ecam[..got.ecam_count]
            .iter()
            .map(|&(h, arg)| (Vmo::from_handle(Handle::from_raw(h)), arg))
            .collect(),
        bootstrap,
    };
    // SAFETY: funkci definuje program makrem `entry!` se spravnou signaturou.
    let code = unsafe { __libos_main(startup) };
    sys::process_exit(code)
}

fn init_heap(process: u32) {
    let Ok(vmo) = sys::vmo_create(HEAP_SIZE) else {
        fail("libos: nedostatek pameti pro heap");
    };
    let Ok(addr) = sys::vm_map(process, vmo, 0, HEAP_SIZE, MAP_READ | MAP_WRITE, 0) else {
        fail("libos: heap nelze namapovat");
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
