//! `stdlib.h`: pamet, prevody cisel, `abort`.
//!
//! `malloc` bere pamet z alokatoru programu (heap libos). Pred kazdym blokem
//! je hlavicka s velikosti (16 B, zachova zarovnani 16 B, ktere C
//! od `malloc` ceka), aby `free` a `realloc` znaly `Layout`.

use alloc::alloc::{Layout, alloc, alloc_zeroed, dealloc, realloc as rust_realloc};

use crate::ctypes::{c_char, c_int, c_long, c_ulong, size_t};

const HEADER: usize = 16;
const ALIGN: usize = 16;

fn layout(size: usize) -> Option<Layout> {
    Layout::from_size_align(size.checked_add(HEADER)?, ALIGN).ok()
}

unsafe fn finish(p: *mut u8, size: usize) -> *mut u8 {
    if p.is_null() {
        return p;
    }
    // SAFETY: blok ma aspon HEADER bajtu, zarovnani 16.
    unsafe {
        (p as *mut usize).write(size);
        p.add(HEADER)
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn malloc(size: size_t) -> *mut u8 {
    match layout(size) {
        // SAFETY: Layout ma nenulovou velikost (aspon HEADER).
        Some(l) => unsafe { finish(alloc(l), size) },
        None => core::ptr::null_mut(),
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn calloc(count: size_t, size: size_t) -> *mut u8 {
    // Preteceni nasobeni = NULL (ne maly blok, do ktereho by se pak psalo).
    let Some(total) = count.checked_mul(size) else {
        return core::ptr::null_mut();
    };
    match layout(total) {
        // SAFETY: dtto.
        Some(l) => unsafe { finish(alloc_zeroed(l), total) },
        None => core::ptr::null_mut(),
    }
}

/// Velikost bloku z hlavicky.
unsafe fn size_of_block(p: *mut u8) -> usize {
    // SAFETY: `p` vratil malloc/calloc/realloc (kontrakt C).
    unsafe { (p.sub(HEADER) as *const usize).read() }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn free(p: *mut u8) {
    if p.is_null() {
        return;
    }
    // SAFETY: kontrakt C; Layout je stejny jako pri alokaci.
    unsafe {
        let size = size_of_block(p);
        dealloc(p.sub(HEADER), layout(size).unwrap());
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn realloc(p: *mut u8, size: size_t) -> *mut u8 {
    if p.is_null() {
        // SAFETY: jako malloc.
        return unsafe { malloc(size) };
    }
    let Some(new) = layout(size) else {
        return core::ptr::null_mut();
    };
    // SAFETY: kontrakt C; stary Layout odpovida alokaci.
    unsafe {
        let old = size_of_block(p);
        let q = rust_realloc(p.sub(HEADER), layout(old).unwrap(), new.size());
        finish(q, size)
    }
}

/// Funkce pro `abort` (v systemu ukonci proces; vychozi = panika).
#[cfg_attr(all(not(test), feature = "service-io"), unsafe(no_mangle))]
pub extern "C" fn abort() -> ! {
    crate::stdio::flush();
    panic!("cstd: abort() z kodu v C");
}

/// `__cstd_assert_fail` z makra `assert` (`assert.h`).
#[cfg_attr(all(not(test), feature = "service-io"), unsafe(no_mangle))]
pub unsafe extern "C" fn __cstd_assert_fail(expr: *const c_char, file: *const c_char, line: c_int) -> ! {
    // SAFETY: retezce z makra (literaly zakoncene nulou).
    let (e, f) = unsafe {
        (
            core::slice::from_raw_parts(expr, crate::string::strlen(expr)),
            core::slice::from_raw_parts(file, crate::string::strlen(file)),
        )
    };
    crate::stdio::flush();
    panic!(
        "cstd: assert({}) selhal v {}:{line}",
        core::str::from_utf8(e).unwrap_or("?"),
        core::str::from_utf8(f).unwrap_or("?")
    );
}

/// Spolecny prevod textu na cislo (`strtoul` a spol.). Vraci (hodnota,
/// zaporne, preteceni, pocet prectenych bajtu; 0 = zadne cislo).
pub fn parse_integer(s: &[u8], base: u32) -> (u64, bool, bool, usize) {
    let mut i = 0;
    while i < s.len() && matches!(s[i], b' ' | b'\t' | b'\n' | 0x0B | 0x0C | b'\r') {
        i += 1;
    }
    let mut neg = false;
    if i < s.len() && (s[i] == b'+' || s[i] == b'-') {
        neg = s[i] == b'-';
        i += 1;
    }
    let mut base = base;
    let hex_prefix = i + 1 < s.len() && s[i] == b'0' && (s[i + 1] | 0x20) == b'x';
    let hex_digit_after = hex_prefix && s.get(i + 2).is_some_and(|c| c.is_ascii_hexdigit());
    if (base == 0 || base == 16) && hex_digit_after {
        base = 16;
        i += 2;
    } else if base == 0 {
        base = if i < s.len() && s[i] == b'0' { 8 } else { 10 };
    }
    if !(2..=36).contains(&base) {
        return (0, false, false, 0);
    }
    let digits_start = i;
    let mut v: u64 = 0;
    let mut overflow = false;
    while i < s.len() {
        let d = match s[i] {
            c @ b'0'..=b'9' => (c - b'0') as u32,
            c @ b'a'..=b'z' => (c - b'a') as u32 + 10,
            c @ b'A'..=b'Z' => (c - b'A') as u32 + 10,
            _ => break,
        };
        if d >= base {
            break;
        }
        match v.checked_mul(base as u64).and_then(|x| x.checked_add(d as u64)) {
            Some(x) => v = x,
            None => overflow = true,
        }
        i += 1;
    }
    if i == digits_start {
        return (0, false, false, 0);
    }
    (v, neg, overflow, i)
}

/// Retezec od `p` (zakonceny nulou) jako rez.
unsafe fn cstr<'a>(p: *const c_char) -> &'a [u8] {
    // SAFETY: kontrakt C.
    unsafe { core::slice::from_raw_parts(p, crate::string::strlen(p)) }
}

unsafe fn set_end(end: *mut *mut c_char, p: *const c_char, used: usize) {
    if !end.is_null() {
        // SAFETY: `end` je platny ukazatel (kontrakt C), `used` <= delka.
        unsafe { *end = p.add(used) as *mut c_char };
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn strtoull(p: *const c_char, end: *mut *mut c_char, base: c_int) -> u64 {
    // SAFETY: kontrakt C.
    let (v, neg, overflow, used) = parse_integer(unsafe { cstr(p) }, base.max(0) as u32);
    unsafe { set_end(end, p, used) };
    if overflow {
        u64::MAX
    } else if neg {
        v.wrapping_neg()
    } else {
        v
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn strtoul(p: *const c_char, end: *mut *mut c_char, base: c_int) -> c_ulong {
    // SAFETY: kontrakt C (`unsigned long` ma 64 bitu).
    unsafe { strtoull(p, end, base) }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn strtoll(p: *const c_char, end: *mut *mut c_char, base: c_int) -> i64 {
    // SAFETY: kontrakt C.
    let (v, neg, overflow, used) = parse_integer(unsafe { cstr(p) }, base.max(0) as u32);
    unsafe { set_end(end, p, used) };
    match (neg, overflow) {
        (false, true) => i64::MAX,
        (true, true) => i64::MIN,
        (false, false) => i64::try_from(v).unwrap_or(i64::MAX),
        (true, false) => {
            if v > i64::MAX as u64 + 1 {
                i64::MIN
            } else {
                (v as i64).wrapping_neg()
            }
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn strtol(p: *const c_char, end: *mut *mut c_char, base: c_int) -> c_long {
    // SAFETY: kontrakt C (`long` ma 64 bitu).
    unsafe { strtoll(p, end, base) }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn atoi(p: *const c_char) -> c_int {
    // SAFETY: kontrakt C.
    unsafe { strtol(p, core::ptr::null_mut(), 10) as c_int }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub extern "C" fn abs(v: c_int) -> c_int {
    v.wrapping_abs()
}

/// Prevod textu na `double` (`strtod`): vraci (hodnota, pocet prectenych
/// bajtu, mimo rozsah). 0 bajtu = zadne cislo. Umi desitkovy tvar
/// s exponentem, sestnactkovy `0x1.8p3`, `inf`, `infinity`, `nan`,
/// `nan(...)`. Desitkove cislice prevadi `core` (presne zaokrouhleni).
pub fn parse_float(s: &[u8]) -> (f64, usize, bool) {
    let mut i = 0;
    while i < s.len() && matches!(s[i], b' ' | b'\t' | b'\n' | 0x0B | 0x0C | b'\r') {
        i += 1;
    }
    let start = i;
    let mut neg = false;
    if i < s.len() && (s[i] == b'+' || s[i] == b'-') {
        neg = s[i] == b'-';
        i += 1;
    }
    let sign = |v: f64| if neg { -v } else { v };
    let rest = &s[i..];
    let starts = |w: &[u8]| rest.len() >= w.len() && rest[..w.len()].eq_ignore_ascii_case(w);
    if starts(b"infinity") {
        return (sign(f64::INFINITY), i + 8, false);
    }
    if starts(b"inf") {
        return (sign(f64::INFINITY), i + 3, false);
    }
    if starts(b"nan") {
        let mut j = i + 3;
        if s.get(j) == Some(&b'(')
            && let Some(k) = s[j + 1..].iter().position(|&c| !(c.is_ascii_alphanumeric() || c == b'_'))
            && s[j + 1 + k] == b')'
        {
            j += k + 2;
        }
        return (sign(f64::NAN), j, false);
    }
    if rest.len() > 2 && rest[0] == b'0' && (rest[1] | 0x20) == b'x' {
        if let Some((v, used)) = parse_hex_float(&rest[2..]) {
            let r = sign(v);
            return (r, i + 2 + used, v.is_infinite());
        }
        // "0x" bez cislic: precte se jen "0".
        return (sign(0.0), i + 1, false);
    }
    // Desitkovy tvar: cislice, tecka, cislice, exponent.
    let mut j = i;
    let mut digits = 0;
    while j < s.len() && s[j].is_ascii_digit() {
        j += 1;
        digits += 1;
    }
    if j < s.len() && s[j] == b'.' {
        j += 1;
        while j < s.len() && s[j].is_ascii_digit() {
            j += 1;
            digits += 1;
        }
    }
    if digits == 0 {
        return (0.0, 0, false);
    }
    if j < s.len() && (s[j] | 0x20) == b'e' {
        let mut k = j + 1;
        if k < s.len() && (s[k] == b'+' || s[k] == b'-') {
            k += 1;
        }
        if k < s.len() && s[k].is_ascii_digit() {
            while k < s.len() && s[k].is_ascii_digit() {
                k += 1;
            }
            j = k;
        }
    }
    let text = core::str::from_utf8(&s[start..j]).unwrap_or("0");
    // `core` neprijme "+" pred cislem jen s teckou na konci ("1." ano).
    let v: f64 = text.parse().unwrap_or(0.0);
    let all_zero = s[i..j].iter().take_while(|c| **c != b'e' && **c != b'E').all(|c| *c == b'0' || *c == b'.');
    let range = v.is_infinite() || (v == 0.0 && !all_zero);
    (v, j, range)
}

/// Sestnactkova cast za `0x`: (hodnota, prectene bajty).
fn parse_hex_float(s: &[u8]) -> Option<(f64, usize)> {
    let mut i = 0;
    let mut mant: u64 = 0;
    let mut exp: i64 = 0;
    let mut digits = 0;
    let mut sticky = false;
    let mut point = false;
    while i < s.len() {
        let c = s[i];
        if c == b'.' && !point {
            point = true;
            i += 1;
            continue;
        }
        let Some(d) = (c as char).to_digit(16) else { break };
        digits += 1;
        if mant >> 60 == 0 {
            mant = mant << 4 | u64::from(d);
            if point {
                exp -= 4;
            }
        } else {
            // Cislice navic: jen zaokrouhlovaci informace a posun.
            sticky |= d != 0;
            if !point {
                exp += 4;
            }
        }
        i += 1;
    }
    if digits == 0 {
        return None;
    }
    if i < s.len() && (s[i] | 0x20) == b'p' {
        let mut k = i + 1;
        let mut eneg = false;
        if k < s.len() && (s[k] == b'+' || s[k] == b'-') {
            eneg = s[k] == b'-';
            k += 1;
        }
        if k < s.len() && s[k].is_ascii_digit() {
            let mut e: i64 = 0;
            while k < s.len() && s[k].is_ascii_digit() {
                e = (e * 10 + i64::from(s[k] - b'0')).min(100_000);
                k += 1;
            }
            exp += if eneg { -e } else { e };
            i = k;
        }
    }
    if sticky {
        mant |= 1;
    }
    // mant * 2^exp; nasobeni mocninou dvou je presne, dokud nepretece.
    let mut v = mant as f64;
    let mut e = exp;
    while e > 0 {
        let step = e.min(1000);
        v *= f64::from_bits(((1023 + step.min(1023)) as u64) << 52);
        e -= step.min(1023);
    }
    while e < 0 {
        let step = (-e).min(1022);
        v *= f64::from_bits(((1023 - step) as u64) << 52);
        e += step;
    }
    Some((v, i))
}
