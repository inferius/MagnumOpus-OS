//! Cisla a cas v sirokych retezcich: `wcstol` a spol., `wcstod`,
//! `wcstof` (`wcstold` je v `c/wchar_ld.c`), `wcsftime`.
//!
//! Cisla jsou jen v ASCII: uvodni bile znaky se preskoci podle `iswspace`
//! (i siroke mezery), zbytek se prevede ASCII cast retezce stejnym kodem
//! jako `strtol`/`strtod`.

use core::ptr;

use super::{wchar_t, wctype};
use crate::time::{format_tm, tm};
use crate::{c_int, c_long, c_ulong, errno, size_t};

/// ASCII cast za uvodnimi bilymi znaky: (pocet bilych znaku, bajty).
///
/// # Safety
///
/// `s` je siroky retezec zakonceny nulou.
unsafe fn ascii_tail(s: *const wchar_t) -> (usize, Vec<u8>) {
    let mut i = 0;
    // SAFETY: kontrakt funkce.
    while unsafe { *s.add(i) } != 0 && wctype::iswspace(unsafe { *s.add(i) } as u32) != 0 {
        i += 1;
    }
    let mut out = Vec::new();
    loop {
        // SAFETY: viz vyse.
        let c = unsafe { *s.add(i + out.len()) } as u32;
        if c == 0 || c >= 0x80 {
            break;
        }
        out.push(c as u8);
    }
    (i, out)
}

/// Nastavi `*end` na `s + skip + used`, nebo na `s` (zadne cislo).
unsafe fn set_end(s: *const wchar_t, end: *mut *mut wchar_t, skip: usize, used: usize) {
    if !end.is_null() {
        let p = if used == 0 { s } else { s.wrapping_add(skip + used) };
        // SAFETY: `end` je platny (kontrakt C).
        unsafe { *end = p as *mut wchar_t };
    }
}

/// Cele cislo se znamenkem v rozsahu `min..=max` (preteceni = ERANGE).
pub fn to_signed(v: u64, neg: bool, overflow: bool, min: i64, max: i64) -> (i64, bool) {
    if overflow {
        return (if neg { min } else { max }, true);
    }
    if neg {
        let lim = (min as i128).unsigned_abs() as u64;
        if v > lim { (min, true) } else { ((v as i128).wrapping_neg() as i64, false) }
    } else if v > max as u64 {
        (max, true)
    } else {
        (v as i64, false)
    }
}

