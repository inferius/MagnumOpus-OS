//! `wctype.h`: tridy sirokych znaku a prevody velikosti pismen podle
//! Unicode (tabulky `libs/uniprop` z UCD, stejna verze jako
//! `core::char`). Nezavisi na locale (jako musl); `iswdigit` a `iswxdigit`
//! jsou jen ASCII (standard C).

use super::{WEOF, wctrans_t, wctype_t, wint_t};
use crate::{c_bytes, c_char, c_int};

/// Cisla trid pro `wctype`/`iswctype` (0 = neplatna).
const CLASSES: [(&[u8], fn(u32) -> bool); 12] = [
    (b"alnum", uniprop::is_alnum),
    (b"alpha", uniprop::is_alpha),
    (b"blank", uniprop::is_blank),
    (b"cntrl", uniprop::is_cntrl),
    (b"digit", uniprop::is_digit),
    (b"graph", uniprop::is_graph),
    (b"lower", uniprop::is_lower),
    (b"print", uniprop::is_print),
    (b"punct", uniprop::is_punct),
    (b"space", uniprop::is_space),
    (b"upper", uniprop::is_upper),
    (b"xdigit", uniprop::is_xdigit),
];

macro_rules! class {
    ($($name:ident => $test:path;)*) => {$(
        #[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
        pub extern "C" fn $name(c: wint_t) -> c_int {
            $test(c) as c_int
        }
    )*};
}

class! {
    iswalnum => uniprop::is_alnum;
    iswalpha => uniprop::is_alpha;
    iswblank => uniprop::is_blank;
    iswcntrl => uniprop::is_cntrl;
    iswdigit => uniprop::is_digit;
    iswgraph => uniprop::is_graph;
    iswlower => uniprop::is_lower;
    iswprint => uniprop::is_print;
    iswpunct => uniprop::is_punct;
    iswspace => uniprop::is_space;
    iswupper => uniprop::is_upper;
    iswxdigit => uniprop::is_xdigit;
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub extern "C" fn towupper(c: wint_t) -> wint_t {
    if c == WEOF { c } else { uniprop::to_upper(c) }
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub extern "C" fn towlower(c: wint_t) -> wint_t {
    if c == WEOF { c } else { uniprop::to_lower(c) }
}

/// Trida podle jmena (`"alpha"`, ...): cislo 1-12, nezname = 0.
pub fn class_by_name(name: &[u8]) -> wctype_t {
    CLASSES.iter().position(|(n, _)| *n == name).map_or(0, |i| i as wctype_t + 1)
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn wctype(name: *const c_char) -> wctype_t {
    if name.is_null() {
        return 0;
    }
    // SAFETY: retezec C.
    class_by_name(unsafe { c_bytes(name) })
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub extern "C" fn iswctype(c: wint_t, t: wctype_t) -> c_int {
    match (t as usize).checked_sub(1).and_then(|i| CLASSES.get(i)) {
        Some((_, f)) => f(c) as c_int,
        None => 0,
    }
}

/// `wctrans_t` je v C ukazatel; hodnoty 1 a 2 jsou jen znacky (jako musl).
const TOUPPER: wctrans_t = 1 as wctrans_t;
const TOLOWER: wctrans_t = 2 as wctrans_t;

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn wctrans(name: *const c_char) -> wctrans_t {
    if name.is_null() {
        return core::ptr::null();
    }
    // SAFETY: retezec C.
    match unsafe { c_bytes(name) } {
        b"toupper" => TOUPPER,
        b"tolower" => TOLOWER,
        _ => core::ptr::null(),
    }
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub extern "C" fn towctrans(c: wint_t, t: wctrans_t) -> wint_t {
    if t == TOUPPER {
        towupper(c)
    } else if t == TOLOWER {
        towlower(c)
    } else {
        c
    }
}

/// `wcwidth`: sirka v bunkach terminalu (0, 1, 2; -1 netisknutelny).
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub extern "C" fn wcwidth(c: super::wchar_t) -> c_int {
    uniprop::width(c as u32)
}

/// `wcswidth`: soucet sirek nejvys `n` znaku; -1 = netisknutelny znak.
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn wcswidth(s: *const super::wchar_t, n: crate::size_t) -> c_int {
    let mut total: c_int = 0;
    for i in 0..n {
        // SAFETY: retezec zakonceny nulou nebo aspon `n` znaku.
        let c = unsafe { *s.add(i) } as u32;
        if c == 0 {
            break;
        }
        let w = uniprop::width(c);
        if w < 0 {
            return -1;
        }
        total = total.saturating_add(w);
    }
    total
}
