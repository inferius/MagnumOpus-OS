//! `magnumopus/thread.h`: rozsireni cstd pro vlakna (ne POSIX) - druhy
//! ukazatel vlakna (x86_64 baze GS programu pro TEB Windows vrstvy).
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
