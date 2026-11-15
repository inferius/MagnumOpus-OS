//! `magnumopus/thread.h`: rozsireni cstd pro vlakna (ne POSIX) - druhy
//! ukazatel vlakna (x86_64 baze GS programu pro TEB Windows vrstvy).
//! Navic `ffs` (strings.h), ktery unixova cast Wine potrebuje vsude.
//!
//! Program bazi GS sam necte (CR4.FSGSBASE je vypnute), proto si cstd
//! pamatuje posledni nastavenou hodnotu vlakna v TLS.

use core::ffi::c_void;
use std::cell::Cell;

use crate::{c_int, errno};

std::thread_local! {
    static SECOND: Cell<usize> = const { Cell::new(0) };
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub extern "C" fn mos_thread_set_second_pointer(value: *mut c_void) -> c_int {
    match crate::sys::set_second_pointer(value as u64) {
        Ok(()) => {
            SECOND.with(|s| s.set(value as usize));
            0
        }
        Err(e) => errno::ret(Err(e)),
    }
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub extern "C" fn mos_thread_get_second_pointer() -> *mut c_void {
    SECOND.with(|s| s.get()) as *mut c_void
}

// --- ffs (strings.h, POSIX; Wine unix_private.h) -------------------------

/// Index nejnizsiho nastaveneho bitu od 1, 0 = zadny bit.
pub fn ffs_u64(v: u64) -> i32 {
    if v == 0 { 0 } else { v.trailing_zeros() as i32 + 1 }
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub extern "C" fn ffs(v: c_int) -> c_int {
    ffs_u64(u64::from(v as u32))
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub extern "C" fn ffsl(v: core::ffi::c_long) -> c_int {
    ffs_u64(v as u64)
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub extern "C" fn ffsll(v: i64) -> c_int {
    ffs_u64(v as u64)
}
