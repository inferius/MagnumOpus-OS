//! Start procesu aplikace se `std`.
//!
//! Jadro skoci na `_start` s handlem bootstrap kanalu. Runtime precte
//! startovni zpravu (`libos::rt::startup`), rozdeli handly (blok
//! parametru, stdio, relace FS), pripravi TLS hlavniho vlakna a zavola
//! `main`, kterou pro program se `std` vygeneruje rustc (vola
//! `std::rt::lang_start`).

use core::cell::UnsafeCell;
use core::ptr;
use core::sync::atomic::{AtomicBool, Ordering};

use libos::{Channel, Startup};

/// Hodnota nastavena jednou pri startu (pred `main` a pred prvnim vlaknem),
/// pak jen ctena.
pub(crate) struct Init<T>(UnsafeCell<Option<T>>);

// SAFETY: zapisuje se jen v `start_rust` pred spustenim dalsich vlaken;
// potom uz jen cteni.
unsafe impl<T: Sync> Sync for Init<T> {}

impl<T> Init<T> {
    pub(crate) const fn new() -> Self {
        Self(UnsafeCell::new(None))
    }

    /// # Safety
    ///
    /// Volat jen pri startu, kdy bezi jedine vlakno a nikdo hodnotu necte.
    pub(crate) unsafe fn set(&self, value: T) {
        // SAFETY: predano volajicim.
        unsafe { *self.0.get() = Some(value) };
    }

    pub(crate) fn get(&self) -> Option<&T> {
        // SAFETY: po startu se hodnota uz nemeni (viz `set`).
        unsafe { (*self.0.get()).as_ref() }
    }
}

/// Bootstrap kanal (spojeni s rodicem).
static BOOTSTRAP: Init<Channel> = Init::new();

/// Bootstrap kanal od rodice (napr. pro protokol mezi rodicem a ditetem).
pub fn bootstrap_channel() -> Option<&'static Channel> {
    BOOTSTRAP.get()
}

/// Netagovane handly startovni zpravy (`Startup::handles`, v poradi
/// zpravy): aplikace s oknem ma 0 = kanal okna, 1 = listek, 2+ = dalsi
/// (init `gui.rs`). Runtime je nepouziva; vezme si je knihovna aplikace
/// (`mosdl`) pres [`take_handles`].
static HANDLES: Init<alloc::vec::Vec<u32>> = Init::new();
static HANDLES_TAKEN: AtomicBool = AtomicBool::new(false);

/// Tagovane handly, ktere runtime nepouzil (napr. `tag::AUDIO` pro
/// `mosdl`): (stitek, syrovy handle). Bere je [`take_tagged`].
static TAGGED: libos::sync::Mutex<alloc::vec::Vec<(u32, u32)>> = libos::sync::Mutex::new(alloc::vec::Vec::new());

/// Vezme tagovany handle startovni zpravy, ktery runtime nepouzil
/// (volajici ho vlastni a zavira). Kazdy jen jednou.
pub fn take_tagged(tag: u32) -> Option<u32> {
    let mut t = TAGGED.lock();
    let i = t.iter().position(|(k, _)| *k == tag)?;
    Some(t.remove(i).1)
}

/// Vezme netagovane handly startovni zpravy (syrova cisla, volajici je
/// vlastni a zavira). Jen jednou: dalsi volani vrati prazdny seznam.
pub fn take_handles() -> alloc::vec::Vec<u32> {
    if HANDLES_TAKEN.swap(true, Ordering::AcqRel) {
        return alloc::vec::Vec::new();
    }
    HANDLES.get().cloned().unwrap_or_default()
}

/// Syrovy handle vlastniho procesu (pro dalsi kopii `libos` v aplikaci:
/// `libos::rt::set_self_process`).
pub fn self_process() -> u32 {
    libos::rt::self_process()
}

#[cfg(all(target_arch = "x86_64", not(target_env = "dyn")))]
#[unsafe(naked)]
#[unsafe(no_mangle)]
extern "C" fn _start() -> ! {
    // Zarovnani jako po instrukci call, nulovy ukazatel ramce (konec
    // vypisu zasobniku).
    core::arch::naked_asm!("xor ebp, ebp", "and rsp, -16", "call {}", "ud2", sym start_rust)
}

