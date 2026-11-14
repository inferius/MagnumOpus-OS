//! Start procesu aplikace se `std`.
//!
//! Jadro skoci na `_start` s handlem bootstrap kanalu. Runtime precte
//! startovni zpravu (`libos::rt::startup`), rozdeli handly (blok
//! parametru, stdio, relace FS), pripravi TLS hlavniho vlakna a zavola
//! `main`, kterou pro program se `std` vygeneruje rustc (vola
//! `std::rt::lang_start`).

use core::cell::UnsafeCell;
use core::ptr;

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

#[cfg(target_arch = "x86_64")]
#[unsafe(naked)]
#[unsafe(no_mangle)]
extern "C" fn _start() -> ! {
    // Zarovnani jako po instrukci call, nulovy ukazatel ramce (konec
    // vypisu zasobniku).
    core::arch::naked_asm!("xor ebp, ebp", "and rsp, -16", "call {}", "ud2", sym start_rust)
}

#[cfg(target_arch = "aarch64")]
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

unsafe extern "C" {
    /// Vygeneruje rustc pro binarku se `std` (`lang_start`).
    fn main(argc: i32, argv: *const *const u8) -> i32;
}

extern "C" fn start_rust(bootstrap: u64) -> ! {
    // Heap (dlmalloc) si pamet mapuje sam pri prvni alokaci; staci, ze
    // `startup` nastavi handle vlastniho procesu.
    let startup = libos::rt::startup(bootstrap, |_| {});
    // SAFETY: bezi jen toto vlakno, nikdo nic necte.
    unsafe { init(startup) };
    // SAFETY: `main` vygeneroval rustc; argumenty cte std pres `env`.
    let code = unsafe { main(0, ptr::null()) };
    crate::exit(code)
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
    }
    // Handle vlastniho procesu pouziva runtime po celou dobu behu
    // (`libos::rt::self_process`: mapovani pameti, vlakna); zavrit se
    // nesmi. Ostatni nepouzite handly ze zpravy se zavrou.
    core::mem::forget(startup.process);
}
