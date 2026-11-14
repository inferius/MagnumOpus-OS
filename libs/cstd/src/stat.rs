//! `sys/stat.h`: `stat`, `fstat`, `lstat`, `mkdir`, `chmod`, `umask`.
//!
//! Prava v FS jsou granty, ne bity: `st_mode` se sklada z druhu objektu
//! a ucinnych prav relace (cteni, zapis). `chmod` nema ucinek (uspeje,
//! kdyz objekt existuje), prava se meni popisem opravneni (ACL).

use std::sync::atomic::{AtomicU32, Ordering};

use crate::sys::{self, Kind, Stat};
use crate::{c_bytes, c_char, c_int, errno};

pub const S_IFMT: u32 = 0o170000;
pub const S_IFIFO: u32 = 0o010000;
pub const S_IFCHR: u32 = 0o020000;
pub const S_IFDIR: u32 = 0o040000;
pub const S_IFREG: u32 = 0o100000;
pub const S_IFLNK: u32 = 0o120000;

/// `struct timespec`.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct timespec {
    pub tv_sec: i64,
    pub tv_nsec: i64,
}

/// `struct stat` (vlastni rozlozeni, stejne na obou architekturach; viz
/// `include/sys/stat.h`).
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct stat {
    pub st_dev: u64,
    pub st_ino: u64,
    pub st_mode: u32,
    pub st_nlink: u32,
    pub st_uid: u32,
    pub st_gid: u32,
    pub st_rdev: u64,
    pub st_size: i64,
    pub st_blksize: i64,
    pub st_blocks: i64,
    pub st_atim: timespec,
    pub st_mtim: timespec,
    pub st_ctim: timespec,
}

/// `st_mode` z druhu a prav.
pub fn mode_of(s: &Stat) -> u32 {
    let (t, base) = match s.kind {
        Kind::File => (S_IFREG, 0o111),
        Kind::Dir => (S_IFDIR, 0o111),
        Kind::Symlink => (S_IFLNK, 0o777),
        Kind::Stream => (S_IFCHR, 0),
        Kind::Pipe => (S_IFIFO, 0),
    };
    let mut m = t;
    if s.kind == Kind::Symlink {
        return m | base;
    }
    if s.readable {
        m |= 0o444;
    }
    if s.writable {
        m |= 0o200;
    }
    // Spousteni neurcuje bit (manifest a NOEXEC datasetu); adresar je
    // prochazetelny, soubor "spustitelny" jen u programu - bit nevime.
    if s.kind == Kind::Dir {
        m |= base;
    }
    m
}

/// `struct stat` z metadat.
pub fn to_c(s: &Stat) -> stat {
    let ts = |(sec, ns): (i64, u32)| timespec { tv_sec: sec, tv_nsec: i64::from(ns) };
    stat {
        st_dev: 1,
        st_ino: s.ino,
        st_mode: mode_of(s),
        st_nlink: 1,
        st_uid: u32::try_from(s.uid).unwrap_or(u32::MAX),
        st_gid: 0,
        st_rdev: 0,
        st_size: s.size as i64,
        st_blksize: 4096,
        st_blocks: s.size.div_ceil(512) as i64,
        st_atim: ts(s.mtime),
        st_mtim: ts(s.mtime),
        st_ctim: ts(s.ctime),
    }
}

fn fill(r: sys::R<Stat>, out: *mut stat) -> c_int {
    if out.is_null() {
        return errno::ret(Err(errno::EFAULT));
    }
    match r {
        Ok(s) => {
            // SAFETY: `out` ukazuje na `struct stat` (kontrakt C).
            unsafe { *out = to_c(&s) };
            0
        }
        Err(e) => errno::ret(Err(e)),
    }
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn stat(path: *const c_char, out: *mut stat) -> c_int {
    // SAFETY: retezec C.
    fill(sys::stat(unsafe { c_bytes(path) }), out)
}

/// Symlinky FS zatim nevytvari, `lstat` = `stat`.
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn lstat(path: *const c_char, out: *mut stat) -> c_int {
    // SAFETY: kontrakt C.
    unsafe { stat(path, out) }
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn fstat(fd: c_int, out: *mut stat) -> c_int {
    fill(sys::fstat(fd), out)
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn mkdir(path: *const c_char, _mode: u32) -> c_int {
    // SAFETY: retezec C.
    errno::ret(sys::mkdir(unsafe { c_bytes(path) }))
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn chmod(path: *const c_char, _mode: u32) -> c_int {
    // SAFETY: retezec C.
    errno::ret(sys::stat(unsafe { c_bytes(path) }).map(|_| ()))
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub extern "C" fn fchmod(fd: c_int, _mode: u32) -> c_int {
    errno::ret(sys::status(fd).map(|_| ()))
}

static UMASK: AtomicU32 = AtomicU32::new(0o022);

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub extern "C" fn umask(mask: u32) -> u32 {
    UMASK.swap(mask & 0o777, Ordering::Relaxed)
}
