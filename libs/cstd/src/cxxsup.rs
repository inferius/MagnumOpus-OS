//! Funkce C, ktere potrebuje beh C++ (libc++, libc++abi): `getentropy`
//! (`std::random_device`), `aligned_alloc` a `posix_memalign` (`operator
//! new` se zarovnanim), `timespec_get` (C11, `<ctime>`). Plan: docs/cs/cpp.md.
//!
//! Locale a siroke znaky (`<locale>`, `<cwchar>`) jsou jinde (`wchar.rs`,
//! `locale.rs`).

use core::ffi::c_void;

use crate::stat::timespec;
use crate::{c_int, errno, size_t, sys, time};

/// Nejvic bajtu na jedno volani `getentropy` (POSIX 2024, jako glibc).
pub const GETENTROPY_MAX: usize = 256;

/// `TIME_UTC` (C11) a `TIME_MONOTONIC` (C23) pro `timespec_get`.
pub const TIME_UTC: c_int = 1;
pub const TIME_MONOTONIC: c_int = 2;

/// Jak dlouho cekat mezi pokusy, dokud generator jadra neni nasazeny.
const SEED_WAIT: core::time::Duration = core::time::Duration::from_millis(10);

/// Naplni `buf` z nasazeneho generatoru jadra; nenasazeny generator ceka
/// (jako Linux `getentropy` pred inicializaci `urandom`).
pub fn entropy_blocking(buf: &mut [u8]) {
    while !sys::entropy(buf) {
        std::thread::sleep(SEED_WAIT);
    }
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn getentropy(buf: *mut c_void, len: size_t) -> c_int {
    if len > GETENTROPY_MAX {
        // POSIX 2024 rika EINVAL (glibc vraci EIO); drzime se POSIX.
        return errno::ret(Err(errno::EINVAL));
    }
    if len == 0 {
        return 0;
    }
    if buf.is_null() {
        return errno::ret(Err(errno::EFAULT));
    }
    // SAFETY: kontrakt C: `buf` ma aspon `len` bajtu.
    let b = unsafe { core::slice::from_raw_parts_mut(buf.cast::<u8>(), len) };
    entropy_blocking(b);
    0
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub extern "C" fn aligned_alloc(align: size_t, size: size_t) -> *mut c_void {
    // C17: zarovnani musi byt platne (mocnina 2); velikost nemusi byt
    // nasobkem zarovnani (DR 460).
    match cstd_core::stdlib::alloc_aligned(size, align) {
        Some(p) if !p.is_null() => p.cast(),
        Some(_) => {
            errno::set(errno::ENOMEM);
            core::ptr::null_mut()
        }
        None => {
            errno::set(errno::EINVAL);
            core::ptr::null_mut()
        }
    }
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn posix_memalign(out: *mut *mut c_void, align: size_t, size: size_t) -> c_int {
    // POSIX: zarovnani je mocnina 2 a nasobek sizeof(void *); chyba se
    // vraci, errno se nemeni.
    if !align.is_power_of_two() || align % core::mem::size_of::<*mut c_void>() != 0 {
        return errno::EINVAL;
    }
    match cstd_core::stdlib::alloc_aligned(size, align) {
        Some(p) if !p.is_null() => {
            // SAFETY: kontrakt C: `out` je platny ukazatel.
            unsafe { *out = p.cast() };
            0
        }
        _ => errno::ENOMEM,
    }
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn timespec_get(ts: *mut timespec, base: c_int) -> c_int {
    let clock = match base {
        TIME_UTC => time::CLOCK_REALTIME,
        TIME_MONOTONIC => time::CLOCK_MONOTONIC,
        _ => return 0,
    };
    let Some(ns) = time::clock_ns(clock) else { return 0 };
    if ts.is_null() {
        return 0;
    }
    // SAFETY: kontrakt C.
    unsafe { *ts = time::to_timespec(ns) };
    base
}

// --- lldiv (C99; libc++ <cstdlib> ho vola primo) ----------------------------

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct lldiv_t {
    pub quot: i64,
    pub rem: i64,
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub extern "C" fn lldiv(a: i64, b: i64) -> lldiv_t {
    lldiv_t { quot: a / b, rem: a % b }
}

// --- konec procesu: __cxa_atexit, atexit, thread_local -------------------
//
// Jeden seznam pro `atexit` i `__cxa_atexit` (destruktory statickych objektu
// C++): C i C++ chteji jejich spolecne obracene poradi. Seznam projde jedna
// funkce v `osrt::at_exit`, zaregistrovana az po vyprazdneni proudu stdio
// (to tedy bezi az po vsech destruktorech). Pred nim destruktory
// `thread_local` vlakna, ktere vola `exit` (jako glibc).

/// Funkce pro konec: (funkce, argument, modul `__dso_handle`).
#[derive(Clone, Copy)]
struct ExitFn {
    f: unsafe extern "C" fn(*mut c_void),
    arg: usize,
    dso: usize,
}

static EXIT_FNS: std::sync::Mutex<Vec<ExitFn>> = std::sync::Mutex::new(Vec::new());

/// Zaregistruje beh seznamu v `osrt::at_exit` (jednou, po stdio).
fn ensure_exit_hook() {
    #[cfg(target_os = "magnumopus")]
    {
        static ONCE: std::sync::Once = std::sync::Once::new();
        ONCE.call_once(|| {
            crate::stdio::ensure_exit_hook();
            std::os::magnumopus::rt::at_exit(run_at_exit);
        });
    }
}

#[cfg_attr(not(target_os = "magnumopus"), allow(dead_code))]
extern "C" fn run_at_exit() {
    run_thread_dtors();
    finalize(None);
}

/// Spusti a odebere funkce modulu `dso` (`None` = vsechny) od posledni
/// zaregistrovane; funkce smi registrovat dalsi (probehnou take).
pub fn finalize(dso: Option<usize>) {
    loop {
        let next = {
            let mut l = EXIT_FNS.lock().unwrap_or_else(|e| e.into_inner());
            let pos = l.iter().rposition(|e| dso.is_none_or(|d| e.dso == d));
            pos.map(|i| l.remove(i))
        };
        let Some(e) = next else { break };
        // SAFETY: funkce a argument od volajiciho __cxa_atexit / atexit;
        // zamek se pri volani nedrzi.
        unsafe { (e.f)(e.arg as *mut c_void) };
    }
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn __cxa_atexit(f: unsafe extern "C" fn(*mut c_void), arg: *mut c_void, dso: *mut c_void) -> c_int {
    ensure_exit_hook();
    EXIT_FNS.lock().unwrap_or_else(|e| e.into_inner()).push(ExitFn { f, arg: arg as usize, dso: dso as usize });
    0
}

/// Spusti destruktory modulu (`dlclose` knihovny v C++); NULL = vsechny.
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn __cxa_finalize(dso: *mut c_void) {
    finalize(if dso.is_null() { None } else { Some(dso as usize) });
}

unsafe extern "C" fn call_plain(arg: *mut c_void) {
    // SAFETY: `arg` je funkce `extern "C" fn()` z `atexit_fn`.
    let f: extern "C" fn() = unsafe { core::mem::transmute(arg) };
    f();
}

/// `atexit` (C): do spolecneho seznamu s `__cxa_atexit`.
pub fn atexit_fn(f: extern "C" fn()) -> c_int {
    // SAFETY: call_plain prevede argument zpet na stejny typ funkce.
    unsafe { __cxa_atexit(call_plain, f as *mut c_void, core::ptr::null_mut()) }
}

/// `__dso_handle` statickeho programu (adresa urcuje modul pro
/// `__cxa_atexit`). Program PIE ma vlastni skryty v `crt1.o`, sdilene
/// knihovny v `crtdso.o` (sysroot C).
#[cfg(all(target_os = "magnumopus", not(target_env = "dyn")))]
#[unsafe(no_mangle)]
pub static __dso_handle: u8 = 0;

std::thread_local! {
    /// Destruktory `thread_local` objektu C++ aktualniho vlakna.
    static THREAD_DTORS: core::cell::RefCell<Vec<(unsafe extern "C" fn(*mut c_void), usize)>> =
        const { core::cell::RefCell::new(Vec::new()) };
}

/// `__cxa_thread_atexit_impl` (glibc ABI, libc++abi ji vola pro
/// `thread_local` s destruktorem). Modul `dso` se nesleduje: knihovna
/// s `thread_local` se nesmi odebrat (`dlclose`), dokud vlakno bezi.
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn __cxa_thread_atexit_impl(
    f: unsafe extern "C" fn(*mut c_void),
    obj: *mut c_void,
    _dso: *mut c_void,
) -> c_int {
    ensure_exit_hook();
    THREAD_DTORS.with(|d| d.borrow_mut().push((f, obj as usize)));
    0
}

/// Destruktory `thread_local` aktualniho vlakna v obracenem poradi (konec
/// vlakna z `pthread_create`, `exit`); smi registrovat dalsi.
pub fn run_thread_dtors() {
    loop {
        let Some((f, obj)) = THREAD_DTORS.with(|d| d.borrow_mut().pop()) else { break };
        // SAFETY: destruktor a objekt od libc++abi.
        unsafe { f(obj as *mut c_void) };
    }
}
