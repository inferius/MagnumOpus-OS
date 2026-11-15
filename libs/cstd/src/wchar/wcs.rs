//! Siroke retezce: `wcs*` a `wmem*`. Porovnani podle hodnot `wchar_t`
//! (na x86_64 se znamenkem, jako glibc); razeni (`wcscoll`) zatim jako
//! locale C.

use core::cmp::Ordering;
use core::ptr;

use super::{wchar_t, wlen, wslice};
use crate::{c_int, size_t};

fn ord(o: Ordering) -> c_int {
    match o {
        Ordering::Less => -1,
        Ordering::Equal => 0,
        Ordering::Greater => 1,
    }
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn wcslen(s: *const wchar_t) -> size_t {
    // SAFETY: kontrakt C.
    unsafe { wlen(s) }
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn wcsnlen(s: *const wchar_t, n: size_t) -> size_t {
    let mut i = 0;
    // SAFETY: aspon `n` znaku nebo nula drive.
    while i < n && unsafe { *s.add(i) } != 0 {
        i += 1;
    }
    i
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn wcscpy(d: *mut wchar_t, s: *const wchar_t) -> *mut wchar_t {
    // SAFETY: kontrakt C (`d` se vejde cely `s` vcetne nuly).
    unsafe { ptr::copy(s, d, wlen(s) + 1) };
    d
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn wcpcpy(d: *mut wchar_t, s: *const wchar_t) -> *mut wchar_t {
    // SAFETY: kontrakt C.
    unsafe {
        let n = wlen(s);
        ptr::copy(s, d, n + 1);
        d.add(n)
    }
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn wcsncpy(d: *mut wchar_t, s: *const wchar_t, n: size_t) -> *mut wchar_t {
    // SAFETY: kontrakt C (`d` ma `n` znaku).
    unsafe {
        let k = wcsnlen(s, n);
        ptr::copy(s, d, k);
        for i in k..n {
            *d.add(i) = 0;
        }
    }
    d
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn wcpncpy(d: *mut wchar_t, s: *const wchar_t, n: size_t) -> *mut wchar_t {
    // SAFETY: kontrakt C.
    unsafe {
        let k = wcsnlen(s, n);
        wcsncpy(d, s, n);
        d.add(k)
    }
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn wcscat(d: *mut wchar_t, s: *const wchar_t) -> *mut wchar_t {
    // SAFETY: kontrakt C.
    unsafe { wcscpy(d.add(wlen(d)), s) };
    d
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn wcsncat(d: *mut wchar_t, s: *const wchar_t, n: size_t) -> *mut wchar_t {
    // SAFETY: kontrakt C (`d` ma misto na `n` znaku a nulu).
    unsafe {
        let end = d.add(wlen(d));
        let k = wcsnlen(s, n);
        ptr::copy(s, end, k);
        *end.add(k) = 0;
    }
    d
}

/// `wcslcpy`/`wcslcat` (BSD): vzdy zakonci nulou (je-li `n` > 0), vraci
/// delku, kterou by vysledek mel.
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn wcslcpy(d: *mut wchar_t, s: *const wchar_t, n: size_t) -> size_t {
    // SAFETY: kontrakt C.
    unsafe {
        let len = wlen(s);
        if n > 0 {
            let k = len.min(n - 1);
            ptr::copy(s, d, k);
            *d.add(k) = 0;
        }
        len
    }
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn wcslcat(d: *mut wchar_t, s: *const wchar_t, n: size_t) -> size_t {
    // SAFETY: kontrakt C.
    unsafe {
        let dl = wcsnlen(d, n);
        if dl == n {
            return n + wlen(s);
        }
        dl + wcslcpy(d.add(dl), s, n - dl)
    }
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn wcscmp(a: *const wchar_t, b: *const wchar_t) -> c_int {
    // SAFETY: kontrakt C.
    unsafe { wcsncmp(a, b, usize::MAX) }
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn wcsncmp(a: *const wchar_t, b: *const wchar_t, n: size_t) -> c_int {
    for i in 0..n {
        // SAFETY: retezce zakoncene nulou.
        let (x, y) = unsafe { (*a.add(i), *b.add(i)) };
        if x != y {
            return ord(x.cmp(&y));
        }
        if x == 0 {
            break;
        }
    }
    0
}

fn fold(c: wchar_t) -> u32 {
    uniprop::to_lower(c as u32)
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn wcsncasecmp(a: *const wchar_t, b: *const wchar_t, n: size_t) -> c_int {
    for i in 0..n {
        // SAFETY: retezce zakoncene nulou.
        let (x, y) = unsafe { (*a.add(i), *b.add(i)) };
        let (fx, fy) = (fold(x), fold(y));
        if fx != fy {
            return ord(fx.cmp(&fy));
        }
        if x == 0 {
            break;
        }
    }
    0
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn wcscasecmp(a: *const wchar_t, b: *const wchar_t) -> c_int {
    // SAFETY: kontrakt C.
    unsafe { wcsncasecmp(a, b, usize::MAX) }
}

/// `wcscoll`: razeni jako locale C (podle hodnot).
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn wcscoll(a: *const wchar_t, b: *const wchar_t) -> c_int {
    // SAFETY: kontrakt C.
    unsafe { wcscmp(a, b) }
}

/// `wcsxfrm`: kopie (locale C); vraci delku zdroje.
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn wcsxfrm(d: *mut wchar_t, s: *const wchar_t, n: size_t) -> size_t {
    // SAFETY: kontrakt C.
    unsafe {
        let len = wlen(s);
        if len < n {
            ptr::copy(s, d, len + 1);
        }
        len
    }
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn wcschr(s: *const wchar_t, c: wchar_t) -> *mut wchar_t {
    let mut p = s;
    loop {
        // SAFETY: retezec zakonceny nulou (hledana nula je jeho konec).
        let x = unsafe { *p };
        if x == c {
            return p as *mut wchar_t;
        }
        if x == 0 {
            return ptr::null_mut();
        }
        p = p.wrapping_add(1);
    }
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn wcschrnul(s: *const wchar_t, c: wchar_t) -> *mut wchar_t {
    // SAFETY: kontrakt C (retezec zakonceny nulou).
    unsafe {
        let p = wcschr(s, c);
        if p.is_null() { s.add(wlen(s)) as *mut wchar_t } else { p }
    }
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn wcsrchr(s: *const wchar_t, c: wchar_t) -> *mut wchar_t {
    // SAFETY: kontrakt C (koncova nula patri k retezci).
    let all = unsafe { core::slice::from_raw_parts(s, wlen(s) + 1) };
    all.iter().rposition(|&x| x == c).map_or(ptr::null_mut(), |i| s.wrapping_add(i) as *mut wchar_t)
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn wcsspn(s: *const wchar_t, set: *const wchar_t) -> size_t {
    // SAFETY: kontrakt C.
    let (s, set) = unsafe { (wslice(s), wslice(set)) };
    s.iter().take_while(|c| set.contains(c)).count()
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn wcscspn(s: *const wchar_t, set: *const wchar_t) -> size_t {
    // SAFETY: kontrakt C.
    let (s, set) = unsafe { (wslice(s), wslice(set)) };
    s.iter().take_while(|c| !set.contains(c)).count()
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn wcspbrk(s: *const wchar_t, set: *const wchar_t) -> *mut wchar_t {
    // SAFETY: kontrakt C.
    let (sl, setl) = unsafe { (wslice(s), wslice(set)) };
    sl.iter().position(|c| setl.contains(c)).map_or(ptr::null_mut(), |i| s.wrapping_add(i) as *mut wchar_t)
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn wcsstr(h: *const wchar_t, n: *const wchar_t) -> *mut wchar_t {
    // SAFETY: kontrakt C.
    let (hs, ns) = unsafe { (wslice(h), wslice(n)) };
    if ns.is_empty() {
        return h as *mut wchar_t;
    }
    hs.windows(ns.len()).position(|w| w == ns).map_or(ptr::null_mut(), |i| h.wrapping_add(i) as *mut wchar_t)
}

/// `wcstok` (C99, se stavem v `save`).
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn wcstok(s: *mut wchar_t, delim: *const wchar_t, save: *mut *mut wchar_t) -> *mut wchar_t {
    // SAFETY: kontrakt C (`save` je platny; `s` nebo `*save` je retezec).
    unsafe {
        let mut p = if s.is_null() { *save } else { s };
        if p.is_null() {
            return ptr::null_mut();
        }
        p = p.add(wcsspn(p, delim));
        if *p == 0 {
            *save = ptr::null_mut();
            return ptr::null_mut();
        }
        let end = p.add(wcscspn(p, delim));
        if *end == 0 {
            *save = ptr::null_mut();
        } else {
            *end = 0;
            *save = end.add(1);
        }
        p
    }
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn wcsdup(s: *const wchar_t) -> *mut wchar_t {
    // SAFETY: kontrakt C; `malloc` vraci zarovnani 16 B.
    unsafe {
        let n = wlen(s) + 1;
        let d = cstd_core::stdlib::malloc(n * size_of::<wchar_t>()) as *mut wchar_t;
        if d.is_null() {
            crate::errno::set(crate::errno::ENOMEM);
            return d;
        }
        ptr::copy_nonoverlapping(s, d, n);
        d
    }
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn wmemchr(s: *const wchar_t, c: wchar_t, n: size_t) -> *mut wchar_t {
    if n == 0 {
        return ptr::null_mut();
    }
    // SAFETY: `s` ma `n` znaku.
    let sl = unsafe { core::slice::from_raw_parts(s, n) };
    sl.iter().position(|&x| x == c).map_or(ptr::null_mut(), |i| s.wrapping_add(i) as *mut wchar_t)
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn wmemcmp(a: *const wchar_t, b: *const wchar_t, n: size_t) -> c_int {
    if n == 0 {
        return 0;
    }
    // SAFETY: oba maji `n` znaku.
    let (x, y) = unsafe { (core::slice::from_raw_parts(a, n), core::slice::from_raw_parts(b, n)) };
    ord(x.cmp(y))
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn wmemcpy(d: *mut wchar_t, s: *const wchar_t, n: size_t) -> *mut wchar_t {
    // SAFETY: kontrakt C (neprekryvaji se).
    unsafe { ptr::copy_nonoverlapping(s, d, n) };
    d
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn wmempcpy(d: *mut wchar_t, s: *const wchar_t, n: size_t) -> *mut wchar_t {
    // SAFETY: kontrakt C.
    unsafe {
        ptr::copy_nonoverlapping(s, d, n);
        d.add(n)
    }
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn wmemmove(d: *mut wchar_t, s: *const wchar_t, n: size_t) -> *mut wchar_t {
    // SAFETY: kontrakt C.
    unsafe { ptr::copy(s, d, n) };
    d
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn wmemset(d: *mut wchar_t, c: wchar_t, n: size_t) -> *mut wchar_t {
    for i in 0..n {
        // SAFETY: `d` ma `n` znaku.
        unsafe { *d.add(i) = c };
    }
    d
}
