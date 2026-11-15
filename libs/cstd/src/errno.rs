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
pub const ESRCH: c_int = 3;
pub const EINTR: c_int = 4;
pub const EIO: c_int = 5;
pub const ENXIO: c_int = 6;
pub const E2BIG: c_int = 7;
pub const ENOEXEC: c_int = 8;
pub const EBADF: c_int = 9;
pub const ECHILD: c_int = 10;
pub const EAGAIN: c_int = 11;
pub const ENOMEM: c_int = 12;
pub const EACCES: c_int = 13;
pub const EFAULT: c_int = 14;
pub const ENOTBLK: c_int = 15;
pub const EBUSY: c_int = 16;
pub const EEXIST: c_int = 17;
pub const EXDEV: c_int = 18;
pub const ENODEV: c_int = 19;
pub const ENOTDIR: c_int = 20;
pub const EISDIR: c_int = 21;
pub const EINVAL: c_int = 22;
pub const ENFILE: c_int = 23;
pub const EMFILE: c_int = 24;
pub const ENOTTY: c_int = 25;
pub const ETXTBSY: c_int = 26;
pub const EFBIG: c_int = 27;
pub const ENOSPC: c_int = 28;
pub const ESPIPE: c_int = 29;
pub const EROFS: c_int = 30;
pub const EMLINK: c_int = 31;
pub const EPIPE: c_int = 32;
pub const EDOM: c_int = 33;
pub const ERANGE: c_int = 34;
pub const EDEADLK: c_int = 35;
pub const ENAMETOOLONG: c_int = 36;
pub const ENOLCK: c_int = 37;
pub const ENOSYS: c_int = 38;
pub const ENOTEMPTY: c_int = 39;
pub const ELOOP: c_int = 40;
pub const ENOMSG: c_int = 42;
pub const EIDRM: c_int = 43;
pub const ECHRNG: c_int = 44;
pub const EL2NSYNC: c_int = 45;
pub const EL3HLT: c_int = 46;
pub const EL3RST: c_int = 47;
pub const ELNRNG: c_int = 48;
pub const EUNATCH: c_int = 49;
pub const ENOCSI: c_int = 50;
pub const EL2HLT: c_int = 51;
pub const EBADE: c_int = 52;
pub const EBADR: c_int = 53;
pub const EXFULL: c_int = 54;
pub const ENOANO: c_int = 55;
pub const EBADRQC: c_int = 56;
pub const EBADSLT: c_int = 57;
pub const EBFONT: c_int = 59;
pub const ENOSTR: c_int = 60;
pub const ENODATA: c_int = 61;
pub const ETIME: c_int = 62;
pub const ENOSR: c_int = 63;
pub const ENONET: c_int = 64;
pub const ENOPKG: c_int = 65;
pub const EREMOTE: c_int = 66;
pub const ENOLINK: c_int = 67;
pub const EADV: c_int = 68;
pub const ESRMNT: c_int = 69;
pub const ECOMM: c_int = 70;
pub const EPROTO: c_int = 71;
pub const EMULTIHOP: c_int = 72;
pub const EDOTDOT: c_int = 73;
pub const EBADMSG: c_int = 74;
pub const EOVERFLOW: c_int = 75;
pub const ENOTUNIQ: c_int = 76;
pub const EBADFD: c_int = 77;
pub const EREMCHG: c_int = 78;
pub const ELIBACC: c_int = 79;
pub const ELIBBAD: c_int = 80;
pub const ELIBSCN: c_int = 81;
pub const ELIBMAX: c_int = 82;
pub const ELIBEXEC: c_int = 83;
pub const EILSEQ: c_int = 84;
pub const ERESTART: c_int = 85;
pub const ESTRPIPE: c_int = 86;
pub const EUSERS: c_int = 87;
pub const ENOTSOCK: c_int = 88;
pub const EDESTADDRREQ: c_int = 89;
pub const EMSGSIZE: c_int = 90;
pub const EPROTOTYPE: c_int = 91;
pub const ENOPROTOOPT: c_int = 92;
pub const EPROTONOSUPPORT: c_int = 93;
pub const ESOCKTNOSUPPORT: c_int = 94;
pub const ENOTSUP: c_int = 95;
pub const EPFNOSUPPORT: c_int = 96;
pub const EAFNOSUPPORT: c_int = 97;
pub const EADDRINUSE: c_int = 98;
pub const EADDRNOTAVAIL: c_int = 99;
pub const ENETDOWN: c_int = 100;
pub const ENETUNREACH: c_int = 101;
pub const ENETRESET: c_int = 102;
pub const ECONNABORTED: c_int = 103;
pub const ECONNRESET: c_int = 104;
pub const ENOBUFS: c_int = 105;
pub const EISCONN: c_int = 106;
pub const ENOTCONN: c_int = 107;
pub const ESHUTDOWN: c_int = 108;
pub const ETOOMANYREFS: c_int = 109;
pub const ETIMEDOUT: c_int = 110;
pub const ECONNREFUSED: c_int = 111;
pub const EHOSTDOWN: c_int = 112;
pub const EHOSTUNREACH: c_int = 113;
pub const EALREADY: c_int = 114;
pub const EINPROGRESS: c_int = 115;
pub const ESTALE: c_int = 116;
pub const EUCLEAN: c_int = 117;
pub const ENOTNAM: c_int = 118;
pub const ENAVAIL: c_int = 119;
pub const EISNAM: c_int = 120;
pub const EREMOTEIO: c_int = 121;
pub const EDQUOT: c_int = 122;
pub const ENOMEDIUM: c_int = 123;
pub const EMEDIUMTYPE: c_int = 124;
pub const ECANCELED: c_int = 125;
pub const ENOKEY: c_int = 126;
pub const EKEYEXPIRED: c_int = 127;
pub const EKEYREVOKED: c_int = 128;
pub const EKEYREJECTED: c_int = 129;
pub const EOWNERDEAD: c_int = 130;
pub const ENOTRECOVERABLE: c_int = 131;
pub const ERFKILL: c_int = 132;
pub const EHWPOISON: c_int = 133;

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
        -40 => ELOOP,
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
        ESRCH => b"No such process\0",
        EINTR => b"Interrupted system call\0",
        EIO => b"Input/output error\0",
        ENXIO => b"No such device or address\0",
        E2BIG => b"Argument list too long\0",
        ENOEXEC => b"Exec format error\0",
        EBADF => b"Bad file descriptor\0",
        ECHILD => b"No child processes\0",
        EAGAIN => b"Resource temporarily unavailable\0",
        ENOMEM => b"Cannot allocate memory\0",
        EACCES => b"Permission denied\0",
        EFAULT => b"Bad address\0",
        ENOTBLK => b"Block device required\0",
        EBUSY => b"Device or resource busy\0",
        EEXIST => b"File exists\0",
        EXDEV => b"Invalid cross-device link\0",
        ENODEV => b"No such device\0",
        ENOTDIR => b"Not a directory\0",
        EISDIR => b"Is a directory\0",
        EINVAL => b"Invalid argument\0",
        ENFILE => b"Too many open files in system\0",
        EMFILE => b"Too many open files\0",
        ENOTTY => b"Inappropriate ioctl for device\0",
        ETXTBSY => b"Text file busy\0",
        EFBIG => b"File too large\0",
        ENOSPC => b"No space left on device\0",
        ESPIPE => b"Illegal seek\0",
        EROFS => b"Read-only file system\0",
        EMLINK => b"Too many links\0",
        EPIPE => b"Broken pipe\0",
        EDOM => b"Numerical argument out of domain\0",
        ERANGE => b"Numerical result out of range\0",
        EDEADLK => b"Resource deadlock avoided\0",
        ENAMETOOLONG => b"File name too long\0",
        ENOLCK => b"No locks available\0",
        ENOSYS => b"Function not implemented\0",
        ENOTEMPTY => b"Directory not empty\0",
        ELOOP => b"Too many levels of symbolic links\0",
        ENOMSG => b"No message of desired type\0",
        EIDRM => b"Identifier removed\0",
        ECHRNG => b"Channel number out of range\0",
        EL2NSYNC => b"Level 2 not synchronized\0",
        EL3HLT => b"Level 3 halted\0",
        EL3RST => b"Level 3 reset\0",
        ELNRNG => b"Link number out of range\0",
        EUNATCH => b"Protocol driver not attached\0",
        ENOCSI => b"No CSI structure available\0",
        EL2HLT => b"Level 2 halted\0",
        EBADE => b"Invalid exchange\0",
        EBADR => b"Invalid request descriptor\0",
        EXFULL => b"Exchange full\0",
        ENOANO => b"No anode\0",
        EBADRQC => b"Invalid request code\0",
        EBADSLT => b"Invalid slot\0",
        EBFONT => b"Bad font file format\0",
        ENOSTR => b"Device not a stream\0",
        ENODATA => b"No data available\0",
        ETIME => b"Timer expired\0",
        ENOSR => b"Out of streams resources\0",
        ENONET => b"Machine is not on the network\0",
        ENOPKG => b"Package not installed\0",
        EREMOTE => b"Object is remote\0",
        ENOLINK => b"Link has been severed\0",
        EADV => b"Advertise error\0",
        ESRMNT => b"Srmount error\0",
        ECOMM => b"Communication error on send\0",
        EPROTO => b"Protocol error\0",
        EMULTIHOP => b"Multihop attempted\0",
        EDOTDOT => b"RFS specific error\0",
        EBADMSG => b"Bad message\0",
        EOVERFLOW => b"Value too large for defined data type\0",
        ENOTUNIQ => b"Name not unique on network\0",
        EBADFD => b"File descriptor in bad state\0",
        EREMCHG => b"Remote address changed\0",
        ELIBACC => b"Can not access a needed shared library\0",
        ELIBBAD => b"Accessing a corrupted shared library\0",
        ELIBSCN => b".lib section in a.out corrupted\0",
        ELIBMAX => b"Attempting to link in too many shared libraries\0",
        ELIBEXEC => b"Cannot exec a shared library directly\0",
        EILSEQ => b"Invalid or incomplete multibyte or wide character\0",
        ERESTART => b"Interrupted system call should be restarted\0",
        ESTRPIPE => b"Streams pipe error\0",
        EUSERS => b"Too many users\0",
        ENOTSOCK => b"Socket operation on non-socket\0",
        EDESTADDRREQ => b"Destination address required\0",
        EMSGSIZE => b"Message too long\0",
        EPROTOTYPE => b"Protocol wrong type for socket\0",
        ENOPROTOOPT => b"Protocol not available\0",
        EPROTONOSUPPORT => b"Protocol not supported\0",
        ESOCKTNOSUPPORT => b"Socket type not supported\0",
        ENOTSUP => b"Operation not supported\0",
        EPFNOSUPPORT => b"Protocol family not supported\0",
        EAFNOSUPPORT => b"Address family not supported by protocol\0",
        EADDRINUSE => b"Address already in use\0",
        EADDRNOTAVAIL => b"Cannot assign requested address\0",
        ENETDOWN => b"Network is down\0",
        ENETUNREACH => b"Network is unreachable\0",
        ENETRESET => b"Network dropped connection on reset\0",
        ECONNABORTED => b"Software caused connection abort\0",
        ECONNRESET => b"Connection reset by peer\0",
        ENOBUFS => b"No buffer space available\0",
        EISCONN => b"Transport endpoint is already connected\0",
        ENOTCONN => b"Transport endpoint is not connected\0",
        ESHUTDOWN => b"Cannot send after transport endpoint shutdown\0",
        ETOOMANYREFS => b"Too many references: cannot splice\0",
        ETIMEDOUT => b"Connection timed out\0",
        ECONNREFUSED => b"Connection refused\0",
        EHOSTDOWN => b"Host is down\0",
        EHOSTUNREACH => b"No route to host\0",
        EALREADY => b"Operation already in progress\0",
        EINPROGRESS => b"Operation now in progress\0",
        ESTALE => b"Stale file handle\0",
        EUCLEAN => b"Structure needs cleaning\0",
        ENOTNAM => b"Not a XENIX named type file\0",
        ENAVAIL => b"No XENIX semaphores available\0",
        EISNAM => b"Is a named type file\0",
        EREMOTEIO => b"Remote I/O error\0",
        EDQUOT => b"Disk quota exceeded\0",
        ENOMEDIUM => b"No medium found\0",
        EMEDIUMTYPE => b"Wrong medium type\0",
        ECANCELED => b"Operation canceled\0",
        ENOKEY => b"Required key not available\0",
        EKEYEXPIRED => b"Key has expired\0",
        EKEYREVOKED => b"Key has been revoked\0",
        EKEYREJECTED => b"Key was rejected by service\0",
        EOWNERDEAD => b"Owner died\0",
        ENOTRECOVERABLE => b"State not recoverable\0",
        ERFKILL => b"Operation not possible due to RF-kill\0",
        EHWPOISON => b"Memory page has hardware error\0",
        _ => b"Unknown error\0",
    }
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub extern "C" fn strerror(e: c_int) -> *const u8 {
    message(e).as_ptr()
}