/// Cele cislo bez znamenka (zaporne = dvojkovy doplnek jako `strtoul`).
pub fn to_unsigned(v: u64, neg: bool, overflow: bool) -> (u64, bool) {
    if overflow { (u64::MAX, true) } else if neg { (v.wrapping_neg(), false) } else { (v, false) }
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn wcstoll(s: *const wchar_t, end: *mut *mut wchar_t, base: c_int) -> i64 {
    // SAFETY: kontrakt C.
    let (skip, a) = unsafe { ascii_tail(s) };
    if base != 0 && !(2..=36).contains(&base) {
        errno::set(errno::EINVAL);
        unsafe { set_end(s, end, 0, 0) };
        return 0;
    }
    let (v, neg, ovf, used) = cstd_core::stdlib::parse_integer(&a, base as u32);
    // SAFETY: `used` <= delka ASCII casti.
    unsafe { set_end(s, end, skip, used) };
    let (r, range) = to_signed(v, neg, ovf, i64::MIN, i64::MAX);
    if range {
        errno::set(errno::ERANGE);
    }
    r
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn wcstol(s: *const wchar_t, end: *mut *mut wchar_t, base: c_int) -> c_long {
    // SAFETY: kontrakt C (`long` ma 64 bitu).
    unsafe { wcstoll(s, end, base) }
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn wcstoimax(s: *const wchar_t, end: *mut *mut wchar_t, base: c_int) -> i64 {
    // SAFETY: kontrakt C.
    unsafe { wcstoll(s, end, base) }
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn wcstoull(s: *const wchar_t, end: *mut *mut wchar_t, base: c_int) -> u64 {
    // SAFETY: kontrakt C.
    let (skip, a) = unsafe { ascii_tail(s) };
    if base != 0 && !(2..=36).contains(&base) {
        errno::set(errno::EINVAL);
        unsafe { set_end(s, end, 0, 0) };
        return 0;
    }
    let (v, neg, ovf, used) = cstd_core::stdlib::parse_integer(&a, base as u32);
    // SAFETY: viz `wcstoll`.
    unsafe { set_end(s, end, skip, used) };
    let (r, range) = to_unsigned(v, neg, ovf);
    if range {
        errno::set(errno::ERANGE);
    }
    r
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn wcstoul(s: *const wchar_t, end: *mut *mut wchar_t, base: c_int) -> c_ulong {
    // SAFETY: kontrakt C.
    unsafe { wcstoull(s, end, base) }
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn wcstoumax(s: *const wchar_t, end: *mut *mut wchar_t, base: c_int) -> u64 {
    // SAFETY: kontrakt C.
    unsafe { wcstoull(s, end, base) }
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn wcstod(s: *const wchar_t, end: *mut *mut wchar_t) -> f64 {
    // SAFETY: kontrakt C.
    let (skip, a) = unsafe { ascii_tail(s) };
    let (v, used, range) = cstd_core::stdlib::parse_float(&a);
    // SAFETY: viz `wcstoll`.
    unsafe { set_end(s, end, skip, used) };
    if range {
        errno::set(errno::ERANGE);
    }
    v
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn wcstof(s: *const wchar_t, end: *mut *mut wchar_t) -> f32 {
    // SAFETY: kontrakt C.
    let v = unsafe { wcstod(s, end) };
    let f = v as f32;
    if (f.is_infinite() && v.is_finite()) || (f == 0.0 && v != 0.0) {
        errno::set(errno::ERANGE);
    }
    f
}

/// Jeden prevod `%...` formatu `strftime` (priznaky, sirka, `E`/`O`
/// a pismeno); vraci jeho delku.
fn conversion_len(fmt: &[u32]) -> usize {
    let mut i = 1;
    while i < fmt.len() && matches!(fmt[i], 0x30..=0x39 | 0x5F | 0x2D | 0x5E | 0x23 | 0x2B | 0x45 | 0x4F) {
        i += 1;
    }
    (i + 1).min(fmt.len())
}

/// `wcsftime` nad sirokym formatem: doslovne znaky se kopiruji, kazdy
/// prevod formatuje `strftime` (vystup locale C je ASCII).
pub fn format_tm_wide(fmt: &[u32], t: &tm) -> Vec<u32> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < fmt.len() {
        if fmt[i] != b'%' as u32 {
            out.push(fmt[i]);
            i += 1;
            continue;
        }
        let n = conversion_len(&fmt[i..]);
        let spec: Vec<u8> = fmt[i..i + n].iter().map(|&c| if c < 0x80 { c as u8 } else { b'?' }).collect();
        let text = format_tm(&spec, t);
        match core::str::from_utf8(&text) {
            Ok(s) => out.extend(s.chars().map(|c| c as u32)),
            Err(_) => out.extend(text.iter().map(|&b| b as u32)),
        }
        i += n;
    }
    out
}

/// `wcsftime`: 0 = vysledek se nevejde.
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn wcsftime(buf: *mut wchar_t, max: size_t, fmt: *const wchar_t, t: *const tm) -> size_t {
    // SAFETY: kontrakt C.
    let (Some(t), false, false) = (unsafe { t.as_ref() }, fmt.is_null(), buf.is_null()) else { return 0 };
    // SAFETY: siroky retezec zakonceny nulou.
    let f: Vec<u32> = unsafe { super::wslice(fmt) }.iter().map(|&c| c as u32).collect();
    let out = format_tm_wide(&f, t);
    if out.len() >= max {
        return 0;
    }
    for (i, &c) in out.iter().enumerate() {
        // SAFETY: `buf` ma `max` znaku.
        unsafe { *buf.add(i) = c as wchar_t };
    }
    // SAFETY: viz vyse.
    unsafe { *buf.add(out.len()) = 0 };
    out.len()
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn wcsftime_l(buf: *mut wchar_t, max: size_t, fmt: *const wchar_t, t: *const tm, _l: crate::locale::locale_t) -> size_t {
    // SAFETY: kontrakt C.
    unsafe { wcsftime(buf, max, fmt, t) }
}

/// Nevyuzite: zabrani varovani o `ptr` na hostiteli.
#[allow(dead_code)]
fn _unused() -> *const u8 {
    ptr::null()
}
