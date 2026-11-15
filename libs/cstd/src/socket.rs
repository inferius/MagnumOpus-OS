//! Sockety BSD (`sys/socket.h`, `netinet/in.h`, `arpa/inet.h`,
//! `netdb.h`) nad relaci site (osrt::sock, netproto v2).
//!
//! `AF_INET` a `AF_INET6`, `SOCK_STREAM` (TCP) a `SOCK_DGRAM` (UDP).
//! `AF_UNIX` neni (mistni IPC jsou kanaly). Co program smi (cile, porty,
//! naslouchani), urcuje relace site z manifestu, ne `setsockopt`.

use core::ffi::c_void;
use core::time::Duration;
use std::ffi::CString;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV4, SocketAddrV6};

use crate::sys::{self, SockKind};
use crate::unistd::iovec;
use crate::{c_bytes, c_char, c_int, errno, size_t};

pub const AF_UNSPEC: c_int = 0;
pub const AF_UNIX: c_int = 1;
pub const AF_INET: c_int = 2;
pub const AF_INET6: c_int = 10;
pub const AF_NETLINK: c_int = 16;
pub const AF_PACKET: c_int = 17;
pub const SOCK_STREAM: c_int = 1;
pub const SOCK_DGRAM: c_int = 2;
pub const SOCK_RAW: c_int = 3;
pub const SOCK_SEQPACKET: c_int = 5;
pub const SCM_RIGHTS: c_int = 1;
pub const SO_LINGER: c_int = 13;
pub const SO_PROTOCOL: c_int = 38;
pub const SO_DOMAIN: c_int = 39;
pub const IPPROTO_IP: c_int = 0;
pub const IPPROTO_IPV6: c_int = 41;
pub const IP_TTL: c_int = 2;
/// Vychozi TTL / pocet skoku netd (jako Linux).
pub const DEFAULT_TTL: c_int = 64;
pub const SOCK_NONBLOCK: c_int = 0o4000;
pub const SOCK_CLOEXEC: c_int = 0o2000000;
pub const IPPROTO_TCP: c_int = 6;
pub const IPPROTO_UDP: c_int = 17;
pub const SOL_SOCKET: c_int = 1;
pub const SO_REUSEADDR: c_int = 2;
pub const SO_TYPE: c_int = 3;
pub const SO_ERROR: c_int = 4;
pub const SO_SNDBUF: c_int = 7;
pub const SO_RCVBUF: c_int = 8;
pub const SO_KEEPALIVE: c_int = 9;
pub const SO_RCVTIMEO: c_int = 20;
pub const SO_SNDTIMEO: c_int = 21;
pub const SO_ACCEPTCONN: c_int = 30;
pub const TCP_NODELAY: c_int = 1;
pub const MSG_PEEK: c_int = 2;
pub const MSG_DONTWAIT: c_int = 0x40;
pub const MSG_NOSIGNAL: c_int = 0x4000;
pub const SHUT_RD: c_int = 0;
pub const SHUT_WR: c_int = 1;
pub const SHUT_RDWR: c_int = 2;

/// `struct sockaddr_in` (16 B).
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct sockaddr_in {
    pub sin_family: u16,
    pub sin_port: u16,
    pub sin_addr: u32,
    pub sin_zero: [u8; 8],
}

/// `struct sockaddr_in6` (28 B).
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct sockaddr_in6 {
    pub sin6_family: u16,
    pub sin6_port: u16,
    pub sin6_flowinfo: u32,
    pub sin6_addr: [u8; 16],
    pub sin6_scope_id: u32,
}

/// `struct sockaddr_storage` (128 B).
#[repr(C, align(8))]
#[derive(Clone, Copy)]
pub struct sockaddr_storage {
    pub ss_family: u16,
    _data: [u8; 126],
}

/// Adresa z `struct sockaddr *` C.
///
/// # Safety
///
/// `p` ukazuje na `len` bajtu.
pub unsafe fn read_addr(p: *const c_void, len: u32) -> Result<SocketAddr, c_int> {
    if p.is_null() || len < 2 {
        return Err(errno::EINVAL);
    }
    // SAFETY: aspon 2 bajty (rodina).
    let family = c_int::from(unsafe { p.cast::<u16>().read_unaligned() });
    match family {
        AF_INET if len as usize >= size_of::<sockaddr_in>() => {
            // SAFETY: dost bajtu pro sockaddr_in.
            let a = unsafe { p.cast::<sockaddr_in>().read_unaligned() };
            let ip = Ipv4Addr::from(u32::from_be(a.sin_addr));
            Ok(SocketAddr::V4(SocketAddrV4::new(ip, u16::from_be(a.sin_port))))
        }
        AF_INET6 if len as usize >= size_of::<sockaddr_in6>() => {
            // SAFETY: dost bajtu pro sockaddr_in6.
            let a = unsafe { p.cast::<sockaddr_in6>().read_unaligned() };
            let ip = Ipv6Addr::from(a.sin6_addr);
            Ok(SocketAddr::V6(SocketAddrV6::new(ip, u16::from_be(a.sin6_port), a.sin6_flowinfo, a.sin6_scope_id)))
        }
        AF_INET | AF_INET6 => Err(errno::EINVAL),
        _ => Err(errno::EAFNOSUPPORT),
    }
}

