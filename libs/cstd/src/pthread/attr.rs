//! Atributy vlakna (`pthread_attr_t`, 64 B jako Linux): velikost
//! a vlastni zasobnik, odpojeni, ochranna oblast, rozsah, planovani.
//!
//! Vlastni zasobnik (`pthread_attr_setstack`) se opravdu pouzije: vlakno
//! bezi v pameti volajiciho (`libos::thread::spawn_on_stack`), ktera musi
//! zit do konce vlakna; ochrannou oblast pod nim si zajisti volajici
//! (`guardsize` se pak jako na Linuxu ignoruje). Bez vlastniho zasobniku ma
//! vlakno vzdy ochrannou oblast od `libos` (`vm::STACK_GUARD`), `guardsize`
//! se jen ulozi.

use core::ffi::c_void;

use super::{PTHREAD_CREATE_DETACHED, PTHREAD_CREATE_JOINABLE, SCHED_OTHER, STACK_DEFAULT, STACK_MIN, sched_param};
use crate::{c_int, errno};

pub const PTHREAD_SCOPE_SYSTEM: c_int = 0;
pub const PTHREAD_SCOPE_PROCESS: c_int = 1;
pub const PTHREAD_INHERIT_SCHED: c_int = 0;
pub const PTHREAD_EXPLICIT_SCHED: c_int = 1;
/// Vychozi `guardsize` (jedna stranka jako glibc).
pub const GUARD_DEFAULT: usize = 4096;

#[repr(C)]
pub struct pthread_attr_t {
    pub(super) stack: usize,
    pub(super) detach: c_int,
    scope: c_int,
    /// Nejnizsi adresa vlastniho zasobniku (0 = zasobnik od systemu).
    addr: usize,
    guard: usize,
    inherit: c_int,
    _rest: [u8; 28],
}

impl pthread_attr_t {
    /// Konec vlastniho zasobniku (nejvyssi adresa + 1), je-li zadany.
    pub(super) fn own_stack(&self) -> Option<usize> {
        (self.addr != 0).then(|| self.addr + self.stack)
    }
}

