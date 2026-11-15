//! Funkce bez parametru delky pro porty, s pojistkou, kde ji jde dat
//! (docs/cs/cstd.md, kapitola Doplnek POSIX):
//!
//! - `sprintf`, `vsprintf`: hlavicka v C je mapuje na `__sprintf_chk` /
//!   `__vsprintf_chk` s `__builtin_object_size(buf, 1)` (jako
//!   `_FORTIFY_SOURCE` v glibc). Zna-li prekladac velikost bufferu,
//!   preteceni = `abort` se zpravou na stderr misto prepsani pameti; nezna-li
//!   (`(size_t)-1`), chova se jako bezny `sprintf`.
//! - `strtok`: stav v TLS (kazde vlakno vlastni), tedy bezpecny i ve
//!   vlaknech; jinak jako `strtok_r`.

use core::ffi::VaList;
use std::cell::Cell;
use std::io::Write as _;

use cstd_core::format::{self, Out, VaArgs};

use crate::{c_bytes, c_char, c_int, errno, size_t};

/// Vystup formatu do pameti.
struct VecOut(Vec<u8>);

impl Out for VecOut {
    fn put(&mut self, b: u8) {
        self.0.push(b);
    }
}

/// Velikost "nevim" z `__builtin_object_size`.
pub const UNKNOWN: size_t = size_t::MAX;

/// Naformatuje do `Vec`; chyba kodovani = `EILSEQ`.
///
/// # Safety
///
/// `fmt` je retezec C a `ap` odpovida formatu.
unsafe fn render(fmt: *const c_char, ap: &mut VaList) -> Result<Vec<u8>, c_int> {
    if fmt.is_null() {
        return Err(errno::EINVAL);
    }
    // SAFETY: retezec C.
    let f = unsafe { c_bytes(fmt) };
    let mut out = VecOut(Vec::new());
    format::format_enc(&mut out, f, &mut VaArgs(ap), crate::locale::ctype()).map_err(|_| errno::EILSEQ)?;
    Ok(out.0)
}

/// Preteceni bufferu: zprava a konec procesu (jako glibc).
pub fn overflow(func: &str) -> ! {
    let _ = writeln!(std::io::stderr(), "*** buffer overflow detected ***: {func}");
    crate::stdlib::abort()
}

/// Spolecne jadro: zapise text a nulu do `buf` (nejvys `size` bajtu, jinak
/// `abort`); vraci delku bez nuly.
///
/// # Safety
///
/// `buf` ma aspon delku vysledku + 1 bajt (nebo `size` bajtu, je-li zname).
unsafe fn write_out(func: &str, buf: *mut c_char, size: size_t, fmt: *const c_char, ap: &mut VaList) -> c_int {
    if buf.is_null() {
        errno::set(errno::EINVAL);
        return -1;
    }
    // SAFETY: kontrakt C.
    let text = match unsafe { render(fmt, ap) } {
        Ok(t) => t,
        Err(e) => {
            errno::set(e);
            return -1;
        }
    };
    if size != UNKNOWN && text.len() >= size {
        overflow(func);
    }
    let Ok(n) = c_int::try_from(text.len()) else {
        errno::set(errno::EOVERFLOW);
        return -1;
    };
    // SAFETY: buffer staci (kontrakt C, u znameho `size` overeno vyse).
    unsafe {
        core::ptr::copy_nonoverlapping(text.as_ptr(), buf, text.len());
        *buf.add(text.len()) = 0;
    }
    n
}

/// # Safety
///
/// `buf` staci na vysledek; `fmt` a `ap` podle normy C.
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn __vsprintf_chk(buf: *mut c_char, _flag: c_int, size: size_t, fmt: *const c_char, mut ap: VaList) -> c_int {
    // SAFETY: kontrakt C.
    unsafe { write_out("vsprintf", buf, size, fmt, &mut ap) }
}

/// # Safety
///
/// Jako `__vsprintf_chk`.
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn __sprintf_chk(buf: *mut c_char, _flag: c_int, size: size_t, fmt: *const c_char, mut args: ...) -> c_int {
    // SAFETY: kontrakt C.
    unsafe { write_out("sprintf", buf, size, fmt, &mut args) }
}

/// `vsprintf` bez znalosti velikosti (vola ho kod, ktery hlavicku obesel,
/// a C++).
///
/// # Safety
///
/// `buf` staci na vysledek.
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn vsprintf(buf: *mut c_char, fmt: *const c_char, mut ap: VaList) -> c_int {
    // SAFETY: kontrakt C.
    unsafe { write_out("vsprintf", buf, UNKNOWN, fmt, &mut ap) }
}

/// # Safety
///
/// `buf` staci na vysledek.
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn sprintf(buf: *mut c_char, fmt: *const c_char, mut args: ...) -> c_int {
    // SAFETY: kontrakt C.
    unsafe { write_out("sprintf", buf, UNKNOWN, fmt, &mut args) }
}

thread_local! {
    /// Stav `strtok` aktualniho vlakna.
    static STRTOK: Cell<*mut c_char> = const { Cell::new(core::ptr::null_mut()) };
}

/// `strtok`: jako `strtok_r` se stavem vlakna.
///
/// # Safety
///
/// `s` (nebo retezec z predchoziho volani ve vlakne) a `delim` jsou
/// retezce C; retezec zije mezi volanimi.
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn strtok(s: *mut c_char, delim: *const c_char) -> *mut c_char {
    STRTOK.with(|st| {
        let mut save = st.get();
        // SAFETY: kontrakt C.
        let r = unsafe { crate::misc::strtok_r(s, delim, &mut save) };
        st.set(save);
        r
    })
}
