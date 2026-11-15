//! `unistd.h` a `fcntl.h`: cisla fd (`open`, `read`, `write`, `lseek`,
//! `dup`, `pipe`, `fcntl`), soubory podle cesty, pracovni adresar
//! a procesy bez `fork`.
//!
//! Priznaky `O_*` a prikazy `F_*` maji hodnoty jako Linux (stejne na obou
//! architekturach, i `O_DIRECTORY`); vlastni hlavicky je deklaruji stejne.

use crate::sys::{self, OpenFlags, Seek};
use crate::{c_bytes, c_char, c_int, errno, size_t};

pub const O_RDONLY: c_int = 0;
pub const O_WRONLY: c_int = 1;
pub const O_RDWR: c_int = 2;
pub const O_ACCMODE: c_int = 3;
pub const O_CREAT: c_int = 0o100;
pub const O_EXCL: c_int = 0o200;
pub const O_NOCTTY: c_int = 0o400;
pub const O_TRUNC: c_int = 0o1000;
pub const O_APPEND: c_int = 0o2000;
pub const O_NONBLOCK: c_int = 0o4000;
pub const O_DIRECTORY: c_int = 0o200000;
pub const O_NOFOLLOW: c_int = 0o400000;
pub const O_CLOEXEC: c_int = 0o2000000;

pub const F_DUPFD: c_int = 0;
pub const F_GETFD: c_int = 1;
pub const F_SETFD: c_int = 2;
pub const F_GETFL: c_int = 3;
pub const F_SETFL: c_int = 4;
pub const F_GETLK: c_int = 5;
pub const F_SETLK: c_int = 6;
pub const F_SETLKW: c_int = 7;
pub const F_DUPFD_CLOEXEC: c_int = 1030;
pub const FD_CLOEXEC: c_int = 1;

pub const AT_FDCWD: c_int = -100;

pub const R_OK: c_int = 4;
pub const W_OK: c_int = 2;
pub const X_OK: c_int = 1;
pub const F_OK: c_int = 0;

/// Rozlozi priznaky `open`; `None` = neplatna kombinace.
pub fn open_flags(flags: c_int) -> Option<OpenFlags> {
    let acc = flags & O_ACCMODE;
    if acc == 3 {
        return None;
    }
    Some(OpenFlags {
        read: acc == O_RDONLY || acc == O_RDWR,
        write: acc == O_WRONLY || acc == O_RDWR,
        append: flags & O_APPEND != 0,
        create: flags & O_CREAT != 0,
        excl: flags & O_EXCL != 0,
        trunc: flags & O_TRUNC != 0,
        directory: flags & O_DIRECTORY != 0,
        cloexec: flags & O_CLOEXEC != 0,
        nonblock: flags & O_NONBLOCK != 0,
    })
}

fn ssize(r: sys::R<usize>) -> isize {
    errno::ret_val(r)
}

fn ret_fd(r: sys::R<i32>) -> c_int {
    errno::ret_val(r) as c_int
}

/// `open(path, flags, mode)`; `mode` (prava) se nepouzije - prava jsou
/// granty FS, novy soubor dedi popis opravneni od adresare.
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn open(path: *const c_char, flags: c_int, _mode: ...) -> c_int {
    if path.is_null() {
        errno::set(errno::EFAULT);
        return -1;
    }
    let Some(o) = open_flags(flags) else {
        errno::set(errno::EINVAL);
        return -1;
    };
    // SAFETY: retezec C.
    let p = unsafe { c_bytes(path) };
    if p.is_empty() {
        errno::set(errno::ENOENT);
        return -1;
    }
    ret_fd(sys::open(p, &o))
}

