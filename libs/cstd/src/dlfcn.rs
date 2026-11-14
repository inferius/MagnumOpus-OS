//! `dlfcn.h`: `dlopen`, `dlsym`, `dlclose`, `dlerror`, `dladdr`,
//! `dl_iterate_phdr` nad tabulkou zavadece ld.so (DL-4b, knihovny z
//! `/system/lib` pres sluzbu libd). Staticky program zavadec nema:
//! `dlopen` vrati NULL a `dlerror` duvod.

use core::ffi::c_void;

use crate::{c_char, c_int};

/// Tabulka zavadece (jen program PIE pod ld.so).
#[cfg(target_os = "magnumopus")]
fn table() -> Option<&'static std::os::magnumopus::runtime::dl::Interface> {
    std::os::magnumopus::runtime::dl::interface()
}

static NO_LOADER: &[u8] = b"dlopen: program je staticky (bez zavadece ld.so)\0";

std::thread_local! {
    /// Chyba bez zavadece (`dlerror` ji vrati jednou).
    static ERR: core::cell::Cell<bool> = const { core::cell::Cell::new(false) };
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn dlopen(path: *const c_char, flags: c_int) -> *mut c_void {
    #[cfg(target_os = "magnumopus")]
    if let Some(t) = table() {
        // SAFETY: funkce zavadece, retezec C od volajiciho.
        return unsafe { (t.dlopen)(path.cast(), flags) }.cast();
    }
    let _ = (path, flags);
    ERR.with(|e| e.set(true));
    core::ptr::null_mut()
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn dlsym(handle: *mut c_void, name: *const c_char) -> *mut c_void {
    #[cfg(target_os = "magnumopus")]
    if let Some(t) = table() {
        // SAFETY: funkce zavadece.
        return unsafe { (t.dlsym)(handle.cast(), name.cast()) }.cast();
    }
    let _ = (handle, name);
    ERR.with(|e| e.set(true));
    core::ptr::null_mut()
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn dlclose(handle: *mut c_void) -> c_int {
    #[cfg(target_os = "magnumopus")]
    if let Some(t) = table() {
        // SAFETY: funkce zavadece.
        return unsafe { (t.dlclose)(handle.cast()) };
    }
    let _ = handle;
    -1
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub extern "C" fn dlerror() -> *const c_char {
    #[cfg(target_os = "magnumopus")]
    if let Some(t) = table() {
        // SAFETY: funkce zavadece.
        return unsafe { (t.dlerror)() }.cast();
    }
    if ERR.with(|e| e.replace(false)) { NO_LOADER.as_ptr() } else { core::ptr::null() }
}

/// `Dl_info` (jako glibc).
#[repr(C)]
pub struct Dl_info {
    pub dli_fname: *const c_char,
    pub dli_fbase: *mut c_void,
    pub dli_sname: *const c_char,
    pub dli_saddr: *mut c_void,
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn dladdr(addr: *const c_void, info: *mut Dl_info) -> c_int {
    #[cfg(target_os = "magnumopus")]
    if let Some(t) = table() {
        // Rozlozeni Dl_info = abi::loader::DlInfo (4 slova).
        // SAFETY: funkce zavadece, `info` od volajiciho.
        return unsafe { (t.dladdr)(addr.cast(), info.cast()) };
    }
    let _ = (addr, info);
    0
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn dl_iterate_phdr(
    cb: unsafe extern "C" fn(*mut c_void, usize, *mut c_void) -> c_int,
    data: *mut c_void,
) -> c_int {
    #[cfg(target_os = "magnumopus")]
    if let Some(t) = table() {
        // SAFETY: funkce zavadece; zpetne volani ma rozlozeni PhdrCallback.
        return unsafe { (t.dl_iterate_phdr)(core::mem::transmute(cb), data.cast()) };
    }
    let _ = (cb, data);
    0
}
