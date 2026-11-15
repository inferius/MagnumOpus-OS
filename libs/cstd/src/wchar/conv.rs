//! Prevody vicebajtovych a sirokych znaku podle kodovani locale vlakna
//! (`LC_CTYPE`): `mbrtowc`, `wcrtomb`, retezcove `mbsrtowcs`/`wcsrtombs`
//! (i `n`-varianty pro libc++) a starsi bezstavove `mbtowc`, `mblen`,
//! `wctomb`, `mbstowcs`, `wcstombs`. Automat kodovani je v
//! [`cstd_core::mb`].
//!
//! Funkce s nulovym `mbstate_t *` pouzivaji vlastni stav vlakna (standard
//! dovoluje stav sdileny; ve vlakne je bezpecny).

use core::cell::Cell;
use core::ptr;

use cstd_core::mb::{Enc, MB_ERROR, MB_INCOMPLETE, MbState};

use super::{WEOF, mbstate_t, wchar_t, wint_t};
use crate::{c_char, c_int, errno, locale, size_t};

std::thread_local! {
    static MBRTOWC: Cell<MbState> = const { Cell::new(MbState::INITIAL) };
    static MBRLEN: Cell<MbState> = const { Cell::new(MbState::INITIAL) };
    static WCRTOMB: Cell<MbState> = const { Cell::new(MbState::INITIAL) };
    static MBSRTOWCS: Cell<MbState> = const { Cell::new(MbState::INITIAL) };
    static WCSRTOMBS: Cell<MbState> = const { Cell::new(MbState::INITIAL) };
}

/// Prace se stavem z ukazatele, nebo se stavem vlakna `own`.
fn with_state<R>(ps: *mut mbstate_t, own: &'static std::thread::LocalKey<Cell<MbState>>, f: impl FnOnce(&mut MbState) -> R) -> R {
    // SAFETY: `ps` je nulovy nebo platny stav (kontrakt C).
    if let Some(st) = unsafe { ps.as_mut() } {
        return f(st);
    }
    own.with(|c| {
        let mut st = c.get();
        let r = f(&mut st);
        c.set(st);
        r
    })
}