// `openat` a ostatni funkce `*at` jsou v `fsat.rs`.

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn creat(path: *const c_char, mode: u32) -> c_int {
    // SAFETY: kontrakt C.
    unsafe { open(path, O_WRONLY | O_CREAT | O_TRUNC, mode) }
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub extern "C" fn close(fd: c_int) -> c_int {
    errno::ret(sys::close(fd))
}

/// # Safety
///
/// `buf` ma `n` bajtu (nebo `n` = 0).
unsafe fn slice_mut<'a>(buf: *mut u8, n: size_t) -> Option<&'a mut [u8]> {
    if n == 0 {
        return Some(&mut []);
    }
    // SAFETY: predano volajicim.
    (!buf.is_null()).then(|| unsafe { core::slice::from_raw_parts_mut(buf, n) })
}

/// # Safety
///
/// `buf` ma `n` bajtu (nebo `n` = 0).
unsafe fn slice<'a>(buf: *const u8, n: size_t) -> Option<&'a [u8]> {
    if n == 0 {
        return Some(&[]);
    }
    // SAFETY: predano volajicim.
    (!buf.is_null()).then(|| unsafe { core::slice::from_raw_parts(buf, n) })
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn read(fd: c_int, buf: *mut u8, n: size_t) -> isize {
    // SAFETY: kontrakt C.
    match unsafe { slice_mut(buf, n) } {
        Some(b) => ssize(sys::read(fd, b)),
        None => ssize(Err(errno::EFAULT)),
    }
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn write(fd: c_int, buf: *const u8, n: size_t) -> isize {
    // SAFETY: kontrakt C.
    match unsafe { slice(buf, n) } {
        Some(b) => ssize(sys::write(fd, b)),
        None => ssize(Err(errno::EFAULT)),
    }
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn pread(fd: c_int, buf: *mut u8, n: size_t, off: i64) -> isize {
    let Ok(off) = u64::try_from(off) else { return ssize(Err(errno::EINVAL)) };
    // SAFETY: kontrakt C.
    match unsafe { slice_mut(buf, n) } {
        Some(b) => ssize(sys::pread(fd, b, off)),
        None => ssize(Err(errno::EFAULT)),
    }
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn pwrite(fd: c_int, buf: *const u8, n: size_t, off: i64) -> isize {
    let Ok(off) = u64::try_from(off) else { return ssize(Err(errno::EINVAL)) };
    // SAFETY: kontrakt C.
    match unsafe { slice(buf, n) } {
        Some(b) => ssize(sys::pwrite(fd, b, off)),
        None => ssize(Err(errno::EFAULT)),
    }
}

/// `struct iovec`.
#[repr(C)]
pub struct iovec {
    pub iov_base: *mut u8,
    pub iov_len: size_t,
}

/// # Safety
///
/// `iov` ma `cnt` polozek (nebo `cnt` = 0).
unsafe fn iovecs<'a>(iov: *const iovec, cnt: usize) -> Option<&'a [iovec]> {
    if cnt == 0 {
        return Some(&[]);
    }
    // SAFETY: predano volajicim.
    (!iov.is_null()).then(|| unsafe { core::slice::from_raw_parts(iov, cnt) })
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn readv(fd: c_int, iov: *const iovec, cnt: c_int) -> isize {
    let Ok(cnt) = usize::try_from(cnt) else { return ssize(Err(errno::EINVAL)) };
    // SAFETY: `iov` ma `cnt` polozek (kontrakt C).
    let Some(v) = (unsafe { iovecs(iov, cnt) }) else { return ssize(Err(errno::EFAULT)) };
    let mut total = 0usize;
    for x in v {
        // SAFETY: kontrakt C.
        let Some(b) = (unsafe { slice_mut(x.iov_base, x.iov_len) }) else { return ssize(Err(errno::EFAULT)) };
        match sys::read(fd, b) {
            Ok(n) => {
                total += n;
                if n < b.len() {
                    break;
                }
            }
            Err(e) if total == 0 => return ssize(Err(e)),
            Err(_) => break,
        }
    }
    total as isize
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn writev(fd: c_int, iov: *const iovec, cnt: c_int) -> isize {
    let Ok(cnt) = usize::try_from(cnt) else { return ssize(Err(errno::EINVAL)) };
    // SAFETY: `iov` ma `cnt` polozek (kontrakt C).
    let Some(v) = (unsafe { iovecs(iov, cnt) }) else { return ssize(Err(errno::EFAULT)) };
    // Jeden zapis (roura a stdout nesmi michat kusy s jinym vlaknem).
    let mut all = Vec::new();
    for x in v {
        // SAFETY: kontrakt C.
        let Some(b) = (unsafe { slice(x.iov_base, x.iov_len) }) else { return ssize(Err(errno::EFAULT)) };
        all.extend_from_slice(b);
    }
    ssize(sys::write(fd, &all))
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub extern "C" fn lseek(fd: c_int, off: i64, whence: c_int) -> i64 {
    let pos = match whence {
        0 if off >= 0 => Seek::Set(off as u64),
        1 => Seek::Cur(off),
        2 => Seek::End(off),
        _ => return errno::ret_val::<u64>(Err(errno::EINVAL)) as i64,
    };
    errno::ret_val(sys::seek(fd, pos)) as i64
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub extern "C" fn dup(fd: c_int) -> c_int {
    ret_fd(sys::dup(fd, 0, false))
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub extern "C" fn dup2(old: c_int, new: c_int) -> c_int {
    if old == new {
        // POSIX: jen overit platnost.
        return ret_fd(sys::status(old).map(|_| old));
    }
    ret_fd(sys::dup2(old, new, false))
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub extern "C" fn dup3(old: c_int, new: c_int, flags: c_int) -> c_int {
    if old == new || flags & !O_CLOEXEC != 0 {
        return ret_fd(Err(errno::EINVAL));
    }
    ret_fd(sys::dup2(old, new, flags & O_CLOEXEC != 0))
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn pipe2(fds: *mut c_int, flags: c_int) -> c_int {
    if fds.is_null() {
        return errno::ret(Err(errno::EFAULT));
    }
    if flags & !(O_CLOEXEC | O_NONBLOCK) != 0 {
        return errno::ret(Err(errno::EINVAL));
    }
    match sys::pipe(flags & O_CLOEXEC != 0, flags & O_NONBLOCK != 0) {
        Ok((r, w)) => {
            // SAFETY: `fds` ma dve polozky.
            unsafe {
                *fds = r;
                *fds.add(1) = w;
            }
            0
        }
        Err(e) => errno::ret(Err(e)),
    }
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn pipe(fds: *mut c_int) -> c_int {
    // SAFETY: kontrakt C.
    unsafe { pipe2(fds, 0) }
}

/// `fcntl`: duplikace, `FD_CLOEXEC`, stav (`O_APPEND`, `O_NONBLOCK`)
/// a zamky (`flock.rs`).
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn fcntl(fd: c_int, cmd: c_int, mut args: ...) -> c_int {
    match cmd {
        F_DUPFD | F_DUPFD_CLOEXEC => {
            // SAFETY: argument int (kontrakt C).
            let min = unsafe { args.next_arg::<c_int>() };
            ret_fd(sys::dup(fd, min, cmd == F_DUPFD_CLOEXEC))
        }
        F_GETFD => ret_fd(sys::cloexec(fd).map(|c| if c { FD_CLOEXEC } else { 0 })),
        F_SETFD => {
            // SAFETY: argument int.
            let v = unsafe { args.next_arg::<c_int>() };
            errno::ret(sys::set_cloexec(fd, v & FD_CLOEXEC != 0))
        }
        F_GETFL => ret_fd(sys::status(fd).map(|s| s as c_int)),
        F_SETFL => {
            // SAFETY: argument int.
            let v = unsafe { args.next_arg::<c_int>() };
            errno::ret(sys::set_status(fd, v as u32))
        }
        F_GETLK | F_SETLK | F_SETLKW => {
            // SAFETY: argument struct flock *.
            let p = unsafe { args.next_arg::<*mut crate::flock::flock>() };
            // SAFETY: kontrakt C.
            unsafe { crate::flock::fcntl_lock(fd, cmd, p) }
        }
        _ => errno::ret(Err(errno::EINVAL)),
    }
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub extern "C" fn isatty(fd: c_int) -> c_int {
    match sys::isatty(fd) {
        Ok(true) => 1,
        Ok(false) => {
            errno::set(errno::ENOTTY);
            0
        }
        Err(e) => {
            errno::set(e);
            0
        }
    }
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub extern "C" fn fsync(fd: c_int) -> c_int {
    errno::ret(sys::fsync(fd))
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub extern "C" fn fdatasync(fd: c_int) -> c_int {
    errno::ret(sys::fsync(fd))
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub extern "C" fn ftruncate(fd: c_int, len: i64) -> c_int {
    match u64::try_from(len) {
        Ok(l) => errno::ret(sys::ftruncate(fd, l)),
        Err(_) => errno::ret(Err(errno::EINVAL)),
    }
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn truncate(path: *const c_char, len: i64) -> c_int {
    // SAFETY: kontrakt C.
    let fd = unsafe { open(path, O_WRONLY, 0) };
    if fd < 0 {
        return -1;
    }
    let r = ftruncate(fd, len);
    let _ = sys::close(fd);
    r
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn unlink(path: *const c_char) -> c_int {
    // SAFETY: retezec C.
    errno::ret(sys::unlink(unsafe { c_bytes(path) }))
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn rmdir(path: *const c_char) -> c_int {
    // SAFETY: retezec C.
    errno::ret(sys::rmdir(unsafe { c_bytes(path) }))
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn getcwd(buf: *mut c_char, size: size_t) -> *mut c_char {
    let cwd = sys::getcwd();
    if buf.is_null() {
        // Rozsireni glibc/musl: alokovat (velikost 0 = podle potreby).
        let n = if size == 0 { cwd.len() + 1 } else { size };
        if n < cwd.len() + 1 {
            errno::set(errno::ERANGE);
            return core::ptr::null_mut();
        }
        // SAFETY: malloc z cstd-core.
        let p = unsafe { cstd_core::stdlib::malloc(n) };
        if p.is_null() {
            errno::set(errno::ENOMEM);
            return p;
        }
        // SAFETY: `p` ma `n` >= delka + 1 bajtu.
        unsafe {
            core::ptr::copy_nonoverlapping(cwd.as_ptr(), p, cwd.len());
            *p.add(cwd.len()) = 0;
        }
        return p;
    }
    if size < cwd.len() + 1 {
        errno::set(if size == 0 { errno::EINVAL } else { errno::ERANGE });
        return core::ptr::null_mut();
    }
    // SAFETY: `buf` ma `size` bajtu.
    unsafe {
        core::ptr::copy_nonoverlapping(cwd.as_ptr(), buf, cwd.len());
        *buf.add(cwd.len()) = 0;
    }
    buf
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn chdir(path: *const c_char) -> c_int {
    // SAFETY: retezec C.
    errno::ret(sys::chdir(unsafe { c_bytes(path) }))
}

/// `fchdir`: adresar otevreny jako `fd` (cesta z doby otevreni; po
/// prejmenovani adresare zastara).
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub extern "C" fn fchdir(fd: c_int) -> c_int {
    errno::ret(sys::fchdir(fd))
}

/// `access`: existence a ucinna prava relace (`X_OK` = soubor existuje;
/// spousteni urcuje manifest, ne bit).
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn access(path: *const c_char, mode: c_int) -> c_int {
    // SAFETY: retezec C.
    let st = match sys::stat(unsafe { c_bytes(path) }) {
        Ok(s) => s,
        Err(e) => return errno::ret(Err(e)),
    };
    if (mode & R_OK != 0 && !st.readable) || (mode & W_OK != 0 && !st.writable) {
        return errno::ret(Err(errno::EACCES));
    }
    0
}

/// Procesy vznikaji jen pres `posix_spawn` (`docs/cs/cstd.md`, Bez `fork`).
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub extern "C" fn fork() -> c_int {
    errno::ret(Err(errno::ENOSYS))
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub extern "C" fn vfork() -> c_int {
    errno::ret(Err(errno::ENOSYS))
}

/// Nahrazeni procesu (`exec*`) neexistuje; vsechny varianty vraci `ENOSYS`.
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub extern "C" fn execv(_path: *const c_char, _argv: *const *const c_char) -> c_int {
    errno::ret(Err(errno::ENOSYS))
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub extern "C" fn execvp(_file: *const c_char, _argv: *const *const c_char) -> c_int {
    errno::ret(Err(errno::ENOSYS))
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub extern "C" fn execve(_path: *const c_char, _argv: *const *const c_char, _envp: *const *const c_char) -> c_int {
    errno::ret(Err(errno::ENOSYS))
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn execl(_path: *const c_char, _arg: *const c_char, _args: ...) -> c_int {
    errno::ret(Err(errno::ENOSYS))
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn execlp(_file: *const c_char, _arg: *const c_char, _args: ...) -> c_int {
    errno::ret(Err(errno::ENOSYS))
}

pub const SC_PAGESIZE: c_int = 30;
pub const SC_OPEN_MAX: c_int = 4;
pub const SC_NPROCESSORS_CONF: c_int = 83;
pub const SC_NPROCESSORS_ONLN: c_int = 84;
pub const SC_CLK_TCK: c_int = 2;

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub extern "C" fn sysconf(name: c_int) -> i64 {
    match name {
        SC_PAGESIZE => sys::page_size() as i64,
        SC_OPEN_MAX => 4096,
        SC_CLK_TCK => crate::time::CLK_TCK as i64,
        SC_NPROCESSORS_CONF | SC_NPROCESSORS_ONLN => {
            std::thread::available_parallelism().map_or(1, |n| n.get() as i64)
        }
        _ => {
            errno::set(errno::EINVAL);
            -1
        }
    }
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub extern "C" fn getpagesize() -> c_int {
    sys::page_size() as c_int
}

// --- relace a skupiny procesu ----------------------------------------------------
//
// Skupiny procesu ani relace terminalu nejsou: kazdy proces je vudce vlastni
// relace i skupiny (cislo = `getpid()`); potomek z `posix_spawn` je v novem
// jobu (docs/cs/cstd.md, kapitola Doplnek POSIX).

/// `setsid`: proces uz je vudce sve relace, vraci jeji cislo.
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub extern "C" fn setsid() -> c_int {
    crate::signal::getpid()
}

/// `getsid(0)` / `getsid(getpid())` = vlastni relace; jiny proces `ESRCH`.
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub extern "C" fn getsid(pid: c_int) -> c_int {
    own_group(pid)
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub extern "C" fn getpgid(pid: c_int) -> c_int {
    own_group(pid)
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub extern "C" fn getpgrp() -> c_int {
    crate::signal::getpid()
}

fn own_group(pid: c_int) -> c_int {
    let me = crate::signal::getpid();
    if pid == 0 || pid == me {
        me
    } else {
        errno::ret(Err(errno::ESRCH))
    }
}

/// `setpgid`: jen "zustat ve vlastni skupine" (`(0, 0)` nebo vlastni cislo);
/// presun do jine skupiny = `EPERM`.
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub extern "C" fn setpgid(pid: c_int, pgid: c_int) -> c_int {
    let me = crate::signal::getpid();
    if (pid == 0 || pid == me) && (pgid == 0 || pgid == me) {
        0
    } else {
        errno::ret(Err(errno::EPERM))
    }
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub extern "C" fn setpgrp() -> c_int {
    0
}