/// # Safety
///
/// `a` ukazuje na pamet pro `pthread_attr_t`.
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn pthread_attr_init(a: *mut pthread_attr_t) -> c_int {
    let v = pthread_attr_t {
        stack: STACK_DEFAULT,
        detach: PTHREAD_CREATE_JOINABLE,
        scope: PTHREAD_SCOPE_SYSTEM,
        addr: 0,
        guard: GUARD_DEFAULT,
        inherit: PTHREAD_INHERIT_SCHED,
        _rest: [0; 28],
    };
    // SAFETY: kontrakt C.
    unsafe { a.write(v) };
    0
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub extern "C" fn pthread_attr_destroy(_a: *mut pthread_attr_t) -> c_int {
    0
}

/// # Safety
///
/// `a` je inicializovany atribut.
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn pthread_attr_setstacksize(a: *mut pthread_attr_t, size: usize) -> c_int {
    if size < STACK_MIN {
        return errno::EINVAL;
    }
    // SAFETY: kontrakt C.
    unsafe { (*a).stack = size };
    0
}

/// # Safety
///
/// `a` je inicializovany atribut, `size` platny ukazatel.
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn pthread_attr_getstacksize(a: *const pthread_attr_t, size: *mut usize) -> c_int {
    // SAFETY: kontrakt C.
    unsafe { *size = (*a).stack };
    0
}

/// Vlastni zasobnik `[addr, addr + size)`: adresa zarovnana na 16 B,
/// velikost aspon `PTHREAD_STACK_MIN`.
///
/// # Safety
///
/// `a` je inicializovany atribut; pamet zasobniku zije do konce vlakna.
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn pthread_attr_setstack(a: *mut pthread_attr_t, addr: *mut c_void, size: usize) -> c_int {
    let base = addr as usize;
    if size < STACK_MIN || base == 0 || base % 16 != 0 || base.checked_add(size).is_none() {
        return errno::EINVAL;
    }
    // SAFETY: kontrakt C.
    unsafe {
        (*a).addr = base;
        (*a).stack = size;
    }
    0
}

/// # Safety
///
/// `a` je inicializovany atribut, vystupy platne ukazatele.
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn pthread_attr_getstack(a: *const pthread_attr_t, addr: *mut *mut c_void, size: *mut usize) -> c_int {
    // SAFETY: kontrakt C.
    unsafe {
        *addr = (*a).addr as *mut c_void;
        *size = (*a).stack;
    }
    0
}

/// # Safety
///
/// `a` je inicializovany atribut.
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn pthread_attr_setguardsize(a: *mut pthread_attr_t, size: usize) -> c_int {
    // SAFETY: kontrakt C.
    unsafe { (*a).guard = size };
    0
}

/// # Safety
///
/// `a` je inicializovany atribut, `size` platny ukazatel.
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn pthread_attr_getguardsize(a: *const pthread_attr_t, size: *mut usize) -> c_int {
    // SAFETY: kontrakt C.
    unsafe { *size = (*a).guard };
    0
}

/// # Safety
///
/// `a` je inicializovany atribut.
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn pthread_attr_setdetachstate(a: *mut pthread_attr_t, s: c_int) -> c_int {
    if s != PTHREAD_CREATE_JOINABLE && s != PTHREAD_CREATE_DETACHED {
        return errno::EINVAL;
    }
    // SAFETY: kontrakt C.
    unsafe { (*a).detach = s };
    0
}

/// # Safety
///
/// `a` je inicializovany atribut, `s` platny ukazatel.
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn pthread_attr_getdetachstate(a: *const pthread_attr_t, s: *mut c_int) -> c_int {
    // SAFETY: kontrakt C.
    unsafe { *s = (*a).detach };
    0
}

/// Vlakna planuje jadro (1:1): jen `PTHREAD_SCOPE_SYSTEM`, rozsah procesu
/// `ENOTSUP` (jako Linux).
///
/// # Safety
///
/// `a` je inicializovany atribut.
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn pthread_attr_setscope(a: *mut pthread_attr_t, scope: c_int) -> c_int {
    match scope {
        PTHREAD_SCOPE_SYSTEM => {
            // SAFETY: kontrakt C.
            unsafe { (*a).scope = scope };
            0
        }
        PTHREAD_SCOPE_PROCESS => errno::ENOTSUP,
        _ => errno::EINVAL,
    }
}

/// # Safety
///
/// `a` je inicializovany atribut, `scope` platny ukazatel.
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn pthread_attr_getscope(a: *const pthread_attr_t, scope: *mut c_int) -> c_int {
    // SAFETY: kontrakt C.
    unsafe { *scope = (*a).scope };
    0
}

/// # Safety
///
/// `a` je inicializovany atribut.
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn pthread_attr_setinheritsched(a: *mut pthread_attr_t, v: c_int) -> c_int {
    if v != PTHREAD_INHERIT_SCHED && v != PTHREAD_EXPLICIT_SCHED {
        return errno::EINVAL;
    }
    // SAFETY: kontrakt C.
    unsafe { (*a).inherit = v };
    0
}

/// # Safety
///
/// `a` je inicializovany atribut, `v` platny ukazatel.
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn pthread_attr_getinheritsched(a: *const pthread_attr_t, v: *mut c_int) -> c_int {
    // SAFETY: kontrakt C.
    unsafe { *v = (*a).inherit };
    0
}

/// Jako `pthread_setschedparam`: jen `SCHED_OTHER` s prioritou 0
/// (`sched.h`), jina znama politika `ENOTSUP`.
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub extern "C" fn pthread_attr_setschedpolicy(_a: *mut pthread_attr_t, policy: c_int) -> c_int {
    match policy {
        SCHED_OTHER => 0,
        p if super::known_policy(p) => errno::ENOTSUP,
        _ => errno::EINVAL,
    }
}

/// # Safety
///
/// `policy` je platny ukazatel.
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn pthread_attr_getschedpolicy(_a: *const pthread_attr_t, policy: *mut c_int) -> c_int {
    // SAFETY: kontrakt C.
    unsafe { *policy = SCHED_OTHER };
    0
}

/// # Safety
///
/// `p` je NULL nebo platny ukazatel.
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn pthread_attr_setschedparam(_a: *mut pthread_attr_t, p: *const sched_param) -> c_int {
    // SAFETY: kontrakt C.
    match unsafe { p.as_ref() } {
        Some(p) if p.sched_priority == 0 => 0,
        _ => errno::EINVAL,
    }
}

/// # Safety
///
/// `p` je platny ukazatel.
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn pthread_attr_getschedparam(_a: *const pthread_attr_t, p: *mut sched_param) -> c_int {
    // SAFETY: kontrakt C.
    unsafe { (*p).sched_priority = 0 };
    0
}
