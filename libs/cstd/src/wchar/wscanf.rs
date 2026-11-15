//! Rodina `wscanf` (`swscanf`, `fwscanf`, `wscanf` a `v`-varianty) nad
//! motorem `scanf` v sirokem rezimu.

use core::ffi::VaList;

use super::wio::getwc_locked;
use super::{WEOF, wchar_t, wslice};
use crate::scanf::{Input, Mode, Units, VaSink, scan_units};
use crate::stdio::{FILE, lock_file, stdin_ptr};
use crate::{c_int, locale};

/// Siroky proud `FILE` s nahledem jednoho znaku.
struct WStream {
    f: *mut FILE,
    look: Option<Option<u32>>,
    n: usize,
}

impl Input for WStream {
    fn peek(&mut self) -> Option<u32> {
        if self.look.is_none() {
            // SAFETY: platny FILE (kontrakt C).
            let c = unsafe { lock_file(self.f) }.map_or(WEOF, |mut s| getwc_locked(&mut s));
            self.look = Some((c != WEOF).then_some(c));
        }
        self.look.flatten()
    }
    fn next(&mut self) -> Option<u32> {
        let c = self.peek();
        self.look = None;
        if c.is_some() {
            self.n += 1;
        }
        c
    }
    fn count(&self) -> usize {
        self.n
    }
}

impl Drop for WStream {
    fn drop(&mut self) {
        if let Some(Some(c)) = self.look {
            // SAFETY: platny FILE.
            unsafe { super::wio::ungetwc(c, self.f) };
        }
    }
}

fn mode() -> Mode {
    Mode { wide: true, enc: locale::ctype() }
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

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn vswscanf(s: *const wchar_t, fmt: *const wchar_t, mut ap: VaList) -> c_int {
    // SAFETY: siroke retezce C.
    let (src, f) = unsafe { (wfmt(s), wfmt(fmt)) };
    scan_units(mode(), &f, &mut Units { s: &src, pos: 0 }, &mut VaSink(&mut ap))
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn swscanf(s: *const wchar_t, fmt: *const wchar_t, args: ...) -> c_int {
    // SAFETY: kontrakt C.
    unsafe { vswscanf(s, fmt, args) }
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn vfwscanf(f: *mut FILE, fmt: *const wchar_t, mut ap: VaList) -> c_int {
    // SAFETY: siroky retezec C.
    let fm = unsafe { wfmt(fmt) };
    scan_units(mode(), &fm, &mut WStream { f, look: None, n: 0 }, &mut VaSink(&mut ap))
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn fwscanf(f: *mut FILE, fmt: *const wchar_t, args: ...) -> c_int {
    // SAFETY: kontrakt C.
    unsafe { vfwscanf(f, fmt, args) }
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn vwscanf(fmt: *const wchar_t, ap: VaList) -> c_int {
    // SAFETY: stdin je platny FILE.
    unsafe { vfwscanf(stdin_ptr(), fmt, ap) }
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn wscanf(fmt: *const wchar_t, args: ...) -> c_int {
    // SAFETY: kontrakt C.
    unsafe { vfwscanf(stdin_ptr(), fmt, args) }
}

/// Testovaci vstup ze sirokych znaku (host testy).
pub fn scan_wide(s: &[u32], fmt: &[u32], out: &mut dyn crate::scanf::Sink) -> c_int {
    scan_units(mode(), fmt, &mut Units { s, pos: 0 }, out)
}
