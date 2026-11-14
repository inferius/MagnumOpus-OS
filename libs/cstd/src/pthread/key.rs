//! Klice vlaken (`pthread_key_create`, `pthread_getspecific`, ...)
//! a uklid pri zruseni (`pthread_cleanup_push`/`pop`).
//!
//! Klicu je nejvic 1024 (`PTHREAD_KEYS_MAX`); destruktory se volaji pri
//! konci vlakna z `pthread_create` (navrat, `pthread_exit`, zruseni),
//! nejvys 4 kola (`PTHREAD_DESTRUCTOR_ITERATIONS`). Hlavni vlakno je
//! nevola (proces konci).

use core::ffi::c_void;
use std::cell::RefCell;
use std::sync::Mutex;

use crate::{c_int, errno};

pub const KEYS_MAX: usize = 1024;
pub const DESTRUCTOR_ITERATIONS: usize = 4;

type Dtor = Option<unsafe extern "C" fn(*mut c_void)>;

/// Klic: (pouzity, generace, destruktor). Generace odlisi znovu
/// pouzite cislo klice od hodnot stareho klice ve vlaknech.
static KEYS: Mutex<Vec<(bool, u32, Dtor)>> = Mutex::new(Vec::new());

thread_local! {
    /// Hodnoty vlakna: (generace klice, hodnota).
    static VALUES: RefCell<Vec<(u32, usize)>> = const { RefCell::new(Vec::new()) };
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn pthread_key_create(key: *mut u32, dtor: Dtor) -> c_int {
    if key.is_null() {
        return errno::EINVAL;
    }
    let mut k = KEYS.lock().unwrap_or_else(|e| e.into_inner());
    let i = match k.iter().position(|e| !e.0) {
        Some(i) => i,
        None if k.len() < KEYS_MAX => {
            k.push((false, 0, None));
            k.len() - 1
        }
        None => return errno::EAGAIN,
    };
    let generation_no = k[i].1.wrapping_add(1);
    k[i] = (true, generation_no, dtor);
    // SAFETY: kontrakt C.
    unsafe { *key = i as u32 };
    0
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub extern "C" fn pthread_key_delete(key: u32) -> c_int {
    let mut k = KEYS.lock().unwrap_or_else(|e| e.into_inner());
    match k.get_mut(key as usize) {
        Some(e) if e.0 => {
            // Hodnoty ve vlaknech se nemazou (POSIX), destruktor uz ne.
            e.0 = false;
            e.2 = None;
            0
        }
        _ => errno::EINVAL,
    }
}

fn generation(key: u32) -> Option<u32> {
    let k = KEYS.lock().unwrap_or_else(|e| e.into_inner());
    k.get(key as usize).filter(|e| e.0).map(|e| e.1)
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub extern "C" fn pthread_getspecific(key: u32) -> *mut c_void {
    let Some(generation_no) = generation(key) else { return core::ptr::null_mut() };
    VALUES.with(|v| {
        v.borrow().get(key as usize).filter(|e| e.0 == generation_no).map_or(core::ptr::null_mut(), |e| e.1 as *mut c_void)
    })
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub extern "C" fn pthread_setspecific(key: u32, value: *const c_void) -> c_int {
    let Some(generation_no) = generation(key) else { return errno::EINVAL };
    VALUES.with(|v| {
        let mut v = v.borrow_mut();
        let i = key as usize;
        if v.len() <= i {
            v.resize(i + 1, (0, 0));
        }
        v[i] = (generation_no, value as usize);
    });
    0
}

/// Destruktory klicu aktualniho vlakna (konec vlakna z `pthread_create`).
pub fn run_destructors() {
    run_cleanup();
    for _ in 0..DESTRUCTOR_ITERATIONS {
        let mut any = false;
        let n = VALUES.with(|v| v.borrow().len());
        for i in 0..n {
            let (generation_no, val) = VALUES.with(|v| v.borrow()[i]);
            if val == 0 {
                continue;
            }
            let dtor = {
                let k = KEYS.lock().unwrap_or_else(|e| e.into_inner());
                k.get(i).filter(|e| e.0 && e.1 == generation_no).and_then(|e| e.2)
            };
            // POSIX: hodnota se pred volanim vynuluje.
            VALUES.with(|v| v.borrow_mut()[i].1 = 0);
            if let Some(d) = dtor {
                any = true;
                // SAFETY: destruktor od volajiciho pthread_key_create.
                unsafe { d(val as *mut c_void) };
            }
        }
        if !any {
            break;
        }
    }
}

/// Zaznam `pthread_cleanup_push` (makro v `pthread.h` ho ma na zasobniku).
#[repr(C)]
pub struct cleanup {
    f: Option<unsafe extern "C" fn(*mut c_void)>,
    arg: *mut c_void,
    prev: *mut cleanup,
}

thread_local! {
    static CLEANUP: core::cell::Cell<*mut cleanup> = const { core::cell::Cell::new(core::ptr::null_mut()) };
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn __cstd_cleanup_push(c: *mut cleanup, f: unsafe extern "C" fn(*mut c_void), arg: *mut c_void) {
    let prev = CLEANUP.with(|x| x.get());
    // SAFETY: zaznam na zasobniku volajiciho (makro).
    unsafe { c.write(cleanup { f: Some(f), arg, prev }) };
    CLEANUP.with(|x| x.set(c));
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn __cstd_cleanup_pop(c: *mut cleanup, execute: c_int) {
    // SAFETY: zaznam z __cstd_cleanup_push (vrchol seznamu).
    let rec = unsafe { &*c };
    CLEANUP.with(|x| x.set(rec.prev));
    if execute != 0
        && let Some(f) = rec.f
    {
        // SAFETY: funkce od volajiciho.
        unsafe { f(rec.arg) };
    }
}

/// Uklid pri ukonceni vlakna (`pthread_exit`, zruseni): vsechny zaznamy
/// od vrcholu.
fn run_cleanup() {
    loop {
        let c = CLEANUP.with(|x| x.get());
        if c.is_null() {
            break;
        }
        // SAFETY: zaznamy jsou na zasobniku vlakna, ktery jeste existuje
        // (vlakno konci syscallem, zasobnik se nerozbaluje).
        unsafe { __cstd_cleanup_pop(c, 1) };
    }
}
