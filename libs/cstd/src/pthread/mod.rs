//! `pthread.h`: vlakna nad vlakny osrt (TLS jako std, takze `errno`,
//! `_Thread_local` i `thread_local!` funguji i ve vlaknech z C),
//! synchronizace nad futexem (`sync.rs`), klice vlaken (`key.rs`).
//!
//! Cil ma `panic=abort`: `pthread_exit` a zruseni vlakno ukonci syscallem
//! (`sys::exit_thread`) bez rozbaleni zasobniku; destruktory klicu
//! probehnou predtim, destruktory `thread_local!` v Rustu ne.
//!
//! `pthread_t` je ukazatel na popis vlakna (`Info`), hlavni vlakno ho
//! dostane pri prvnim `pthread_self`. Zruseni vlakna je jen odlozene
//! (`pthread_cancel` + body zruseni `pthread_testcancel`, cekani na
//! podminku, `pthread_join`), asynchronni ne (rozhodnuti 5).

mod attr;
mod key;
mod sync;

pub use attr::*;
pub use key::*;
pub use sync::*;

use core::ffi::c_void;
use std::cell::Cell;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use crate::sys;
use crate::{c_int, errno};

pub type pthread_t = *mut Info;
type Start = unsafe extern "C" fn(*mut c_void) -> *mut c_void;

/// Hodnota `PTHREAD_CANCELED` (vysledek zruseneho vlakna).
pub const CANCELED: *mut c_void = usize::MAX as *mut c_void;

pub const PTHREAD_CREATE_JOINABLE: c_int = 0;
pub const PTHREAD_CREATE_DETACHED: c_int = 1;
pub const PTHREAD_CANCEL_ENABLE: c_int = 0;
pub const PTHREAD_CANCEL_DISABLE: c_int = 1;
pub const PTHREAD_CANCEL_DEFERRED: c_int = 0;
pub const PTHREAD_CANCEL_ASYNCHRONOUS: c_int = 1;

/// Nejmensi zasobnik (`PTHREAD_STACK_MIN`) a vychozi velikost.
pub const STACK_MIN: usize = 16 * 1024;
pub const STACK_DEFAULT: usize = 2 * 1024 * 1024;

/// Popis vlakna (`pthread_t`).
pub struct Info {
    join: Mutex<Option<sys::Thread>>,
    detached: AtomicBool,
    /// Pozadavek na zruseni a stav (`PTHREAD_CANCEL_*`).
    cancel_requested: AtomicBool,
    cancel_disabled: AtomicBool,
    /// Futex probouzeni vlakna pri zruseni (cekani na podminku).
    pub(crate) wake: AtomicU32,
    /// Vysledek `pthread_exit` (cte `join` po skonceni).
    result: Mutex<usize>,
}

impl Info {
    fn new() -> Self {
        Info {
            join: Mutex::new(None),
            detached: AtomicBool::new(false),
            cancel_requested: AtomicBool::new(false),
            cancel_disabled: AtomicBool::new(false),
            wake: AtomicU32::new(0),
            result: Mutex::new(0),
        }
    }
}

thread_local! {
    static SELF: Cell<*mut Info> = const { Cell::new(core::ptr::null_mut()) };
    /// Vlakno vzniklo pres `pthread_create` (ne hlavni ani cizi).
    static SELF_CREATED: Cell<bool> = const { Cell::new(false) };
}

/// Popis aktualniho vlakna (hlavni a cizi vlakna ho dostanou poprve tady;
/// zustane do konce procesu).
pub fn current() -> *mut Info {
    SELF.with(|c| {
        if c.get().is_null() {
            c.set(Box::into_raw(Box::new(Info::new())));
        }
        c.get()
    })
}

/// Nenulove cislo vlakna (vlastnik mutexu).
pub fn thread_id() -> u32 {
    // Adresa popisu je unikatni po dobu zivota vlakna; spodni bity
    // zarovnani se zahodi, 0 nikdy.
    let p = current() as usize;
    ((p >> 4) as u32) | 1
}

/// Ukazatel, ktery smi do jineho vlakna (argument a popis pro C).
struct SendPtr(usize);
// SAFETY: jen prenos adresy; synchronizaci zajisti volajici (pthread).
unsafe impl Send for SendPtr {}

