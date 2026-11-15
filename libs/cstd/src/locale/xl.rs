//! Varianty `*_l` s locale v argumentu (POSIX 2008, libc++ je pouziva
//! misto `uselocale`). Kategorie se zatim nelokalizuji, takze se vetsina
//! chova jako zakladni funkce; trida znaku nezavisi na locale (`isalpha_l`
//! = ASCII, `iswalpha_l` = Unicode), kodovani (`MB_CUR_MAX`) ano.

use cstd_core::ctype;

use super::locale_t;
use crate::time::tm;
use crate::wchar::{wctrans_t, wctype_t, wint_t};
use crate::{c_char, c_int, c_long, c_ulong, size_t};

macro_rules! narrow_l {
    ($($name:ident => $base:path;)*) => {$(
        #[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
        pub extern "C" fn $name(c: c_int, _l: locale_t) -> c_int {
            $base(c)
        }
    )*};
}

narrow_l! {
    isalnum_l => ctype::isalnum;
    isalpha_l => ctype::isalpha;
    isblank_l => ctype::isblank;
    iscntrl_l => ctype::iscntrl;
    isdigit_l => ctype::isdigit;
    isgraph_l => ctype::isgraph;
    islower_l => ctype::islower;
    isprint_l => ctype::isprint;
    ispunct_l => ctype::ispunct;
    isspace_l => ctype::isspace;
    isupper_l => ctype::isupper;
    isxdigit_l => ctype::isxdigit;
    tolower_l => ctype::tolower;
    toupper_l => ctype::toupper;
}

macro_rules! wide_l {
    ($($name:ident => $base:path;)*) => {$(
        #[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
        pub extern "C" fn $name(c: wint_t, _l: locale_t) -> c_int {
            $base(c)
        }
    )*};
}

wide_l! {
    iswalnum_l => crate::wchar::wctype::iswalnum;
    iswalpha_l => crate::wchar::wctype::iswalpha;
    iswblank_l => crate::wchar::wctype::iswblank;
    iswcntrl_l => crate::wchar::wctype::iswcntrl;
    iswdigit_l => crate::wchar::wctype::iswdigit;
    iswgraph_l => crate::wchar::wctype::iswgraph;
    iswlower_l => crate::wchar::wctype::iswlower;
    iswprint_l => crate::wchar::wctype::iswprint;
    iswpunct_l => crate::wchar::wctype::iswpunct;
    iswspace_l => crate::wchar::wctype::iswspace;
    iswupper_l => crate::wchar::wctype::iswupper;
    iswxdigit_l => crate::wchar::wctype::iswxdigit;
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub extern "C" fn towlower_l(c: wint_t, _l: locale_t) -> wint_t {
    crate::wchar::wctype::towlower(c)
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub extern "C" fn towupper_l(c: wint_t, _l: locale_t) -> wint_t {
    crate::wchar::wctype::towupper(c)
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub extern "C" fn iswctype_l(c: wint_t, t: wctype_t, _l: locale_t) -> c_int {
    crate::wchar::wctype::iswctype(c, t)
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn wctype_l(name: *const c_char, _l: locale_t) -> wctype_t {
    // SAFETY: kontrakt C.
    unsafe { crate::wchar::wctype::wctype(name) }
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub extern "C" fn towctrans_l(c: wint_t, t: wctrans_t, _l: locale_t) -> wint_t {
    crate::wchar::wctype::towctrans(c, t)
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn wctrans_l(name: *const c_char, _l: locale_t) -> wctrans_t {
    // SAFETY: kontrakt C.
    unsafe { crate::wchar::wctype::wctrans(name) }
}

/// `strxfrm` v locale C: kopie (razeni = poradi bajtu). Vraci delku
/// zdroje; vetsi nebo rovna `n` = obsah `dst` je neurcity.
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn strxfrm(dst: *mut c_char, src: *const c_char, n: size_t) -> size_t {
    // SAFETY: kontrakt C.
    let s = unsafe { crate::c_bytes(src) };
    if s.len() < n {
        // SAFETY: `dst` ma aspon `n` bajtu.
        unsafe { core::ptr::copy_nonoverlapping(s.as_ptr(), dst, s.len() + 1) };
    }
    s.len()
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn strxfrm_l(dst: *mut c_char, src: *const c_char, n: size_t, _l: locale_t) -> size_t {
    // SAFETY: kontrakt C.
    unsafe { strxfrm(dst, src, n) }
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn strcoll_l(a: *const c_char, b: *const c_char, _l: locale_t) -> c_int {
    // SAFETY: kontrakt C.
    unsafe { cstd_core::string::strcmp(a, b) }
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn wcscoll_l(a: *const crate::wchar::wchar_t, b: *const crate::wchar::wchar_t, _l: locale_t) -> c_int {
    // SAFETY: kontrakt C.
    unsafe { crate::wchar::wcs::wcscoll(a, b) }
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn wcsxfrm_l(dst: *mut crate::wchar::wchar_t, src: *const crate::wchar::wchar_t, n: size_t, _l: locale_t) -> size_t {
    // SAFETY: kontrakt C.
    unsafe { crate::wchar::wcs::wcsxfrm(dst, src, n) }
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn strftime_l(buf: *mut c_char, max: size_t, fmt: *const c_char, t: *const tm, _l: locale_t) -> size_t {
    // SAFETY: kontrakt C.
    unsafe { crate::time::strftime(buf, max, fmt, t) }
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn strtod_l(p: *const c_char, end: *mut *mut c_char, _l: locale_t) -> f64 {
    // SAFETY: kontrakt C.
    unsafe { crate::math::strtod(p, end) }
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn strtof_l(p: *const c_char, end: *mut *mut c_char, _l: locale_t) -> f32 {
    // SAFETY: kontrakt C.
    unsafe { crate::math::strtof(p, end) }
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn strtol_l(p: *const c_char, end: *mut *mut c_char, base: c_int, _l: locale_t) -> c_long {
    // SAFETY: kontrakt C.
    unsafe { cstd_core::stdlib::strtol(p, end, base) }
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn strtoll_l(p: *const c_char, end: *mut *mut c_char, base: c_int, _l: locale_t) -> i64 {
    // SAFETY: kontrakt C.
    unsafe { cstd_core::stdlib::strtoll(p, end, base) }
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn strtoul_l(p: *const c_char, end: *mut *mut c_char, base: c_int, _l: locale_t) -> c_ulong {
    // SAFETY: kontrakt C.
    unsafe { cstd_core::stdlib::strtoul(p, end, base) }
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn strtoull_l(p: *const c_char, end: *mut *mut c_char, base: c_int, _l: locale_t) -> u64 {
    // SAFETY: kontrakt C.
    unsafe { cstd_core::stdlib::strtoull(p, end, base) }
}
