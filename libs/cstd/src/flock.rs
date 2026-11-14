//! Zamky souboru: `fcntl(F_GETLK/F_SETLK/F_SETLKW)` a `flock`.
//!
//! Skutecne zamky drzi fsd (byte-range, vlastnik = proces); dokud je
//! protokol FS nema, vraci `ENOLCK`.

use crate::{c_int, errno};

/// `struct flock` (jako Linux).
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct flock {
    pub l_type: i16,
    pub l_whence: i16,
    pub l_start: i64,
    pub l_len: i64,
    pub l_pid: i32,
}

pub const F_RDLCK: i16 = 0;
pub const F_WRLCK: i16 = 1;
pub const F_UNLCK: i16 = 2;

/// Zamky z `fcntl`.
///
/// # Safety
///
/// `p` ukazuje na `struct flock` (nebo NULL).
pub unsafe fn fcntl_lock(_fd: c_int, _cmd: c_int, p: *mut flock) -> c_int {
    if p.is_null() {
        return errno::ret(Err(errno::EFAULT));
    }
    errno::ret(Err(errno::ENOLCK))
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub extern "C" fn flock(_fd: c_int, _op: c_int) -> c_int {
    errno::ret(Err(errno::ENOLCK))
}