/// Konec vlakna s vysledkem: destruktory klicu, ulozeni vysledku.
fn finish(value: usize) {
    // Jako glibc: nejdriv thread_local C++, pak destruktory klicu.
    crate::cxxsup::run_thread_dtors();
    key::run_destructors();
    // SAFETY: popis aktualniho vlakna zije, dokud ho neuvolni join.
    let info = unsafe { &*current() };
    *info.result.lock().unwrap_or_else(|e| e.into_inner()) = value;
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn pthread_create(
    out: *mut pthread_t,
    attr: *const pthread_attr_t,
    start: Start,
    arg: *mut c_void,
) -> c_int {
    if out.is_null() {
        return errno::EINVAL;
    }
    // SAFETY: atributy jsou NULL nebo inicializovane (kontrakt C).
    let (stack, own, detached) = match unsafe { attr.as_ref() } {
        Some(a) => (a.stack.max(STACK_MIN), a.own_stack(), a.detach == PTHREAD_CREATE_DETACHED),
        None => (STACK_DEFAULT, None, false),
    };
    let info = Box::into_raw(Box::new(Info::new()));
    // SAFETY: cerstvy popis.
    unsafe { (*info).detached.store(detached, Ordering::Release) };
    // Zamek join drzime do ulozeni handlu: vlakno, ktere hned skonci
    // a je odpojene, nesmi popis uvolnit driv.
    // SAFETY: popis zije do join.
    let mut slot = unsafe { &*info }.join.lock().unwrap_or_else(|e| e.into_inner());
    let (pinfo, parg) = (SendPtr(info as usize), SendPtr(arg as usize));
    let body: sys::ThreadFn = Box::new(move || {
        let (pinfo, parg) = (pinfo, parg);
        SELF.with(|c| c.set(pinfo.0 as *mut Info));
        SELF_CREATED.with(|c| c.set(true));
        // SAFETY: funkce a argument od volajiciho pthread_create.
        let value = unsafe { start(parg.0 as *mut c_void) } as usize;
        finish(value);
    });
    let spawned = match own {
        Some(top) => sys::spawn_on(top, body),
        None => sys::spawn(stack, body),
    };
    match spawned {
        Ok(t) => {
            if !detached {
                *slot = Some(t);
            }
            drop(slot);
            // SAFETY: `out` je platny (kontrakt C).
            unsafe { *out = info };
            0
        }
        Err(e) => {
            drop(slot);
            // SAFETY: vlakno nevzniklo, popis nikdo nepouziva.
            drop(unsafe { Box::from_raw(info) });
            e
        }
    }
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn pthread_join(t: pthread_t, out: *mut *mut c_void) -> c_int {
    if t.is_null() {
        return errno::ESRCH;
    }
    if t == current() {
        return errno::EDEADLK;
    }
    testcancel();
    // SAFETY: `t` z pthread_create (kontrakt C).
    let info = unsafe { &*t };
    if info.detached.load(Ordering::Acquire) {
        return errno::EINVAL;
    }
    let Some(h) = info.join.lock().unwrap_or_else(|e| e.into_inner()).take() else { return errno::EINVAL };
    if let Err(e) = h.join() {
        return e;
    }
    let value = *info.result.lock().unwrap_or_else(|e| e.into_inner());
    if !out.is_null() {
        // SAFETY: kontrakt C.
        unsafe { *out = value as *mut c_void };
    }
    // SAFETY: vlakno skoncilo a bylo pripojene: popis uz nikdo nepouzije.
    drop(unsafe { Box::from_raw(t) });
    0
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn pthread_detach(t: pthread_t) -> c_int {
    if t.is_null() {
        return errno::ESRCH;
    }
    // SAFETY: `t` z pthread_create.
    let info = unsafe { &*t };
    if info.detached.swap(true, Ordering::AcqRel) {
        return errno::EINVAL;
    }
    // Odpojene vlakno: popis zustane do konce procesu (vlakno ho muze cist).
    drop(info.join.lock().unwrap_or_else(|e| e.into_inner()).take());
    0
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub extern "C" fn pthread_self() -> pthread_t {
    current()
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub extern "C" fn pthread_equal(a: pthread_t, b: pthread_t) -> c_int {
    c_int::from(a == b)
}

/// Ukonci vlakno s vysledkem (bez rozbaleni zasobniku). Hlavni vlakno:
/// ostatni vlakna bezi dal az do jejich konce by vyzadovalo jejich
/// sledovani; tady konci cely proces kodem 0.
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub extern "C" fn pthread_exit(value: *mut c_void) -> ! {
    if !SELF_CREATED.with(Cell::get) {
        crate::stdlib::exit(0);
    }
    finish(value as usize);
    sys::exit_thread()
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub extern "C" fn sched_yield() -> c_int {
    std::thread::yield_now();
    0
}

/// Politiky planovani (`sched.h`). Aplikace maji jen `SCHED_OTHER`
/// s prioritou 0: planovac zatim realny cas aplikacim nedava.
pub const SCHED_OTHER: c_int = 0;
pub const SCHED_FIFO: c_int = 1;
pub const SCHED_RR: c_int = 2;

/// `struct sched_param`.
#[repr(C)]
pub struct sched_param {
    pub sched_priority: c_int,
}

fn known_policy(policy: c_int) -> bool {
    matches!(policy, SCHED_OTHER | SCHED_FIFO | SCHED_RR)
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub extern "C" fn sched_get_priority_min(policy: c_int) -> c_int {
    if known_policy(policy) {
        0
    } else {
        crate::errno::set(crate::errno::EINVAL);
        -1
    }
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub extern "C" fn sched_get_priority_max(policy: c_int) -> c_int {
    if known_policy(policy) {
        0
    } else {
        crate::errno::set(crate::errno::EINVAL);
        -1
    }
}

/// `pthread_getschedparam`: vzdy `SCHED_OTHER`, priorita 0.
///
/// # Safety
///
/// `policy` a `param` jsou platne ukazatele nebo NULL.
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn pthread_getschedparam(_t: pthread_t, policy: *mut c_int, param: *mut sched_param) -> c_int {
    // SAFETY: platne nebo NULL podle smlouvy.
    unsafe {
        if let Some(p) = policy.as_mut() {
            *p = SCHED_OTHER;
        }
        if let Some(p) = param.as_mut() {
            p.sched_priority = 0;
        }
    }
    0
}

/// `pthread_setschedparam`: `SCHED_OTHER` s prioritou 0 projde, realny cas
/// = `EPERM` (jako Linux bez opravneni), nesmysl = `EINVAL`. Vraci kod
/// chyby (ne `errno`), jako vsechny funkce `pthread_*`.
///
/// # Safety
///
/// `param` je platny ukazatel nebo NULL.
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn pthread_setschedparam(_t: pthread_t, policy: c_int, param: *const sched_param) -> c_int {
    // SAFETY: platny nebo NULL podle smlouvy.
    let prio = unsafe { param.as_ref() }.map(|p| p.sched_priority);
    sched_set_check(policy, prio)
}

/// Pravidlo `pthread_setschedparam` (ciste, test na hostiteli).
pub fn sched_set_check(policy: c_int, prio: Option<c_int>) -> c_int {
    match (policy, prio) {
        (_, None) => crate::errno::EINVAL,
        (SCHED_OTHER, Some(0)) => 0,
        (SCHED_FIFO | SCHED_RR, Some(_)) => crate::errno::EPERM,
        _ => crate::errno::EINVAL,
    }
}

/// Bod zruseni: zrusene vlakno skonci s `PTHREAD_CANCELED`.
pub fn testcancel() {
    let me = current();
    // SAFETY: popis aktualniho vlakna.
    let i = unsafe { &*me };
    if i.cancel_requested.load(Ordering::Acquire) && !i.cancel_disabled.load(Ordering::Acquire) {
        pthread_exit(CANCELED);
    }
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub extern "C" fn pthread_testcancel() {
    testcancel();
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn pthread_cancel(t: pthread_t) -> c_int {
    if t.is_null() {
        return errno::ESRCH;
    }
    // SAFETY: `t` z pthread_create.
    let info = unsafe { &*t };
    info.cancel_requested.store(true, Ordering::Release);
    // Probudit, pokud ceka na podminku (sync.rs kontroluje zruseni).
    info.wake.fetch_add(1, Ordering::AcqRel);
    sync::wake_all(&info.wake);
    0
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn pthread_setcancelstate(state: c_int, old: *mut c_int) -> c_int {
    if state != PTHREAD_CANCEL_ENABLE && state != PTHREAD_CANCEL_DISABLE {
        return errno::EINVAL;
    }
    // SAFETY: popis aktualniho vlakna.
    let i = unsafe { &*current() };
    let prev = i.cancel_disabled.swap(state == PTHREAD_CANCEL_DISABLE, Ordering::AcqRel);
    if !old.is_null() {
        // SAFETY: kontrakt C.
        unsafe { *old = if prev { PTHREAD_CANCEL_DISABLE } else { PTHREAD_CANCEL_ENABLE } };
    }
    0
}

/// Jen odlozene zruseni; `PTHREAD_CANCEL_ASYNCHRONOUS` = `ENOTSUP`.
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn pthread_setcanceltype(kind: c_int, old: *mut c_int) -> c_int {
    if !old.is_null() {
        // SAFETY: kontrakt C.
        unsafe { *old = PTHREAD_CANCEL_DEFERRED };
    }
    match kind {
        PTHREAD_CANCEL_DEFERRED => 0,
        PTHREAD_CANCEL_ASYNCHRONOUS => errno::ENOTSUP,
        _ => errno::EINVAL,
    }
}

/// Je zruseni vyzadane (pro cekani v `sync.rs`)?
pub(crate) fn cancel_pending() -> bool {
    // SAFETY: popis aktualniho vlakna.
    let i = unsafe { &*current() };
    i.cancel_requested.load(Ordering::Acquire) && !i.cancel_disabled.load(Ordering::Acquire)
}
