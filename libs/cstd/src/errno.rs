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
pub const ESRCH: c_int = 3;
pub const ENXIO: c_int = 6;
pub const E2BIG: c_int = 7;
pub const ENOEXEC: c_int = 8;
pub const EDEADLK: c_int = 35;
pub const ENAMETOOLONG: c_int = 36;
pub const ENOLCK: c_int = 37;
pub const ELOOP: c_int = 40;
pub const ENOTTY: c_int = 25;
pub const EMLINK: c_int = 31;
pub const EOVERFLOW: c_int = 75;
pub const EILSEQ: c_int = 84;
pub const ENOTSOCK: c_int = 88;
pub const EDESTADDRREQ: c_int = 89;
pub const EMSGSIZE: c_int = 90;
pub const EPROTOTYPE: c_int = 91;
pub const ENOPROTOOPT: c_int = 92;
pub const EPROTONOSUPPORT: c_int = 93;
pub const EAFNOSUPPORT: c_int = 97;
pub const ENETDOWN: c_int = 100;
pub const ECONNABORTED: c_int = 103;
pub const ENOBUFS: c_int = 105;
pub const EISCONN: c_int = 106;
pub const ENFILE: c_int = 23;
pub const ENODEV: c_int = 19;

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

/// `errno` pro kod chyby systemu (`abi::Error` jako `i32`, viz
/// `docs/cs/chyby.md`).
pub fn from_os(code: i32) -> c_int {
    match code {
        -1 => EBADF,
        -2 => EACCES,
        -4 => ETIMEDOUT,
        -5 => ENOMEM,
        -6 => EAGAIN,
        -3 | -7 | -15 | -16 => EINVAL,
        -8 | -35 => ENOENT,
        -9 => ENOTSUP,
        -11 => EFAULT,
        -12 => EAGAIN,
        -13 => EPIPE,
        -14 => ERANGE,
        -17 => ENOSYS,
        -18 => ECANCELED,
        -19 => EEXIST,
        -20 => ENOTDIR,
        -21 => EISDIR,
        -22 => ENOTEMPTY,
        -23 => ENOSPC,
        -25 => EROFS,
        -26 | -27 => EBUSY,
        -29 => ECONNREFUSED,
        -30 => ECONNRESET,
        -31 => EHOSTUNREACH,
        -32 => EADDRINUSE,
        -36 => EADDRNOTAVAIL,
        -39 => EINTR,
        _ => EIO,
    }
}

/// Navrat funkce POSIX bez hodnoty: 0, nebo -1 s `errno`.
pub fn ret(r: Result<(), c_int>) -> c_int {
    match r {
        Ok(()) => 0,
        Err(e) => {
            set(e);
            -1
        }
    }
}

/// Navrat s hodnotou (`ssize_t`, cislo fd ...): hodnota, nebo -1 s `errno`.
pub fn ret_val<T: TryInto<isize>>(r: Result<T, c_int>) -> isize {
    match r {
        Ok(v) => v.try_into().unwrap_or(isize::MAX),
        Err(e) => {
            set(e);
            -1
        }
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
        ESRCH => b"No such process\0",
        ENXIO => b"No such device or address\0",
        E2BIG => b"Argument list too long\0",
        ENOEXEC => b"Exec format error\0",
        EDEADLK => b"Resource deadlock avoided\0",
        ENAMETOOLONG => b"File name too long\0",
        ENOLCK => b"No locks available\0",
        ELOOP => b"Too many levels of symbolic links\0",
        ENOTTY => b"Inappropriate ioctl for device\0",
        EMLINK => b"Too many links\0",
        EOVERFLOW => b"Value too large for defined data type\0",
        EILSEQ => b"Invalid or incomplete multibyte or wide character\0",
        ENOTSOCK => b"Socket operation on non-socket\0",
        EMSGSIZE => b"Message too long\0",
        ENOPROTOOPT => b"Protocol not available\0",
        EPROTONOSUPPORT => b"Protocol not supported\0",
        EAFNOSUPPORT => b"Address family not supported by protocol\0",
        ECONNABORTED => b"Software caused connection abort\0",
        ENOBUFS => b"No buffer space available\0",
        EISCONN => b"Transport endpoint is already connected\0",
        ENODEV => b"No such device\0",
        ENFILE => b"Too many open files in system\0",
        EMFILE => b"Too many open files\0",
        EFAULT => b"Bad address\0",
        EBUSY => b"Device or resource busy\0",
        ECHILD => b"No child processes\0",
        EXDEV => b"Invalid cross-device link\0",
        EFBIG => b"File too large\0",
        EDOM => b"Numerical argument out of domain\0",
        EADDRINUSE => b"Address already in use\0",
        EADDRNOTAVAIL => b"Cannot assign requested address\0",
        ENETUNREACH => b"Network is unreachable\0",
        ENOTCONN => b"Transport endpoint is not connected\0",
        EHOSTUNREACH => b"No route to host\0",
        EALREADY => b"Operation already in progress\0",
        EINPROGRESS => b"Operation now in progress\0",
        ECANCELED => b"Operation canceled\0",
        _ => b"Unknown error\0",
    }
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub extern "C" fn strerror(e: c_int) -> *const u8 {
    message(e).as_ptr()
}
