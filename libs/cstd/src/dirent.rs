//! `dirent.h`: `opendir`, `fdopendir`, `readdir`, `closedir`,
//! `rewinddir`, `dirfd`. Vypis se nacte pri otevreni (a pri
//! `rewinddir`) celym seznamem; prvni dve polozky jsou `.` a `..`.

use std::sync::Mutex;

use crate::sys::{self, Entry, Kind};
use crate::unistd::{O_CLOEXEC, O_DIRECTORY, O_RDONLY};
use crate::{c_char, c_int, errno};

pub const DT_UNKNOWN: u8 = 0;
pub const DT_FIFO: u8 = 1;
pub const DT_CHR: u8 = 2;
pub const DT_DIR: u8 = 4;
pub const DT_REG: u8 = 8;
pub const DT_LNK: u8 = 10;

/// `struct dirent` (jako Linux).
#[repr(C)]
pub struct dirent {
    pub d_ino: u64,
    pub d_off: i64,
    pub d_reclen: u16,
    pub d_type: u8,
    pub d_name: [u8; 256],
}

struct State {
    entries: Vec<Entry>,
    pos: usize,
    cur: dirent,
}

/// Otevreny adresar (`DIR *`).
pub struct DIR {
    fd: c_int,
    s: Mutex<State>,
}

fn d_type(k: Kind) -> u8 {
    match k {
        Kind::File => DT_REG,
        Kind::Dir => DT_DIR,
        Kind::Symlink => DT_LNK,
        Kind::Stream => DT_CHR,
        Kind::Pipe => DT_FIFO,
    }
}

/// Seznam s `.` a `..` na zacatku.
pub fn with_dots(mut list: Vec<Entry>) -> Vec<Entry> {
    let dot = |n: &[u8]| Entry { name: n.to_vec(), kind: Kind::Dir, ino: 0 };
    list.retain(|e| e.name != b"." && e.name != b"..");
    let mut out = vec![dot(b"."), dot(b"..")];
    out.extend(list);
    out
}

fn load(fd: c_int) -> sys::R<Vec<Entry>> {
    sys::list_dir(fd).map(with_dots)
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn fdopendir(fd: c_int) -> *mut DIR {
    match load(fd) {
        Ok(entries) => Box::into_raw(Box::new(DIR {
            fd,
            s: Mutex::new(State {
                entries,
                pos: 0,
                cur: dirent { d_ino: 0, d_off: 0, d_reclen: 0, d_type: 0, d_name: [0; 256] },
            }),
        })),
        Err(e) => {
            errno::set(e);
            core::ptr::null_mut()
        }
    }
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn opendir(path: *const c_char) -> *mut DIR {
    // SAFETY: kontrakt C.
    let fd = unsafe { crate::unistd::open(path, O_RDONLY | O_DIRECTORY | O_CLOEXEC, 0) };
    if fd < 0 {
        return core::ptr::null_mut();
    }
    // SAFETY: platne cislo fd.
    let d = unsafe { fdopendir(fd) };
    if d.is_null() {
        let _ = sys::close(fd);
    }
    d
}

/// Dalsi polozka; NULL = konec (errno beze zmeny) nebo chyba.
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn readdir(d: *mut DIR) -> *mut dirent {
    // SAFETY: `d` z opendir (kontrakt C).
    let Some(d) = (unsafe { d.as_ref() }) else {
        errno::set(errno::EBADF);
        return core::ptr::null_mut();
    };
    let mut s = d.s.lock().unwrap_or_else(|e| e.into_inner());
    let Some(e) = s.entries.get(s.pos).cloned() else { return core::ptr::null_mut() };
    s.pos += 1;
    let n = e.name.len().min(255);
    let pos = s.pos as i64;
    let cur = &mut s.cur;
    cur.d_ino = e.ino;
    cur.d_off = pos;
    cur.d_reclen = size_of::<dirent>() as u16;
    cur.d_type = d_type(e.kind);
    cur.d_name[..n].copy_from_slice(&e.name[..n]);
    cur.d_name[n] = 0;
    // Ukazatel plati do dalsiho readdir/closedir (POSIX).
    &mut s.cur as *mut dirent
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn closedir(d: *mut DIR) -> c_int {
    if d.is_null() {
        return errno::ret(Err(errno::EBADF));
    }
    // SAFETY: `d` vytvoril fdopendir (Box).
    let d = unsafe { Box::from_raw(d) };
    errno::ret(sys::close(d.fd))
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn rewinddir(d: *mut DIR) {
    // SAFETY: kontrakt C.
    if let Some(d) = unsafe { d.as_ref() } {
        let mut s = d.s.lock().unwrap_or_else(|e| e.into_inner());
        if let Ok(list) = load(d.fd) {
            s.entries = list;
        }
        s.pos = 0;
    }
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn dirfd(d: *mut DIR) -> c_int {
    // SAFETY: kontrakt C.
    match unsafe { d.as_ref() } {
        Some(d) => d.fd,
        None => errno::ret(Err(errno::EINVAL)),
    }
}