/// Zapise adresu do `struct sockaddr *` (oriznute na `*len`), `*len` =
/// skutecna delka.
///
/// # Safety
///
/// `p` ma `*len` bajtu (nebo je NULL).
pub unsafe fn write_addr(a: SocketAddr, p: *mut c_void, len: *mut u32) {
    let mut buf = [0u8; 28];
    let n = match a {
        SocketAddr::V4(v) => {
            let s = sockaddr_in {
                sin_family: AF_INET as u16,
                sin_port: v.port().to_be(),
                sin_addr: u32::from(*v.ip()).to_be(),
                sin_zero: [0; 8],
            };
            // SAFETY: buf ma 28 >= 16 bajtu.
            unsafe { buf.as_mut_ptr().cast::<sockaddr_in>().write_unaligned(s) };
            size_of::<sockaddr_in>()
        }
        SocketAddr::V6(v) => {
            let s = sockaddr_in6 {
                sin6_family: AF_INET6 as u16,
                sin6_port: v.port().to_be(),
                sin6_flowinfo: v.flowinfo(),
                sin6_addr: v.ip().octets(),
                sin6_scope_id: v.scope_id(),
            };
            // SAFETY: buf ma 28 bajtu.
            unsafe { buf.as_mut_ptr().cast::<sockaddr_in6>().write_unaligned(s) };
            size_of::<sockaddr_in6>()
        }
    };
    if p.is_null() || len.is_null() {
        return;
    }
    // SAFETY: kontrakt C.
    unsafe {
        let room = (*len as usize).min(n);
        core::ptr::copy_nonoverlapping(buf.as_ptr(), p.cast::<u8>(), room);
        *len = n as u32;
    }
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub extern "C" fn socket(domain: c_int, ty: c_int, proto: c_int) -> c_int {
    let v6 = match domain {
        AF_INET => false,
        AF_INET6 => true,
        // Ramce linkove vrstvy jen pro netd (Linux bez CAP_NET_RAW = EPERM).
        AF_PACKET => return errno::ret(Err(errno::EPERM)),
        _ => return errno::ret(Err(errno::EAFNOSUPPORT)),
    };
    let kind = match (ty & 0xf, proto) {
        // Surove sockety (ICMP, vlastni hlavicky) aplikace nedostanou
        // (jako Linux bez CAP_NET_RAW); ping a spol. jdou pres netd.
        (SOCK_RAW, _) => return errno::ret(Err(errno::EPERM)),
        (SOCK_STREAM, 0 | IPPROTO_TCP) => SockKind::Stream,
        (SOCK_DGRAM, 0 | IPPROTO_UDP) => SockKind::Datagram,
        (SOCK_STREAM | SOCK_DGRAM, _) => return errno::ret(Err(errno::EPROTONOSUPPORT)),
        _ => return errno::ret(Err(errno::EPROTOTYPE)),
    };
    errno::ret_val(sys::socket(kind, v6, ty & SOCK_CLOEXEC != 0, ty & SOCK_NONBLOCK != 0)) as c_int
}

/// `socketpair`: `AF_UNIX` neni.
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub extern "C" fn socketpair(_d: c_int, _t: c_int, _p: c_int, _fds: *mut c_int) -> c_int {
    errno::ret(Err(errno::EAFNOSUPPORT))
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn bind(fd: c_int, a: *const c_void, len: u32) -> c_int {
    // SAFETY: kontrakt C.
    errno::ret(unsafe { read_addr(a, len) }.and_then(|a| sys::sock_bind(fd, a)))
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub extern "C" fn listen(fd: c_int, backlog: c_int) -> c_int {
    errno::ret(sys::sock_listen(fd, backlog.clamp(1, 4096) as u16))
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn connect(fd: c_int, a: *const c_void, len: u32) -> c_int {
    crate::pthread::testcancel();
    // SAFETY: kontrakt C.
    errno::ret(unsafe { read_addr(a, len) }.and_then(|a| sys::sock_connect(fd, a)))
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn accept4(fd: c_int, a: *mut c_void, len: *mut u32, flags: c_int) -> c_int {
    crate::pthread::testcancel();
    match sys::sock_accept(fd, flags & SOCK_CLOEXEC != 0, flags & SOCK_NONBLOCK != 0) {
        Ok((n, from)) => {
            // SAFETY: kontrakt C.
            unsafe { write_addr(from, a, len) };
            n
        }
        Err(e) => errno::ret(Err(e)),
    }
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn accept(fd: c_int, a: *mut c_void, len: *mut u32) -> c_int {
    // SAFETY: kontrakt C.
    unsafe { accept4(fd, a, len, 0) }
}

fn flags_ok(flags: c_int) -> Result<bool, c_int> {
    if flags & MSG_PEEK != 0 {
        return Err(errno::ENOTSUP);
    }
    Ok(flags & MSG_DONTWAIT != 0)
}

/// # Safety
///
/// `buf` ma `n` bajtu.
unsafe fn buf_mut<'a>(buf: *mut c_void, n: size_t) -> Result<&'a mut [u8], c_int> {
    if n == 0 {
        return Ok(&mut []);
    }
    if buf.is_null() {
        return Err(errno::EFAULT);
    }
    // SAFETY: kontrakt C.
    Ok(unsafe { core::slice::from_raw_parts_mut(buf.cast(), n) })
}

/// # Safety
///
/// `buf` ma `n` bajtu.
unsafe fn buf_ref<'a>(buf: *const c_void, n: size_t) -> Result<&'a [u8], c_int> {
    if n == 0 {
        return Ok(&[]);
    }
    if buf.is_null() {
        return Err(errno::EFAULT);
    }
    // SAFETY: kontrakt C.
    Ok(unsafe { core::slice::from_raw_parts(buf.cast(), n) })
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn recv(fd: c_int, buf: *mut c_void, n: size_t, flags: c_int) -> isize {
    crate::pthread::testcancel();
    // SAFETY: kontrakt C.
    let r = flags_ok(flags).and_then(|dw| unsafe { buf_mut(buf, n) }.and_then(|b| sys::sock_recv(fd, b, dw)));
    errno::ret_val(r)
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn send(fd: c_int, buf: *const c_void, n: size_t, flags: c_int) -> isize {
    crate::pthread::testcancel();
    // SAFETY: kontrakt C.
    let r = flags_ok(flags).and_then(|dw| unsafe { buf_ref(buf, n) }.and_then(|b| sys::sock_send(fd, b, dw)));
    errno::ret_val(r)
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn recvfrom(
    fd: c_int,
    buf: *mut c_void,
    n: size_t,
    flags: c_int,
    from: *mut c_void,
    len: *mut u32,
) -> isize {
    crate::pthread::testcancel();
    // SAFETY: kontrakt C.
    let r = flags_ok(flags).and_then(|dw| unsafe { buf_mut(buf, n) }.and_then(|b| sys::sock_recv_from(fd, b, dw)));
    match r {
        Ok((n, a)) => {
            // SAFETY: kontrakt C.
            unsafe { write_addr(a, from, len) };
            n as isize
        }
        Err(e) => errno::ret_val::<usize>(Err(e)),
    }
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn sendto(
    fd: c_int,
    buf: *const c_void,
    n: size_t,
    flags: c_int,
    to: *const c_void,
    len: u32,
) -> isize {
    crate::pthread::testcancel();
    if to.is_null() {
        // SAFETY: kontrakt C.
        return unsafe { send(fd, buf, n, flags) };
    }
    // SAFETY: kontrakt C.
    let r = flags_ok(flags)
        .and_then(|_| unsafe { read_addr(to, len) })
        .and_then(|a| unsafe { buf_ref(buf, n) }.and_then(|b| sys::sock_send_to(fd, b, a)));
    errno::ret_val(r)
}

/// `struct msghdr` (jako Linux).
#[repr(C)]
pub struct msghdr {
    pub msg_name: *mut c_void,
    pub msg_namelen: u32,
    pub msg_iov: *mut iovec,
    pub msg_iovlen: size_t,
    pub msg_control: *mut c_void,
    pub msg_controllen: size_t,
    pub msg_flags: c_int,
}

/// `struct cmsghdr` (jako Linux; data za hlavickou zarovnana na `size_t`).
#[repr(C)]
pub struct cmsghdr {
    pub cmsg_len: size_t,
    pub cmsg_level: c_int,
    pub cmsg_type: c_int,
}

/// Ridici zpravy (`msg_control`) netd neprenasi: `SCM_RIGHTS` (predani fd)
/// jde v systemu jinak (handly v kanalech), ostatni (`IP_PKTINFO` ...) nejsou.
/// Kazda neprazdna ridici zprava = `EOPNOTSUPP`.
///
/// # Safety
///
/// `m` je platna zprava (kontrakt C).
pub unsafe fn control_supported(m: &msghdr) -> Result<(), c_int> {
    if m.msg_controllen == 0 || m.msg_control.is_null() {
        return Ok(());
    }
    if m.msg_controllen < size_of::<cmsghdr>() {
        return Err(errno::EINVAL);
    }
    // SAFETY: aspon jedna hlavicka (kontrakt C).
    let h = unsafe { m.msg_control.cast::<cmsghdr>().read_unaligned() };
    if h.cmsg_len < size_of::<cmsghdr>() || h.cmsg_len > m.msg_controllen {
        return Err(errno::EINVAL);
    }
    Err(errno::ENOTSUP)
}

/// `sendmsg` bez ridicich zprav (`control_supported`): kusy se spoji do
/// jednoho zapisu.
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn sendmsg(fd: c_int, m: *const msghdr, flags: c_int) -> isize {
    // SAFETY: kontrakt C.
    let Some(m) = (unsafe { m.as_ref() }) else { return errno::ret_val::<usize>(Err(errno::EFAULT)) };
    // SAFETY: kontrakt C.
    if let Err(e) = unsafe { control_supported(m) } {
        return errno::ret_val::<usize>(Err(e));
    }
    let mut all = Vec::new();
    for i in 0..m.msg_iovlen {
        // SAFETY: msg_iov ma msg_iovlen polozek.
        let v = unsafe { &*m.msg_iov.add(i) };
        // SAFETY: kontrakt C.
        match unsafe { buf_ref(v.iov_base.cast(), v.iov_len) } {
            Ok(b) => all.extend_from_slice(b),
            Err(e) => return errno::ret_val::<usize>(Err(e)),
        }
    }
    // SAFETY: kontrakt C.
    unsafe { sendto(fd, all.as_ptr().cast(), all.len(), flags, m.msg_name, m.msg_namelen) }
}

/// `recvmsg` bez ridicich zprav (`msg_controllen` = 0): data se rozdeli do
/// kusu.
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn recvmsg(fd: c_int, m: *mut msghdr, flags: c_int) -> isize {
    // SAFETY: kontrakt C.
    let Some(m) = (unsafe { m.as_mut() }) else { return errno::ret_val::<usize>(Err(errno::EFAULT)) };
    let mut total = 0;
    for i in 0..m.msg_iovlen {
        // SAFETY: msg_iov ma msg_iovlen polozek.
        total += unsafe { (*m.msg_iov.add(i)).iov_len };
    }
    let mut tmp = vec![0u8; total];
    let mut namelen = m.msg_namelen;
    // SAFETY: kontrakt C.
    let n = unsafe { recvfrom(fd, tmp.as_mut_ptr().cast(), total, flags, m.msg_name, &mut namelen) };
    if n < 0 {
        return n;
    }
    m.msg_namelen = namelen;
    m.msg_controllen = 0;
    m.msg_flags = 0;
    let mut rest = &tmp[..n as usize];
    for i in 0..m.msg_iovlen {
        // SAFETY: msg_iov ma msg_iovlen polozek.
        let v = unsafe { &*m.msg_iov.add(i) };
        let k = rest.len().min(v.iov_len);
        // SAFETY: iov_base ma iov_len bajtu.
        unsafe { core::ptr::copy_nonoverlapping(rest.as_ptr(), v.iov_base, k) };
        rest = &rest[k..];
    }
    n
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub extern "C" fn shutdown(fd: c_int, how: c_int) -> c_int {
    match how {
        // Uzavreni cteni netd nezna: data se dal prijimaji, program je necte.
        SHUT_RD => errno::ret(sys::sock_is_stream(fd).map(|_| ())),
        SHUT_WR | SHUT_RDWR => errno::ret(sys::sock_shutdown_write(fd)),
        _ => errno::ret(Err(errno::EINVAL)),
    }
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn getsockname(fd: c_int, a: *mut c_void, len: *mut u32) -> c_int {
    match sys::sock_name(fd, false) {
        Ok(x) => {
            // SAFETY: kontrakt C.
            unsafe { write_addr(x, a, len) };
            0
        }
        Err(e) => errno::ret(Err(e)),
    }
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn getpeername(fd: c_int, a: *mut c_void, len: *mut u32) -> c_int {
    match sys::sock_name(fd, true) {
        Ok(x) => {
            // SAFETY: kontrakt C.
            unsafe { write_addr(x, a, len) };
            0
        }
        Err(e) => errno::ret(Err(e)),
    }
}

/// `struct timeval` pro `SO_RCVTIMEO`/`SO_SNDTIMEO`.
#[repr(C)]
#[derive(Clone, Copy)]
struct Tv {
    sec: i64,
    usec: i64,
}

fn tv_to(t: Tv) -> Option<Duration> {
    let d = Duration::from_secs(t.sec.max(0) as u64) + Duration::from_micros(t.usec.max(0) as u64);
    (!d.is_zero()).then_some(d)
}

/// `setsockopt`: casove limity se pouziji; `SO_REUSEADDR`, `SO_KEEPALIVE`,
/// `TCP_NODELAY` a velikosti bufferu se prijmou bez ucinku (netd je
/// nastavuje sam).
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn setsockopt(fd: c_int, level: c_int, name: c_int, v: *const c_void, len: u32) -> c_int {
    if let Err(e) = sys::sock_is_stream(fd) {
        return errno::ret(Err(e));
    }
    match (level, name) {
        (SOL_SOCKET, SO_RCVTIMEO | SO_SNDTIMEO) => {
            if v.is_null() || (len as usize) < size_of::<Tv>() {
                return errno::ret(Err(errno::EINVAL));
            }
            // SAFETY: kontrakt C (struct timeval).
            let t = tv_to(unsafe { v.cast::<Tv>().read_unaligned() });
            let r = if name == SO_RCVTIMEO {
                sys::sock_set_timeouts(fd, Some(t), None)
            } else {
                sys::sock_set_timeouts(fd, None, Some(t))
            };
            errno::ret(r)
        }
        // SO_LINGER: zavreni socketu necha netd dorucit data vzdy.
        (SOL_SOCKET, SO_REUSEADDR | SO_KEEPALIVE | SO_SNDBUF | SO_RCVBUF | SO_LINGER) | (IPPROTO_TCP, TCP_NODELAY) => 0,
        _ => errno::ret(Err(errno::ENOPROTOOPT)),
    }
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn getsockopt(fd: c_int, level: c_int, name: c_int, v: *mut c_void, len: *mut u32) -> c_int {
    let (stream, v6) = match sys::sock_is_stream(fd) {
        Ok(x) => x,
        Err(e) => return errno::ret(Err(e)),
    };
    if v.is_null() || len.is_null() {
        return errno::ret(Err(errno::EFAULT));
    }
    let put_int = |x: c_int| {
        // SAFETY: kontrakt C (aspon int).
        unsafe {
            if (*len as usize) < size_of::<c_int>() {
                return errno::ret(Err(errno::EINVAL));
            }
            v.cast::<c_int>().write_unaligned(x);
            *len = size_of::<c_int>() as u32;
        }
        0
    };
    match (level, name) {
        (SOL_SOCKET, SO_TYPE) => put_int(if stream { SOCK_STREAM } else { SOCK_DGRAM }),
        // Chyba neblokujiciho connect: connect se dokoncuje hned.
        (SOL_SOCKET, SO_ERROR) => put_int(0),
        (SOL_SOCKET, SO_ACCEPTCONN) => put_int(c_int::from(sys::sock_listening(fd).unwrap_or(false))),
        (SOL_SOCKET, SO_REUSEADDR | SO_KEEPALIVE) | (IPPROTO_TCP, TCP_NODELAY) => put_int(0),
        (SOL_SOCKET, SO_SNDBUF | SO_RCVBUF) => put_int(64 * 1024),
        (SOL_SOCKET, SO_DOMAIN) => put_int(if v6 { AF_INET6 } else { AF_INET }),
        (SOL_SOCKET, SO_PROTOCOL) => put_int(if stream { IPPROTO_TCP } else { IPPROTO_UDP }),
        (IPPROTO_IP, IP_TTL) if !v6 => put_int(DEFAULT_TTL),
        (SOL_SOCKET, SO_RCVTIMEO | SO_SNDTIMEO) => {
            let (r, w) = sys::sock_timeouts(fd).unwrap_or((None, None));
            let t = if name == SO_RCVTIMEO { r } else { w }.unwrap_or_default();
            // SAFETY: kontrakt C (struct timeval).
            unsafe {
                if (*len as usize) < size_of::<Tv>() {
                    return errno::ret(Err(errno::EINVAL));
                }
                v.cast::<Tv>().write_unaligned(Tv { sec: t.as_secs() as i64, usec: i64::from(t.subsec_micros()) });
                *len = size_of::<Tv>() as u32;
            }
            0
        }
        _ => errno::ret(Err(errno::ENOPROTOOPT)),
    }
}

// --- adresy ---------------------------------------------------------------

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub extern "C" fn htons(x: u16) -> u16 {
    x.to_be()
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub extern "C" fn ntohs(x: u16) -> u16 {
    u16::from_be(x)
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub extern "C" fn htonl(x: u32) -> u32 {
    x.to_be()
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub extern "C" fn ntohl(x: u32) -> u32 {
    u32::from_be(x)
}

/// Textova adresa (cisla, ne jmeno).
pub fn parse_ip(s: &[u8], family: c_int) -> Option<IpAddr> {
    let t = core::str::from_utf8(s).ok()?;
    match family {
        AF_INET => t.parse::<Ipv4Addr>().ok().map(IpAddr::V4),
        AF_INET6 => t.parse::<Ipv6Addr>().ok().map(IpAddr::V6),
        _ => t.parse::<IpAddr>().ok(),
    }
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn inet_pton(family: c_int, src: *const c_char, dst: *mut c_void) -> c_int {
    if family != AF_INET && family != AF_INET6 {
        return errno::ret(Err(errno::EAFNOSUPPORT));
    }
    // SAFETY: retezec C.
    match parse_ip(unsafe { c_bytes(src) }, family) {
        Some(IpAddr::V4(a)) => {
            // SAFETY: dst ma 4 bajty (in_addr).
            unsafe { core::ptr::copy_nonoverlapping(a.octets().as_ptr(), dst.cast::<u8>(), 4) };
            1
        }
        Some(IpAddr::V6(a)) => {
            // SAFETY: dst ma 16 bajtu (in6_addr).
            unsafe { core::ptr::copy_nonoverlapping(a.octets().as_ptr(), dst.cast::<u8>(), 16) };
            1
        }
        None => 0,
    }
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn inet_ntop(family: c_int, src: *const c_void, dst: *mut c_char, size: u32) -> *const c_char {
    let text = match family {
        // SAFETY: src ma 4 / 16 bajtu (kontrakt C).
        AF_INET => Ipv4Addr::from(unsafe { src.cast::<[u8; 4]>().read_unaligned() }).to_string(),
        AF_INET6 => Ipv6Addr::from(unsafe { src.cast::<[u8; 16]>().read_unaligned() }).to_string(),
        _ => {
            errno::set(errno::EAFNOSUPPORT);
            return core::ptr::null();
        }
    };
    if text.len() + 1 > size as usize {
        errno::set(errno::ENOSPC);
        return core::ptr::null();
    }
    // SAFETY: dst ma size bajtu.
    unsafe {
        core::ptr::copy_nonoverlapping(text.as_ptr(), dst, text.len());
        *dst.add(text.len()) = 0;
    }
    dst
}

/// `inet_addr`: adresa v sitovem poradi, `INADDR_NONE` = chyba.
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn inet_addr(s: *const c_char) -> u32 {
    // SAFETY: retezec C.
    match parse_ip(unsafe { c_bytes(s) }, AF_INET) {
        Some(IpAddr::V4(a)) => u32::from(a).to_be(),
        _ => u32::MAX,
    }
}

thread_local! {
    static NTOA: core::cell::RefCell<[u8; 16]> = const { core::cell::RefCell::new([0; 16]) };
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub extern "C" fn inet_ntoa(a: u32) -> *const c_char {
    let t = Ipv4Addr::from(u32::from_be(a)).to_string();
    NTOA.with(|b| {
        let mut b = b.borrow_mut();
        b[..t.len()].copy_from_slice(t.as_bytes());
        b[t.len()] = 0;
        b.as_ptr()
    })
}

// --- getaddrinfo ------------------------------------------------------------

pub const AI_PASSIVE: c_int = 0x01;
pub const AI_CANONNAME: c_int = 0x02;
pub const AI_NUMERICHOST: c_int = 0x04;
pub const AI_NUMERICSERV: c_int = 0x400;
pub const EAI_BADFLAGS: c_int = -1;
pub const EAI_NONAME: c_int = -2;
pub const EAI_AGAIN: c_int = -3;
pub const EAI_FAIL: c_int = -4;
pub const EAI_FAMILY: c_int = -6;
pub const EAI_SOCKTYPE: c_int = -7;
pub const EAI_SERVICE: c_int = -8;
pub const EAI_MEMORY: c_int = -10;
pub const EAI_SYSTEM: c_int = -11;
pub const NI_NUMERICHOST: c_int = 1;
pub const NI_NUMERICSERV: c_int = 2;

/// `struct addrinfo` (jako Linux).
#[repr(C)]
pub struct addrinfo {
    pub ai_flags: c_int,
    pub ai_family: c_int,
    pub ai_socktype: c_int,
    pub ai_protocol: c_int,
    pub ai_addrlen: u32,
    pub ai_addr: *mut c_void,
    pub ai_canonname: *mut c_char,
    pub ai_next: *mut addrinfo,
}

/// Uzel seznamu: `addrinfo` prvni (ukazatel na uzel = ukazatel na nej).
#[repr(C)]
struct Node {
    ai: addrinfo,
    addr: [u8; 28],
    canon: Option<CString>,
}

/// Port sluzby: cislo, nebo par znamych jmen.
pub fn service_port(s: &[u8], numeric_only: bool) -> Option<u16> {
    if let Ok(t) = core::str::from_utf8(s)
        && let Ok(p) = t.parse::<u16>()
    {
        return Some(p);
    }
    if numeric_only {
        return None;
    }
    match s {
        b"http" => Some(80),
        b"https" => Some(443),
        b"ftp" => Some(21),
        b"ssh" => Some(22),
        b"smtp" => Some(25),
        b"domain" => Some(53),
        b"ntp" => Some(123),
        _ => None,
    }
}

/// Seznam (adresa, druh socketu) pro `getaddrinfo`.
pub fn addr_list(ips: &[IpAddr], port: u16, family: c_int, socktype: c_int) -> Vec<(SocketAddr, c_int)> {
    let mut out = Vec::new();
    for ip in ips {
        let fam_ok = match family {
            AF_INET => ip.is_ipv4(),
            AF_INET6 => ip.is_ipv6(),
            _ => true,
        };
        if !fam_ok {
            continue;
        }
        for t in [SOCK_STREAM, SOCK_DGRAM] {
            if socktype == 0 || socktype == t {
                out.push((SocketAddr::new(*ip, port), t));
            }
        }
    }
    out
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn getaddrinfo(
    node: *const c_char,
    service: *const c_char,
    hints: *const addrinfo,
    res: *mut *mut addrinfo,
) -> c_int {
    if res.is_null() || (node.is_null() && service.is_null()) {
        return EAI_NONAME;
    }
    // SAFETY: kontrakt C.
    let (flags, family, socktype) = match unsafe { hints.as_ref() } {
        Some(h) => (h.ai_flags, h.ai_family, h.ai_socktype),
        None => (0, AF_UNSPEC, 0),
    };
    if !matches!(family, AF_UNSPEC | AF_INET | AF_INET6) {
        return EAI_FAMILY;
    }
    if !matches!(socktype, 0 | SOCK_STREAM | SOCK_DGRAM) {
        return EAI_SOCKTYPE;
    }
    let port = if service.is_null() {
        0
    } else {
        // SAFETY: retezec C.
        match service_port(unsafe { c_bytes(service) }, flags & AI_NUMERICSERV != 0) {
            Some(p) => p,
            None => return EAI_SERVICE,
        }
    };
    let ips: Vec<IpAddr> = if node.is_null() {
        let v4 = if flags & AI_PASSIVE != 0 { Ipv4Addr::UNSPECIFIED } else { Ipv4Addr::LOCALHOST };
        let v6 = if flags & AI_PASSIVE != 0 { Ipv6Addr::UNSPECIFIED } else { Ipv6Addr::LOCALHOST };
        vec![IpAddr::V4(v4), IpAddr::V6(v6)]
    } else {
        // SAFETY: retezec C.
        let name = unsafe { c_bytes(node) };
        match parse_ip(name, AF_UNSPEC) {
            Some(ip) => vec![ip],
            None if flags & AI_NUMERICHOST != 0 => return EAI_NONAME,
            None => {
                let Ok(n) = core::str::from_utf8(name) else { return EAI_NONAME };
                match sys::resolve(n) {
                    Ok(v) if !v.is_empty() => v,
                    Ok(_) => return EAI_NONAME,
                    Err(e) if e == errno::ENOENT => return EAI_NONAME,
                    Err(e) if e == errno::ETIMEDOUT || e == errno::EAGAIN => return EAI_AGAIN,
                    Err(e) => {
                        errno::set(e);
                        return EAI_SYSTEM;
                    }
                }
            }
        }
    };
    let list = addr_list(&ips, port, family, socktype);
    if list.is_empty() {
        return EAI_NONAME;
    }
    let canon = if flags & AI_CANONNAME != 0 && !node.is_null() {
        // SAFETY: retezec C.
        CString::new(unsafe { c_bytes(node) }).ok()
    } else {
        None
    };
    let mut head: *mut addrinfo = core::ptr::null_mut();
    for (i, (addr, t)) in list.into_iter().enumerate().rev() {
        let mut n = Box::new(Node {
            ai: addrinfo {
                ai_flags: flags,
                ai_family: if addr.is_ipv4() { AF_INET } else { AF_INET6 },
                ai_socktype: t,
                ai_protocol: if t == SOCK_STREAM { IPPROTO_TCP } else { IPPROTO_UDP },
                ai_addrlen: 0,
                ai_addr: core::ptr::null_mut(),
                ai_canonname: core::ptr::null_mut(),
                ai_next: head,
            },
            addr: [0; 28],
            canon: if i == 0 { canon.clone() } else { None },
        });
        let mut len = 28u32;
        // SAFETY: n.addr ma 28 bajtu.
        unsafe { write_addr(addr, n.addr.as_mut_ptr().cast(), &mut len) };
        n.ai.ai_addrlen = len;
        let raw = Box::into_raw(n);
        // SAFETY: uzel zije, adresy vnitrnich poli jsou stabilni (Box).
        unsafe {
            (*raw).ai.ai_addr = (*raw).addr.as_mut_ptr().cast();
            if let Some(c) = (*raw).canon.as_ref() {
                (*raw).ai.ai_canonname = c.as_ptr().cast_mut().cast();
            }
        }
        head = raw.cast();
    }
    // SAFETY: kontrakt C.
    unsafe { *res = head };
    0
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn freeaddrinfo(mut p: *mut addrinfo) {
    while !p.is_null() {
        // SAFETY: uzly vytvoril getaddrinfo (Box<Node>, addrinfo prvni).
        let n = unsafe { Box::from_raw(p.cast::<Node>()) };
        p = n.ai.ai_next;
    }
}

// --- stare rozhrani prekladu jmen (netdb.h) ---------------------------------------

pub const HOST_NOT_FOUND: c_int = 1;
pub const TRY_AGAIN: c_int = 2;
pub const NO_RECOVERY: c_int = 3;
pub const NO_DATA: c_int = 4;

std::thread_local! {
    static H_ERRNO: core::cell::Cell<c_int> = const { core::cell::Cell::new(0) };
}

/// Adresa `h_errno` aktualniho vlakna.
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub extern "C" fn __h_errno_location() -> *mut c_int {
    H_ERRNO.with(core::cell::Cell::as_ptr)
}

/// `gethostbyname`: stare rozhrani (neni bezpecne pro vlakna, jen IPv4)
/// neni; NULL s `h_errno` = `NO_RECOVERY`, programy maji `getaddrinfo`.
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub extern "C" fn gethostbyname(_name: *const c_char) -> *mut c_void {
    H_ERRNO.with(|h| h.set(NO_RECOVERY));
    core::ptr::null_mut()
}

/// `gethostbyaddr`: zpetny preklad neni (jako `getnameinfo`).
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub extern "C" fn gethostbyaddr(_addr: *const c_void, _len: u32, _ty: c_int) -> *mut c_void {
    H_ERRNO.with(|h| h.set(NO_RECOVERY));
    core::ptr::null_mut()
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub extern "C" fn hstrerror(e: c_int) -> *const c_char {
    let s: &[u8] = match e {
        HOST_NOT_FOUND => b"Unknown host\0",
        TRY_AGAIN => b"Host name lookup failure\0",
        NO_RECOVERY => b"Unknown server error\0",
        NO_DATA => b"No address associated with name\0",
        _ => b"Resolver internal error\0",
    };
    s.as_ptr().cast()
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub extern "C" fn gai_strerror(e: c_int) -> *const c_char {
    let s: &'static [u8] = match e {
        EAI_BADFLAGS => b"Bad value for ai_flags\0",
        EAI_NONAME => b"Name or service not known\0",
        EAI_AGAIN => b"Temporary failure in name resolution\0",
        EAI_FAIL => b"Non-recoverable failure in name resolution\0",
        EAI_FAMILY => b"ai_family not supported\0",
        EAI_SOCKTYPE => b"ai_socktype not supported\0",
        EAI_SERVICE => b"Servname not supported for ai_socktype\0",
        EAI_MEMORY => b"Memory allocation failure\0",
        EAI_SYSTEM => b"System error\0",
        _ => b"Unknown error\0",
    };
    s.as_ptr()
}

/// `getnameinfo`: jen ciselny tvar (zpetny preklad DNS neni).
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn getnameinfo(
    a: *const c_void,
    len: u32,
    host: *mut c_char,
    hostlen: u32,
    serv: *mut c_char,
    servlen: u32,
    _flags: c_int,
) -> c_int {
    // SAFETY: kontrakt C.
    let Ok(addr) = (unsafe { read_addr(a, len) }) else { return EAI_FAMILY };
    let put = |dst: *mut c_char, cap: u32, t: &str| -> bool {
        if dst.is_null() || cap == 0 {
            return true;
        }
        if t.len() + 1 > cap as usize {
            return false;
        }
        // SAFETY: dst ma cap bajtu.
        unsafe {
            core::ptr::copy_nonoverlapping(t.as_ptr(), dst, t.len());
            *dst.add(t.len()) = 0;
        }
        true
    };
    if !put(host, hostlen, &addr.ip().to_string()) || !put(serv, servlen, &addr.port().to_string()) {
        return -12; // EAI_OVERFLOW
    }
    0
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub static in6addr_any: [u8; 16] = [0; 16];
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub static in6addr_loopback: [u8; 16] = [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1];
