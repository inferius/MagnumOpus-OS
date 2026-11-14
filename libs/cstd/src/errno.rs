//! `errno.h`: cislo chyby posledniho volani (per vlakno) a jeho text.
//!
//! Cisla jsou vlastni cisla POSIX (stejne hodnoty jako bezne systemy, kod
//! v C je porovnava se jmeny). Chyby systemu (`abi::Error` v `io::Error`)
//! se prevadi podle druhu chyby.

use std::cell::Cell;
use std::io;

use crate::c_int;

pub const EPERM: c_int = 1;
pub const ENOENT: c_int = 2;
pub const EINTR: c_int = 4;
pub const EIO: c_int = 5;
pub const EBADF: c_int = 9;
pub const ECHILD: c_int = 10;
pub const EAGAIN: c_int = 11;
pub const ENOMEM: c_int = 12;
pub const EACCES: c_int = 13;
pub const EFAULT: c_int = 14;
pub const EBUSY: c_int = 16;
pub const EEXIST: c_int = 17;
pub const EXDEV: c_int = 18;
pub const ENOTDIR: c_int = 20;
pub const EISDIR: c_int = 21;
pub const EINVAL: c_int = 22;
pub const EMFILE: c_int = 24;
pub const EFBIG: c_int = 27;
pub const ENOSPC: c_int = 28;
pub const ESPIPE: c_int = 29;
pub const EROFS: c_int = 30;
pub const EPIPE: c_int = 32;
pub const EDOM: c_int = 33;
pub const ERANGE: c_int = 34;
pub const ENOSYS: c_int = 38;
pub const ENOTEMPTY: c_int = 39;
pub const ENOTSUP: c_int = 95;
pub const EADDRINUSE: c_int = 98;
pub const EADDRNOTAVAIL: c_int = 99;
pub const ENETUNREACH: c_int = 101;
pub const ECONNRESET: c_int = 104;
pub const ENOTCONN: c_int = 107;
pub const ETIMEDOUT: c_int = 110;
pub const ECONNREFUSED: c_int = 111;
pub const EHOSTUNREACH: c_int = 113;
pub const EALREADY: c_int = 114;
pub const EINPROGRESS: c_int = 115;
pub const ECANCELED: c_int = 125;

thread_local! {
    static ERRNO: Cell<c_int> = const { Cell::new(0) };
}

/// Adresa `errno` aktualniho vlakna (`#define errno (*__errno_location())`).
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub extern "C" fn __errno_location() -> *mut c_int {
    ERRNO.with(Cell::as_ptr)
}

pub fn set(e: c_int) {
    ERRNO.with(|c| c.set(e));
}

pub fn get() -> c_int {
    ERRNO.with(Cell::get)
}

/// `errno` pro chybu std.
pub fn from_io(e: &io::Error) -> c_int {
    use io::ErrorKind as K;
    match e.kind() {
        K::NotFound => ENOENT,
        K::PermissionDenied => EACCES,
        K::ConnectionRefused => ECONNREFUSED,
        K::ConnectionReset => ECONNRESET,
        K::HostUnreachable => EHOSTUNREACH,
        K::NetworkUnreachable => ENETUNREACH,
        K::NotConnected => ENOTCONN,
        K::AddrInUse => EADDRINUSE,
        K::AddrNotAvailable => EADDRNOTAVAIL,
        K::BrokenPipe => EPIPE,
        K::AlreadyExists => EEXIST,
        K::WouldBlock => EAGAIN,
        K::NotADirectory => ENOTDIR,
        K::IsADirectory => EISDIR,
        K::DirectoryNotEmpty => ENOTEMPTY,
        K::ReadOnlyFilesystem => EROFS,
        K::StorageFull => ENOSPC,
        K::NotSeekable => ESPIPE,
        K::QuotaExceeded => ENOSPC,
        K::FileTooLarge => EFBIG,
        K::CrossesDevices => EXDEV,
        K::InvalidInput | K::InvalidData | K::InvalidFilename => EINVAL,
        K::TimedOut => ETIMEDOUT,
        K::Interrupted => EINTR,
        K::Unsupported => ENOTSUP,
        K::OutOfMemory => ENOMEM,
        K::ResourceBusy => EBUSY,
        _ => EIO,
    }
}

/// Nastavi `errno` z chyby std a vrati -1 (bezny navrat funkci POSIX).
pub fn fail(e: &io::Error) -> c_int {
    set(from_io(e));
    -1
}

/// Text chyby (`strerror`); nezname cislo = obecny text.
pub fn message(e: c_int) -> &'static [u8] {
    match e {
        0 => b"Success\0",
        EPERM => b"Operation not permitted\0",
        ENOENT => b"No such file or directory\0",
        EINTR => b"Interrupted system call\0",
        EIO => b"Input/output error\0",
        EBADF => b"Bad file descriptor\0",
        EAGAIN => b"Resource temporarily unavailable\0",
        ENOMEM => b"Cannot allocate memory\0",
        EACCES => b"Permission denied\0",
        EEXIST => b"File exists\0",
        ENOTDIR => b"Not a directory\0",
        EISDIR => b"Is a directory\0",
        EINVAL => b"Invalid argument\0",
        ENOSPC => b"No space left on device\0",
        ESPIPE => b"Illegal seek\0",
        EROFS => b"Read-only file system\0",
        EPIPE => b"Broken pipe\0",
        ERANGE => b"Numerical result out of range\0",
        ENOSYS => b"Function not implemented\0",
        ENOTEMPTY => b"Directory not empty\0",
        ENOTSUP => b"Operation not supported\0",
        ETIMEDOUT => b"Connection timed out\0",
        ECONNREFUSED => b"Connection refused\0",
        ECONNRESET => b"Connection reset by peer\0",
        _ => b"Unknown error\0",
    }
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub extern "C" fn strerror(e: c_int) -> *const u8 {
    message(e).as_ptr()
}
