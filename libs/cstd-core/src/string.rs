//! `string.h`: retezce zakoncene nulou.
//!
//! Vsechny funkce jsou `unsafe extern "C"`: ukazatele dodava kod v C
//! a za jejich platnost (a zakonceni nulou) odpovida on, stejne jako
//! u kazde libc. Funkce s delkou (`strncpy`, `strnlen`, ...) nectou ani
//! nepisou za `n` bajtu.

use crate::ctypes::{c_char, c_int, size_t};

/// Bajt na adrese (pomocna funkce, aby kod nize nebyl plny `unsafe` bloku).
#[inline(always)]
unsafe fn at(p: *const c_char, i: usize) -> u8 {
    // SAFETY: predano volajicim (platny retezec / rozsah).
    unsafe { *p.add(i) }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn strlen(s: *const c_char) -> size_t {
    let mut n = 0;
    // SAFETY: `s` je retezec zakonceny nulou (kontrakt C).
    while unsafe { at(s, n) } != 0 {
        n += 1;
    }
    n
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn strnlen(s: *const c_char, max: size_t) -> size_t {
    let mut n = 0;
    // SAFETY: cte nejvys `max` bajtu (kontrakt C).
    while n < max && unsafe { at(s, n) } != 0 {
        n += 1;
    }
    n
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn strcmp(a: *const c_char, b: *const c_char) -> c_int {
    // SAFETY: oba retezce zakonceny nulou.
    unsafe { strncmp(a, b, usize::MAX) }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn strncmp(a: *const c_char, b: *const c_char, n: size_t) -> c_int {
    let mut i = 0;
    while i < n {
        // SAFETY: konec obou retezcu je nejpozdeji na nule.
        let (x, y) = unsafe { (at(a, i), at(b, i)) };
        if x != y {
            return x as c_int - y as c_int;
        }
        if x == 0 {
            return 0;
        }
        i += 1;
    }
    0
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn strcpy(dst: *mut c_char, src: *const c_char) -> *mut c_char {
    let mut i = 0;
    loop {
        // SAFETY: `dst` je dost velky pro `src` vcetne nuly (kontrakt C).
        let c = unsafe { at(src, i) };
        unsafe { *dst.add(i) = c };
        if c == 0 {
            return dst;
        }
        i += 1;
    }
}

/// Jako C: zkopiruje nejvys `n` bajtu, zbytek doplni nulami; kdyz je `src`
/// delsi, vysledek NENI zakonceny nulou.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn strncpy(dst: *mut c_char, src: *const c_char, n: size_t) -> *mut c_char {
    let mut i = 0;
    while i < n {
        // SAFETY: `dst` ma aspon `n` bajtu.
        let c = unsafe { at(src, i) };
        unsafe { *dst.add(i) = c };
        i += 1;
        if c == 0 {
            break;
        }
    }
    while i < n {
        // SAFETY: dtto.
        unsafe { *dst.add(i) = 0 };
        i += 1;
    }
    dst
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn strcat(dst: *mut c_char, src: *const c_char) -> *mut c_char {
    // SAFETY: `dst` pojme oba retezce (kontrakt C).
    unsafe {
        strcpy(dst.add(strlen(dst)), src);
    }
    dst
}

/// Pripoji nejvys `n` znaku `src` a vzdy zakonci nulou (cil musi mit misto
/// pro `strlen(dst) + n + 1` bajtu).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn strncat(dst: *mut c_char, src: *const c_char, n: size_t) -> *mut c_char {
    // SAFETY: kontrakt C (viz komentar).
    unsafe {
        let start = dst.add(strlen(dst));
        let len = strnlen(src, n);
        let mut i = 0;
        while i < len {
            *start.add(i) = at(src, i);
            i += 1;
        }
        *start.add(len) = 0;
    }
    dst
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn strchr(s: *const c_char, c: c_int) -> *mut c_char {
    let c = c as u8;
    let mut i = 0;
    loop {
        // SAFETY: retezec zakonceny nulou.
        let x = unsafe { at(s, i) };
        if x == c {
            return s.wrapping_add(i) as *mut c_char;
        }
        if x == 0 {
            return core::ptr::null_mut();
        }
        i += 1;
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn strrchr(s: *const c_char, c: c_int) -> *mut c_char {
    let c = c as u8;
    let mut last = core::ptr::null_mut();
    let mut i = 0;
    loop {
        // SAFETY: retezec zakonceny nulou.
        let x = unsafe { at(s, i) };
        if x == c {
            last = s.wrapping_add(i) as *mut c_char;
        }
        if x == 0 {
            return last;
        }
        i += 1;
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn strstr(hay: *const c_char, needle: *const c_char) -> *mut c_char {
    // SAFETY: oba retezce zakonceny nulou.
    let n = unsafe { strlen(needle) };
    if n == 0 {
        return hay as *mut c_char;
    }
    let mut i = 0;
    // SAFETY: porovnani konci nejpozdeji na nule v `hay`.
    while unsafe { at(hay, i) } != 0 {
        if unsafe { strncmp(hay.add(i), needle, n) } == 0 {
            return hay.wrapping_add(i) as *mut c_char;
        }
        i += 1;
    }
    core::ptr::null_mut()
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn memchr(s: *const u8, c: c_int, n: size_t) -> *mut u8 {
    let c = c as u8;
    let mut i = 0;
    while i < n {
        // SAFETY: rozsah `n` bajtu (kontrakt C).
        if unsafe { *s.add(i) } == c {
            return s.wrapping_add(i) as *mut u8;
        }
        i += 1;
    }
    core::ptr::null_mut()
}