fn eilseq() -> usize {
    errno::set(errno::EILSEQ);
    MB_ERROR
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn mbsinit(ps: *const mbstate_t) -> c_int {
    // SAFETY: nulovy nebo platny stav (kontrakt C).
    unsafe { ps.as_ref() }.is_none_or(MbState::is_initial) as c_int
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub extern "C" fn btowc(c: c_int) -> wint_t {
    if !(0..=255).contains(&c) {
        return WEOF;
    }
    match locale::ctype().decode(&[c as u8]) {
        Some((wc, _)) => wc,
        None => WEOF,
    }
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub extern "C" fn wctob(wc: wint_t) -> c_int {
    let mut b = [0u8; 4];
    match locale::ctype().encode(wc, &mut b) {
        Some(1) => b[0] as c_int,
        _ => -1,
    }
}

/// `mbrtowc` s kodovanim `enc` (pro `*_l` a testy).
///
/// # Safety
///
/// Kontrakt `mbrtowc`.
pub unsafe fn mbrtowc_enc(enc: Enc, pwc: *mut wchar_t, s: *const c_char, n: size_t, st: &mut MbState) -> size_t {
    // `s == NULL`: jako `mbrtowc(NULL, "", 1, ps)` - navrat do pocatecniho
    // stavu (rozpracovany znak = chyba).
    let (pwc, bytes): (*mut wchar_t, &[u8]) = if s.is_null() {
        (ptr::null_mut(), b"\0")
    } else {
        // SAFETY: `s` ma aspon `n` bajtu (kontrakt C); cteme jen do prvniho
        // hotoveho znaku.
        (pwc, unsafe { core::slice::from_raw_parts(s, n) })
    };
    if bytes.is_empty() {
        return MB_INCOMPLETE;
    }
    let (c, used) = enc.mbrtowc(bytes, st);
    match (c, used) {
        (_, MB_ERROR) => eilseq(),
        (Some(c), used) => {
            if !pwc.is_null() {
                // SAFETY: `pwc` ukazuje na jeden `wchar_t`.
                unsafe { *pwc = c as wchar_t };
            }
            used
        }
        (None, n) => n,
    }
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn mbrtowc(pwc: *mut wchar_t, s: *const c_char, n: size_t, ps: *mut mbstate_t) -> size_t {
    let enc = locale::ctype();
    // SAFETY: kontrakt C.
    with_state(ps, &MBRTOWC, |st| unsafe { mbrtowc_enc(enc, pwc, s, n, st) })
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn mbrlen(s: *const c_char, n: size_t, ps: *mut mbstate_t) -> size_t {
    let enc = locale::ctype();
    // SAFETY: kontrakt C.
    with_state(ps, &MBRLEN, |st| unsafe { mbrtowc_enc(enc, ptr::null_mut(), s, n, st) })
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn mbtowc(pwc: *mut wchar_t, s: *const c_char, n: size_t) -> c_int {
    // Kodovani nema stav posunu: 0 = "bez zavislosti na stavu".
    if s.is_null() {
        return 0;
    }
    let mut st = MbState::INITIAL;
    // SAFETY: kontrakt C.
    match unsafe { mbrtowc_enc(locale::ctype(), pwc, s, n, &mut st) } {
        MB_ERROR => -1,
        MB_INCOMPLETE => {
            errno::set(errno::EILSEQ);
            -1
        }
        k => k as c_int,
    }
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn mblen(s: *const c_char, n: size_t) -> c_int {
    // SAFETY: kontrakt C.
    unsafe { mbtowc(ptr::null_mut(), s, n) }
}

/// `wcrtomb` s kodovanim `enc`.
///
/// # Safety
///
/// `s` je nulovy nebo ma aspon `MB_CUR_MAX` bajtu.
pub unsafe fn wcrtomb_enc(enc: Enc, s: *mut c_char, wc: wchar_t, st: &mut MbState) -> size_t {
    // `s == NULL`: jako `wcrtomb(buf, L'\0', ps)`.
    let wc = if s.is_null() { 0 } else { wc as u32 };
    if !st.is_initial() {
        *st = MbState::INITIAL;
        return eilseq();
    }
    let mut b = [0u8; 4];
    let Some(k) = enc.encode(wc, &mut b) else { return eilseq() };
    if !s.is_null() {
        // SAFETY: kontrakt funkce.
        unsafe { ptr::copy_nonoverlapping(b.as_ptr(), s, k) };
    }
    k
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn wcrtomb(s: *mut c_char, wc: wchar_t, ps: *mut mbstate_t) -> size_t {
    let enc = locale::ctype();
    // SAFETY: kontrakt C.
    with_state(ps, &WCRTOMB, |st| unsafe { wcrtomb_enc(enc, s, wc, st) })
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn wctomb(s: *mut c_char, wc: wchar_t) -> c_int {
    if s.is_null() {
        return 0;
    }
    let mut st = MbState::INITIAL;
    // SAFETY: kontrakt C.
    match unsafe { wcrtomb_enc(locale::ctype(), s, wc, &mut st) } {
        MB_ERROR => -1,
        k => k as c_int,
    }
}

/// `mbsnrtowcs`: nejvys `nms` bajtu zdroje, nejvys `len` sirokych znaku do
/// `dst` (nulovy `dst` = jen pocitat, `len` se ignoruje a `*src` se
/// nemeni). Vraci pocet znaku bez koncove nuly, nebo `(size_t)-1`.
///
/// # Safety
///
/// Kontrakt `mbsnrtowcs`.
pub unsafe fn mbsnrtowcs_enc(enc: Enc, dst: *mut wchar_t, src: *mut *const c_char, nms: size_t, len: size_t, st: &mut MbState) -> size_t {
    // SAFETY: `src` ukazuje na ukazatel na retezec (kontrakt C).
    let mut p = unsafe { *src };
    let mut left = nms;
    let mut count = 0usize;
    while dst.is_null() || count < len {
        if left == 0 {
            break;
        }
        let mut s2 = *st;
        let start = p;
        // Jeden znak (muze byt rozpracovany ze stavu).
        let mut done = None;
        while left > 0 {
            // SAFETY: viz vyse.
            let b = unsafe { *p };
            p = p.wrapping_add(1);
            left -= 1;
            match enc.feed(&mut s2, b) {
                cstd_core::mb::Step::Char(c) => {
                    done = Some(c);
                    break;
                }
                cstd_core::mb::Step::More => {}
                cstd_core::mb::Step::Bad => {
                    if !dst.is_null() {
                        // SAFETY: viz vyse.
                        unsafe { *src = start };
                    }
                    *st = MbState::INITIAL;
                    return eilseq();
                }
            }
        }
        let Some(c) = done else {
            // Zdroj skoncil uprostred znaku: stav si ho pamatuje.
            *st = s2;
            if !dst.is_null() {
                // SAFETY: viz vyse.
                unsafe { *src = p };
            }
            return count;
        };
        *st = s2;
        if !dst.is_null() {
            // SAFETY: `dst` ma aspon `len` znaku.
            unsafe { *dst.add(count) = c as wchar_t };
        }
        if c == 0 {
            if !dst.is_null() {
                // SAFETY: viz vyse.
                unsafe { *src = ptr::null() };
            }
            return count;
        }
        count += 1;
    }
    if !dst.is_null() {
        // SAFETY: viz vyse.
        unsafe { *src = p };
    }
    count
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn mbsnrtowcs(dst: *mut wchar_t, src: *mut *const c_char, nms: size_t, len: size_t, ps: *mut mbstate_t) -> size_t {
    let enc = locale::ctype();
    // SAFETY: kontrakt C.
    with_state(ps, &MBSRTOWCS, |st| unsafe { mbsnrtowcs_enc(enc, dst, src, nms, len, st) })
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn mbsrtowcs(dst: *mut wchar_t, src: *mut *const c_char, len: size_t, ps: *mut mbstate_t) -> size_t {
    // SAFETY: kontrakt C (retezec konci nulou).
    unsafe { mbsnrtowcs(dst, src, usize::MAX, len, ps) }
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn mbstowcs(dst: *mut wchar_t, src: *const c_char, len: size_t) -> size_t {
    let mut p = src;
    let mut st = MbState::INITIAL;
    // SAFETY: kontrakt C.
    unsafe { mbsnrtowcs_enc(locale::ctype(), dst, &mut p, usize::MAX, len, &mut st) }
}

/// `wcsnrtombs`: nejvys `nwc` znaku zdroje, nejvys `len` bajtu do `dst`
/// (znak, ktery se cely nevejde, se nezapise). Nulovy `dst` = jen pocitat.
///
/// # Safety
///
/// Kontrakt `wcsnrtombs`.
pub unsafe fn wcsnrtombs_enc(enc: Enc, dst: *mut c_char, src: *mut *const wchar_t, nwc: size_t, len: size_t, st: &mut MbState) -> size_t {
    if !st.is_initial() {
        return eilseq();
    }
    // SAFETY: `src` ukazuje na ukazatel na siroky retezec.
    let mut p = unsafe { *src };
    let mut count = 0usize;
    let mut left = nwc;
    while left > 0 {
        // SAFETY: retezec ma aspon `nwc` znaku nebo konci nulou drive.
        let wc = unsafe { *p } as u32;
        let mut b = [0u8; 4];
        let Some(k) = enc.encode(wc, &mut b) else {
            if !dst.is_null() {
                // SAFETY: viz vyse.
                unsafe { *src = p };
            }
            return eilseq();
        };
        if !dst.is_null() {
            if count + k > len {
                break;
            }
            // SAFETY: `dst` ma aspon `len` bajtu.
            unsafe { ptr::copy_nonoverlapping(b.as_ptr(), dst.add(count), k) };
        }
        if wc == 0 {
            if !dst.is_null() {
                // SAFETY: viz vyse.
                unsafe { *src = ptr::null() };
            }
            return count;
        }
        count += k;
        p = p.wrapping_add(1);
        left -= 1;
    }
    if !dst.is_null() {
        // SAFETY: viz vyse.
        unsafe { *src = p };
    }
    count
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn wcsnrtombs(dst: *mut c_char, src: *mut *const wchar_t, nwc: size_t, len: size_t, ps: *mut mbstate_t) -> size_t {
    let enc = locale::ctype();
    // SAFETY: kontrakt C.
    with_state(ps, &WCSRTOMBS, |st| unsafe { wcsnrtombs_enc(enc, dst, src, nwc, len, st) })
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn wcsrtombs(dst: *mut c_char, src: *mut *const wchar_t, len: size_t, ps: *mut mbstate_t) -> size_t {
    // SAFETY: kontrakt C.
    unsafe { wcsnrtombs(dst, src, usize::MAX, len, ps) }
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn wcstombs(dst: *mut c_char, src: *const wchar_t, len: size_t) -> size_t {
    let mut p = src;
    let mut st = MbState::INITIAL;
    // SAFETY: kontrakt C.
    unsafe { wcsnrtombs_enc(locale::ctype(), dst, &mut p, usize::MAX, len, &mut st) }
}

/// Vicebajtovy retezec -> siroke znaky (pro vnitrni pouziti: `fgetws`,
/// `wscanf`, testy); `None` = neplatne kodovani.
pub fn decode_all(enc: Enc, s: &[u8]) -> Option<Vec<u32>> {
    let mut st = MbState::INITIAL;
    let mut out = Vec::with_capacity(s.len());
    for &b in s {
        match enc.feed(&mut st, b) {
            cstd_core::mb::Step::Char(c) => out.push(c),
            cstd_core::mb::Step::More => {}
            cstd_core::mb::Step::Bad => return None,
        }
    }
    st.is_initial().then_some(out)
}

/// Siroke znaky -> vicebajtovy retezec; `None` = znak nejde zakodovat.
pub fn encode_all(enc: Enc, s: &[u32]) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(s.len());
    let mut b = [0u8; 4];
    for &c in s {
        let k = enc.encode(c, &mut b)?;
        out.extend_from_slice(&b[..k]);
    }
    Some(out)
}
