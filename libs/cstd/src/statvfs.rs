//! `sys/statvfs.h`: velikost svazku relace fsd (`std::filesystem::space`,
//! `df`). Cesta se jen overi (`stat`): relace ma jeden svazek.

use crate::{c_bytes, c_char, c_int, errno, sys};

/// Rozlozeni jako `include/sys/statvfs.h` (11 slov).
#[repr(C)]
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct statvfs {
    pub f_bsize: u64,
    pub f_frsize: u64,
    pub f_blocks: u64,
    pub f_bfree: u64,
    pub f_bavail: u64,
    pub f_files: u64,
    pub f_ffree: u64,
    pub f_favail: u64,
    pub f_fsid: u64,
    pub f_flag: u64,
    pub f_namemax: u64,
}

/// Nejdelsi jmeno polozky, ktere hlasime (konzervativne jako Linux).
pub const NAME_MAX: u64 = 255;

/// Popis svazku z (velikost bloku, bloku, volnych bloku).
pub fn from_space((block, blocks, free): (u32, u64, u64)) -> statvfs {
    statvfs {
        f_bsize: u64::from(block),
        f_frsize: u64::from(block),
        f_blocks: blocks,
        f_bfree: free.min(blocks),
        f_bavail: free.min(blocks),
        f_namemax: NAME_MAX,
        ..Default::default()
    }
}

fn fill(r: Result<(), c_int>, out: *mut statvfs) -> c_int {
    let r = r.and_then(|()| sys::volume_space()).map(from_space);
    match r {
        Ok(v) => {
            if out.is_null() {
                return errno::ret(Err(errno::EFAULT));
            }
            // SAFETY: kontrakt C.
            unsafe { *out = v };
            0
        }
        Err(e) => errno::ret(Err(e)),
    }
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn statvfs(path: *const c_char, out: *mut statvfs) -> c_int {
    // SAFETY: retezec C.
    fill(sys::stat(unsafe { c_bytes(path) }).map(|_| ()), out)
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn fstatvfs(fd: c_int, out: *mut statvfs) -> c_int {
    fill(sys::fstat(fd).map(|_| ()), out)
}
