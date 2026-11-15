//! `sys/stat.h`: `stat`, `fstat`, `lstat`, `mkdir`, `chmod`, `umask`.
//!
//! Prava v FS jsou granty, ne bity: `st_mode` je pohled na granty
//! (`fs::mode`: user = vlastnik, group = granty jinym prijemcum, other =
//! `EVERYONE`). Mod pri vytvoreni (`mkdir`, `open(O_CREAT)`) po `umask`
//! zdedene granty jen zuzi; `chmod` je meni (pravo `MANAGE`, jinak
//! `EPERM`) a smi i rozsirit. Bity setuid, setgid a sticky se ignoruji.

use std::sync::atomic::{AtomicU32, Ordering};

use crate::sys::{self, Kind, Stat};
use crate::{c_bytes, c_char, c_int, errno};

pub const S_IFMT: u32 = 0o170000;
pub const S_IFIFO: u32 = 0o010000;
pub const S_IFCHR: u32 = 0o020000;
pub const S_IFDIR: u32 = 0o040000;
pub const S_IFREG: u32 = 0o100000;
pub const S_IFLNK: u32 = 0o120000;
pub const S_IFSOCK: u32 = 0o140000;

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

/// Mod po `umask` (9 bitu).
pub fn masked(mode: u32) -> u16 {
    (mode & 0o777 & !UMASK.load(Ordering::Relaxed)) as u16
}

/// `st_mode` z druhu a prav.
pub fn mode_of(s: &Stat) -> u32 {
    // Odkaz nic neprava: 0777 jen jako zobrazeni (prava jsou na cili).
    if s.kind == Kind::Symlink {
        return S_IFLNK | 0o777;
    }
    if let Some(m) = s.mode {
        return type_bits(s.kind) | u32::from(m);
    }
    mode_from_rights(s)
}

fn type_bits(k: Kind) -> u32 {
    match k {
        Kind::File => S_IFREG,
        Kind::Dir => S_IFDIR,
        Kind::Symlink => S_IFLNK,
        Kind::Stream => S_IFCHR,
        Kind::Pipe => S_IFIFO,
        Kind::Socket => S_IFSOCK,
    }
}

/// `st_mode` z ucinnych prav (proudy, roury, FS bez modu).
fn mode_from_rights(s: &Stat) -> u32 {
    let (t, base) = match s.kind {
        Kind::File => (S_IFREG, 0o111),
        Kind::Dir => (S_IFDIR, 0o111),
        Kind::Symlink => (S_IFLNK, 0o777),
        Kind::Stream => (S_IFCHR, 0),
        Kind::Pipe => (S_IFIFO, 0),
        Kind::Socket => (S_IFSOCK, 0),
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

pub(crate) fn fill(r: sys::R<Stat>, out: *mut stat) -> c_int {
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

/// `lstat`: odkaz samotny (bez nasledovani posledni slozky).
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn lstat(path: *const c_char, out: *mut stat) -> c_int {
    // SAFETY: retezec C.
    fill(sys::lstat(unsafe { c_bytes(path) }), out)
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn fstat(fd: c_int, out: *mut stat) -> c_int {
    fill(sys::fstat(fd), out)
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn mkdir(path: *const c_char, mode: u32) -> c_int {
    // SAFETY: retezec C.
    errno::ret(sys::mkdir_mode(unsafe { c_bytes(path) }, masked(mode)))
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn chmod(path: *const c_char, mode: u32) -> c_int {
    // SAFETY: retezec C.
    errno::ret(sys::chmod(unsafe { c_bytes(path) }, (mode & 0o777) as u16))
}

/// `fchmod`: soubor podle cesty z doby otevreni (`File::path`); jiny fd
/// (roura, socket) jen overi cislo.
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub extern "C" fn fchmod(fd: c_int, mode: u32) -> c_int {
    match sys::fd_path(fd) {
        Ok(p) => errno::ret(sys::chmod(&p, (mode & 0o777) as u16)),
        Err(_) => errno::ret(sys::status(fd).map(|_| ())),
    }
}

static UMASK: AtomicU32 = AtomicU32::new(0o022);

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub extern "C" fn umask(mask: u32) -> u32 {
    UMASK.swap(mask & 0o777, Ordering::Relaxed)
}
