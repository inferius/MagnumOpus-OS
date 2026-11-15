//! `sys/statfs.h` (`sys/vfs.h`): `statfs`, `fstatfs` jako Linux na 64
//! bitech (stejne na x86_64 i aarch64, test `statfs_layout_and_errors`).
//! `statvfs` je v `statvfs.rs` (vetev 7a); obe plni `sys::volume_space`.
//!
//! Vsechny cesty procesu jsou na svazku relace FS (fsproto `INFO`), takze
//! cesta nebo fd jen overi platnost. Pocty souboru (`f_files`) FS nema
//! pevne dane: 0 jako u btrfs.

use crate::{c_bytes, c_char, c_int, c_long, errno, sys};

/// `f_type` svazku MagnumOpus ("MOFS").
pub const MOFS_MAGIC: c_long = 0x4D4F_4653;
/// Nejdelsi jmeno polozky (`NAME_MAX`).
pub const NAME_MAX: c_long = 255;

#[repr(C)]
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct fsid_t {
    pub val: [c_int; 2],
}

/// `struct statfs` (120 B).
#[repr(C)]
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct statfs {
    pub f_type: c_long,
    pub f_bsize: c_long,
    pub f_blocks: u64,
    pub f_bfree: u64,
    pub f_bavail: u64,
    pub f_files: u64,
    pub f_ffree: u64,
    pub f_fsid: fsid_t,
    pub f_namelen: c_long,
    pub f_frsize: c_long,
    pub f_flags: c_long,
    pub f_spare: [c_long; 4],
}

/// Popis svazku z (velikost bloku, bloku, volnych bloku).
pub fn from_space((block, blocks, free): (u32, u64, u64)) -> statfs {
    statfs {
        f_type: MOFS_MAGIC,
        f_bsize: block as c_long,
        f_blocks: blocks,
        f_bfree: free.min(blocks),
        f_bavail: free.min(blocks),
        f_namelen: NAME_MAX,
        f_frsize: block as c_long,
        ..statfs::default()
    }
}

/// Po kontrole cesty/fd (`r`) zapise svazek do `out` (NULL = `EFAULT`).
fn fill(r: Result<(), c_int>, out: *mut statfs) -> c_int {
    match r.and_then(|()| sys::volume_space()).map(from_space) {
        Ok(v) => {
            if out.is_null() {
                return errno::ret(Err(errno::EFAULT));
            }
            // SAFETY: `out` je platny ukazatel (kontrakt C).
            unsafe { out.write(v) };
            0
        }
        Err(e) => errno::ret(Err(e)),
    }
}

/// # Safety
///
/// `path` je retezec C, `out` NULL nebo platny ukazatel.
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn statfs(path: *const c_char, out: *mut statfs) -> c_int {
    // SAFETY: retezec C.
    fill(sys::stat(unsafe { c_bytes(path) }).map(|_| ()), out)
}

/// # Safety
///
/// `out` je NULL nebo platny ukazatel.
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn fstatfs(fd: c_int, out: *mut statfs) -> c_int {
    fill(sys::fstat(fd).map(|_| ()), out)
}