#[cfg(all(target_arch = "aarch64", not(target_env = "dyn")))]
#[unsafe(naked)]
#[unsafe(no_mangle)]
extern "C" fn _start() -> ! {
    core::arch::naked_asm!(
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

#[cfg(not(target_env = "dyn"))]
unsafe extern "C" {
    /// Vygeneruje rustc pro binarku se `std` (`lang_start`), nebo je to
    /// `main` programu v C (crate `#![no_main]` s knihovnou `cstd`).
    fn main(argc: i32, argv: *const *const u8) -> i32;
}

/// `argv` pro C: retezce zakoncene nulou a pole ukazatelu zakoncene NULL
/// (zije do konce procesu). std argumenty cte samo pres `env::args`.
fn c_argv() -> (i32, *const *const u8) {
    let args = crate::env::args();
    let mut ptrs: alloc::vec::Vec<*const u8> = alloc::vec::Vec::with_capacity(args.len() + 1);
    for a in args {
        let mut s = alloc::vec::Vec::with_capacity(a.len() + 1);
        s.extend_from_slice(a);
        s.push(0);
        ptrs.push(alloc::boxed::Box::leak(s.into_boxed_slice()).as_ptr());
    }
    ptrs.push(ptr::null());
    let argc = i32::try_from(args.len()).unwrap_or(i32::MAX);
    (argc, alloc::boxed::Box::leak(ptrs.into_boxed_slice()).as_ptr())
}

/// Funkce `main` programu v C (`int main(int, char **)`).
pub type CMain = unsafe extern "C" fn(i32, *const *const u8) -> i32;

/// Start programu PIE nad `libc.so` (cil `env: dyn`): `_start` z crt1.o
/// programu (sysroot C) preda `main`, bootstrap a SP s auxv a magic od
/// zavadece (`abi::loader::ENTRY_MAGIC`). TLS uz pripravil ld.so.
///
/// # Safety
///
/// Volat jen jednou z `_start` programu s hodnotami od zavadece.
#[cfg(target_env = "dyn")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __libc_start(main: CMain, bootstrap: u64, sp: u64, magic: u64) -> ! {
    // SAFETY: SP (auxv) a magic predal zavadec (`_start` je prenesl).
    unsafe { libos::loader::init(sp, magic) };
    let startup = libos::rt::startup(bootstrap, |_| {});
    // SAFETY: bezi jen toto vlakno.
    unsafe { init(startup) };
    // Konstruktory knihoven a programu (C++ staticke objekty) az nad
    // pripravenym runtime (heap, stdio, soubory), pred main.
    libos::loader::run_init();
    let (argc, argv) = c_argv();
    // SAFETY: `main` programu; `argv` zije do konce procesu.
    let code = unsafe { main(argc, argv) };
    crate::exit(code)
}

#[cfg(not(target_env = "dyn"))]
extern "C" fn start_rust(bootstrap: u64) -> ! {
    // Heap (dlmalloc) si pamet mapuje sam pri prvni alokaci; staci, ze
    // `startup` nastavi handle vlastniho procesu.
    let startup = libos::rt::startup(bootstrap, |_| {});
    // SAFETY: bezi jen toto vlakno, nikdo nic necte.
    unsafe { init(startup) };
    let (argc, argv) = c_argv();
    // SAFETY: konstruktory programu (staticke objekty C++), runtime je
    // pripraveny; jen jednou pred main.
    unsafe { init_array::run_init(argc, argv) };
    // SAFETY: `main` vygeneroval rustc nebo je z programu v C; `argv`
    // zije do konce procesu.
    let code = unsafe { main(argc, argv) };
    crate::exit(code)
}

/// `.init_array` a `.fini_array` statickeho programu (symboly linker
/// skriptu `user/user-std.ld`). Pod zavadecem je spousti ld.so.
#[cfg(not(target_env = "dyn"))]
pub(crate) mod init_array {
    /// Konstruktor: jako glibc dostane `argc`, `argv`, `envp` (vetsina je
    /// bez argumentu, to ABI C dovoluje).
    type InitFn = unsafe extern "C" fn(i32, *const *const u8, *const *const u8);
    type FiniFn = unsafe extern "C" fn();

    unsafe extern "C" {
        static __init_array_start: InitFn;
        static __init_array_end: InitFn;
        static __fini_array_start: FiniFn;
        static __fini_array_end: FiniFn;
    }

    /// # Safety
    ///
    /// Jednou pri startu, pred `main`.
    pub(crate) unsafe fn run_init(argc: i32, argv: *const *const u8) {
        let mut p = &raw const __init_array_start;
        let end = &raw const __init_array_end;
        while p < end {
            // SAFETY: pole ukazatelu na funkce od linkeru.
            unsafe {
                (*p)(argc, argv, core::ptr::null());
                p = p.add(1);
            }
        }
    }

    /// Destruktory `.fini_array` v obracenem poradi (po funkcich `at_exit`).
    pub(crate) fn run_fini() {
        let start = &raw const __fini_array_start;
        let mut p = &raw const __fini_array_end;
        while p > start {
            // SAFETY: pole ukazatelu na funkce od linkeru; `exit` je vola
            // jednou.
            unsafe {
                p = p.sub(1);
                (*p)();
            }
        }
    }
}

/// # Safety
///
/// Jen jednou pri startu, jedine vlakno.
unsafe fn init(mut startup: Startup) {
    // TLS hlavniho vlakna jako prvni: std ho pouziva hned v `lang_start`.
    crate::tls::init_main_thread();
    let params = startup.params.take().and_then(|vmo| libos::params::Params::read(&vmo).ok());
    let flags = params.as_ref().map_or(0, |p| p.flags);
    let take = |s: &mut Startup, tag| s.take_tagged(tag).map(Channel::from_handle);
    let stdin = take(&mut startup, abi::params::tag::STDIN);
    let stdout = take(&mut startup, abi::params::tag::STDOUT);
    let stderr = take(&mut startup, abi::params::tag::STDERR);
    let fs = take(&mut startup, abi::params::tag::FS);
    let net = take(&mut startup, abi::params::tag::NET);
    let launch = take(&mut startup, abi::params::tag::LAUNCH);
    // SAFETY: predano volajicim.
    unsafe {
        crate::env::init(params.unwrap_or_default());
        crate::stdio::init(stdin, stdout, stderr, flags);
        crate::fs::init(fs);
        crate::net::init(net);
        crate::process::init(launch);
        BOOTSTRAP.set(startup.bootstrap);
        HANDLES.set(core::mem::take(&mut startup.handles).into_iter().map(|h| h.into_raw()).collect());
    }
    *TAGGED.lock() = core::mem::take(&mut startup.tagged).into_iter().map(|(t, h)| (t, h.into_raw())).collect();
    // Handle vlastniho procesu pouziva runtime po celou dobu behu
    // (`libos::rt::self_process`: mapovani pameti, vlakna); zavrit se
    // nesmi. Netagovane handly ceka `take_handles`; ostatni nepouzite
    // handly ze zpravy se zavrou.
    core::mem::forget(startup.process);
}
