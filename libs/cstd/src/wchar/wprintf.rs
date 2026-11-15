//! Rodina `wprintf` (`swprintf`, `fwprintf`, `wprintf` a `v`-varianty) nad
//! formatovacim strojem cstd-core (`format::wformat`). Vystup do proudu se
//! zakoduje podle locale vlakna.

use core::ffi::VaList;

use cstd_core::format::{self, Buf, Out, VaArgs};

use super::{wchar_t, wslice};
use crate::stdio::{FILE, lock_file, stdout_ptr};
use crate::{c_int, errno, locale, size_t};

struct WVec(Vec<u32>);

impl Out<u32> for WVec {
    fn put(&mut self, c: u32) {
        self.0.push(c);
    }
}

/// Siroky format jako znaky `u32`.
///
/// # Safety
///
/// `fmt` je siroky retezec zakonceny nulou.
unsafe fn wfmt(fmt: *const wchar_t) -> Vec<u32> {
    // SAFETY: kontrakt funkce.
    unsafe { wslice(fmt) }.iter().map(|&c| c as u32).collect()
}

fn clamp(n: usize) -> c_int {
    c_int::try_from(n).unwrap_or(c_int::MAX)
}

/// `vswprintf`: zapise nejvys `n - 1` znaku a nulu. Na rozdil od
/// `vsnprintf` vraci -1, kdyz se vystup s nulou nevejde.
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn vswprintf(buf: *mut wchar_t, n: size_t, fmt: *const wchar_t, mut ap: VaList) -> c_int {
    // SAFETY: kontrakt C.
    let f = unsafe { wfmt(fmt) };
    let mut tmp = vec![0u32; n.saturating_sub(1)];
    let mut out = Buf { buf: &mut tmp, pos: 0, total: 0 };
    let r = format::wformat(&mut out, &f, &mut VaArgs(&mut ap), locale::ctype());
    let (pos, total) = (out.pos, out.total);
    if n > 0 && !buf.is_null() {
        for (i, &c) in tmp[..pos].iter().enumerate() {
            // SAFETY: `buf` ma `n` znaku.
            unsafe { *buf.add(i) = c as wchar_t };
        }
        // SAFETY: viz vyse.
        unsafe { *buf.add(pos) = 0 };
    }
    if r.is_err() {
        errno::set(errno::EILSEQ);
        return -1;
    }
    if total >= n {
        // Nevejde se (standard nevyzaduje errno; glibc dava EOVERFLOW).
        errno::set(errno::EOVERFLOW);
        return -1;
    }
    clamp(total)
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn swprintf(buf: *mut wchar_t, n: size_t, fmt: *const wchar_t, args: ...) -> c_int {
    // SAFETY: kontrakt C.
    unsafe { vswprintf(buf, n, fmt, args) }
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn vfwprintf(f: *mut FILE, fmt: *const wchar_t, mut ap: VaList) -> c_int {
    // SAFETY: kontrakt C.
    let fm = unsafe { wfmt(fmt) };
    let enc = locale::ctype();
    let mut out = WVec(Vec::new());
    let r = format::wformat(&mut out, &fm, &mut VaArgs(&mut ap), enc);
    // SAFETY: kontrakt C.
    let Some(mut s) = (unsafe { lock_file(f) }) else { return -1 };
    s.orient(1);
    let Some(bytes) = super::conv::encode_all(enc, &out.0) else {
        s.set_error();
        errno::set(errno::EILSEQ);
        return -1;
    };
    // Vystup do chyby prevodu se zapise (jako glibc), pak -1.
    if s.write(&bytes).is_err() {
        return -1;
    }
    if r.is_err() {
        errno::set(errno::EILSEQ);
        return -1;
    }
    clamp(out.0.len())
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn fwprintf(f: *mut FILE, fmt: *const wchar_t, args: ...) -> c_int {
    // SAFETY: kontrakt C.
    unsafe { vfwprintf(f, fmt, args) }
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn vwprintf(fmt: *const wchar_t, ap: VaList) -> c_int {
    // SAFETY: stdout je platny FILE.
    unsafe { vfwprintf(stdout_ptr(), fmt, ap) }
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn wprintf(fmt: *const wchar_t, args: ...) -> c_int {
    // SAFETY: kontrakt C.
    unsafe { vfwprintf(stdout_ptr(), fmt, args) }
}
