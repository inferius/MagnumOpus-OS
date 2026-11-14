//! `localeconv` a `nl_langinfo`: texty locale C (C.UTF-8 se lisi jen
//! v `CODESET`; kategorie se zatim nelokalizuji).

use core::ffi::c_int;

use super::{Locale, locale_t};
use crate::c_char;

/// `struct lconv` (jen ciselna cast locale C).
#[repr(C)]
pub struct lconv {
    pub decimal_point: *const c_char,
    pub thousands_sep: *const c_char,
    pub grouping: *const c_char,
    _rest: [*const c_char; 7],
    _chars: [c_char; 14],
}

struct Lc(lconv);
// SAFETY: jen staticke retezce pro cteni.
unsafe impl Sync for Lc {}

static LCONV: Lc = Lc(lconv {
    decimal_point: b".\0".as_ptr(),
    thousands_sep: b"\0".as_ptr(),
    grouping: b"\0".as_ptr(),
    _rest: [b"\0".as_ptr(); 7],
    _chars: [127; 14],
});

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub extern "C" fn localeconv() -> *const lconv {
    &LCONV.0
}

/// Polozky `nl_langinfo` (cisla musi sedet s `include/langinfo.h`).
pub mod nl {
    use core::ffi::c_int;
    pub const CODESET: c_int = 0;
    pub const D_T_FMT: c_int = 1;
    pub const D_FMT: c_int = 2;
    pub const T_FMT: c_int = 3;
    pub const T_FMT_AMPM: c_int = 4;
    pub const AM_STR: c_int = 5;
    pub const PM_STR: c_int = 6;
    pub const RADIXCHAR: c_int = 7;
    pub const THOUSEP: c_int = 8;
    pub const YESEXPR: c_int = 9;
    pub const NOEXPR: c_int = 10;
    pub const DAY_1: c_int = 11;
    pub const ABDAY_1: c_int = 18;
    pub const MON_1: c_int = 25;
    pub const ABMON_1: c_int = 37;
    pub const CRNCYSTR: c_int = 49;
}

const DAYS: [&[u8]; 7] = [b"Sunday\0", b"Monday\0", b"Tuesday\0", b"Wednesday\0", b"Thursday\0", b"Friday\0", b"Saturday\0"];
const ABDAYS: [&[u8]; 7] = [b"Sun\0", b"Mon\0", b"Tue\0", b"Wed\0", b"Thu\0", b"Fri\0", b"Sat\0"];
const MONTHS: [&[u8]; 12] = [
    b"January\0", b"February\0", b"March\0", b"April\0", b"May\0", b"June\0", b"July\0", b"August\0", b"September\0", b"October\0", b"November\0", b"December\0",
];
const ABMONTHS: [&[u8]; 12] = [b"Jan\0", b"Feb\0", b"Mar\0", b"Apr\0", b"May\0", b"Jun\0", b"Jul\0", b"Aug\0", b"Sep\0", b"Oct\0", b"Nov\0", b"Dec\0"];

/// Text polozky v locale C / C.UTF-8 (zakonceny nulou); neznama = "".
pub fn langinfo(item: c_int, utf8: bool) -> &'static [u8] {
    let idx = |base: c_int, n: c_int| (base..base + n).contains(&item).then(|| (item - base) as usize);
    match item {
        nl::CODESET => {
            if utf8 {
                b"UTF-8\0"
            } else {
                b"ANSI_X3.4-1968\0"
            }
        }
        nl::D_T_FMT => b"%a %b %e %H:%M:%S %Y\0",
        nl::D_FMT => b"%m/%d/%y\0",
        nl::T_FMT => b"%H:%M:%S\0",
        nl::T_FMT_AMPM => b"%I:%M:%S %p\0",
        nl::AM_STR => b"AM\0",
        nl::PM_STR => b"PM\0",
        nl::RADIXCHAR => b".\0",
        nl::YESEXPR => b"^[yY]\0",
        nl::NOEXPR => b"^[nN]\0",
        _ => {
            if let Some(i) = idx(nl::DAY_1, 7) {
                DAYS[i]
            } else if let Some(i) = idx(nl::ABDAY_1, 7) {
                ABDAYS[i]
            } else if let Some(i) = idx(nl::MON_1, 12) {
                MONTHS[i]
            } else if let Some(i) = idx(nl::ABMON_1, 12) {
                ABMONTHS[i]
            } else {
                b"\0"
            }
        }
    }
}

fn utf8_ctype(l: Locale) -> bool {
    l.ctype() == cstd_core::mb::Enc::Utf8
}

/// `nl_langinfo`: texty locale vlakna.
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub extern "C" fn nl_langinfo(item: c_int) -> *const c_char {
    langinfo(item, utf8_ctype(super::current())).as_ptr() as *const c_char
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn nl_langinfo_l(item: c_int, l: locale_t) -> *const c_char {
    // SAFETY: kontrakt C.
    langinfo(item, utf8_ctype(unsafe { super::get(l) })).as_ptr() as *const c_char
}
