//! Siroky vstup a vystup nad `FILE`: `fgetwc`, `fputwc`, `fgetws`,
//! `fputws`, `ungetwc`, `fwide`. Znaky se v proudu ukladaji v kodovani
//! locale vlakna (`LC_CTYPE`); rozpracovany znak cteni si pamatuje proud.
//!
//! Orientace (`fwide`) se nastavi prvni sirokou operaci; bajtove funkce ji
//! nemeni ani nekontroluji (jako glibc v praxi - smichani je chyba
//! programu, ne nase).

use cstd_core::mb::{MbState, Step};

use super::{WEOF, wchar_t, wint_t, wlen};
use crate::stdio::{FILE, Locked, lock_file, stdin_ptr, stdout_ptr};
use crate::{c_int, errno, locale};

/// Precte jeden siroky znak ze zamceneho proudu.
pub(crate) fn getwc_locked(s: &mut Locked) -> wint_t {
    s.orient(1);
    let enc = locale::ctype();
    let mut st: MbState = *s.wstate();
    loop {
        let Some(b) = s.getc() else {
            // Konec uprostred znaku = chyba kodovani.
            if !st.is_initial() {
                *s.wstate() = MbState::INITIAL;
                s.set_error();
                errno::set(errno::EILSEQ);
            }
            return WEOF;
        };
        match enc.feed(&mut st, b) {
            Step::Char(c) => {
                *s.wstate() = MbState::INITIAL;
                return c;
            }
            Step::More => *s.wstate() = st,
            Step::Bad => {
                *s.wstate() = MbState::INITIAL;
                s.set_error();
                errno::set(errno::EILSEQ);
                return WEOF;
            }
        }
    }
}

/// Zapise jeden siroky znak do zamceneho proudu.
pub(crate) fn putwc_locked(s: &mut Locked, c: wint_t) -> wint_t {
    s.orient(1);
    let mut b = [0u8; 4];
    let Some(k) = locale::ctype().encode(c, &mut b) else {
        s.set_error();
        errno::set(errno::EILSEQ);
        return WEOF;
    };
    match s.write(&b[..k]) {
        Ok(()) => c,
        Err(_) => WEOF,
    }
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn fgetwc(f: *mut FILE) -> wint_t {
    // SAFETY: kontrakt C.
    let Some(mut s) = (unsafe { lock_file(f) }) else { return WEOF };
    getwc_locked(&mut s)
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn getwc(f: *mut FILE) -> wint_t {
    // SAFETY: kontrakt C.
    unsafe { fgetwc(f) }
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub extern "C" fn getwchar() -> wint_t {
    // SAFETY: stdin je platny FILE.
    unsafe { fgetwc(stdin_ptr()) }
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn fputwc(c: wchar_t, f: *mut FILE) -> wint_t {
    // SAFETY: kontrakt C.
    let Some(mut s) = (unsafe { lock_file(f) }) else { return WEOF };
    putwc_locked(&mut s, c as wint_t)
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn putwc(c: wchar_t, f: *mut FILE) -> wint_t {
    // SAFETY: kontrakt C.
    unsafe { fputwc(c, f) }
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub extern "C" fn putwchar(c: wchar_t) -> wint_t {
    // SAFETY: stdout je platny FILE.
    unsafe { fputwc(c, stdout_ptr()) }
}

/// `ungetwc`: vrati znak (zakodovany) zpet do proudu.
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn ungetwc(c: wint_t, f: *mut FILE) -> wint_t {
    if c == WEOF {
        return WEOF;
    }
    // SAFETY: kontrakt C.
    let Some(mut s) = (unsafe { lock_file(f) }) else { return WEOF };
    s.orient(1);
    let mut b = [0u8; 4];
    let Some(k) = locale::ctype().encode(c, &mut b) else { return WEOF };
    s.unget(&b[..k]);
    c
}

/// Radek sirokych znaku vcetne `\n`, nejvys `n - 1` znaku, zakonceny nulou.
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn fgetws(buf: *mut wchar_t, n: c_int, f: *mut FILE) -> *mut wchar_t {
    if n <= 0 || buf.is_null() {
        return core::ptr::null_mut();
    }
    // SAFETY: kontrakt C.
    let Some(mut s) = (unsafe { lock_file(f) }) else { return core::ptr::null_mut() };
    let mut i = 0usize;
    let mut failed = false;
    while i + 1 < n as usize {
        let c = getwc_locked(&mut s);
        if c == WEOF {
            failed = errno::get() == errno::EILSEQ;
            break;
        }
        // SAFETY: `buf` ma `n` znaku.
        unsafe { *buf.add(i) = c as wchar_t };
        i += 1;
        if c == b'\n' as u32 {
            break;
        }
    }
    if i == 0 || failed {
        return core::ptr::null_mut();
    }
    // SAFETY: viz vyse.
    unsafe { *buf.add(i) = 0 };
    buf
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn fputws(ws: *const wchar_t, f: *mut FILE) -> c_int {
    // SAFETY: kontrakt C.
    let Some(mut s) = (unsafe { lock_file(f) }) else { return -1 };
    s.orient(1);
    // SAFETY: siroky retezec zakonceny nulou.
    let chars = unsafe { core::slice::from_raw_parts(ws, wlen(ws)) };
    let wide: Vec<u32> = chars.iter().map(|&c| c as u32).collect();
    let Some(bytes) = super::conv::encode_all(locale::ctype(), &wide) else {
        s.set_error();
        errno::set(errno::EILSEQ);
        return -1;
    };
    match s.write(&bytes) {
        Ok(()) => 0,
        Err(_) => -1,
    }
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn fwide(f: *mut FILE, mode: c_int) -> c_int {
    // SAFETY: kontrakt C.
    let Some(mut s) = (unsafe { lock_file(f) }) else { return 0 };
    s.orient(mode)
}
