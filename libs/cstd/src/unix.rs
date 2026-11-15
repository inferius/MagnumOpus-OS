//! Sockety `AF_UNIX` (`SOCK_STREAM`) pro funkce `sys/socket.h` (T1):
//! adresy `sockaddr_un`, ridici zpravy `SCM_RIGHTS` a obsluha funkci
//! socketu pro cislo `AF_UNIX`. Logika spojeni je v `osrt::unix`.
//!
//! - jmeno = cesta (i relativni vuci pracovnimu adresari); abstraktni
//!   jmeno (`sun_path[0] == 0`) neni (`ENOTSUP`),
//! - `SOCK_SEQPACKET` a `SOCK_DGRAM` nejsou (`EPROTONOSUPPORT`),
//! - `SO_PEERCRED`: uzivatel a skupina overene fsd (strana, ktera prijala
//!   spojeni), cislo procesu od klienta; klient o posluchaci vi jen 0.

use core::ffi::c_void;
use core::mem::size_of;

use crate::socket::{SCM_RIGHTS, SOL_SOCKET, cmsghdr, msghdr};
use crate::sys;
use crate::{c_int, errno, size_t};

pub const MSG_CTRUNC: c_int = 0x8;
pub const MSG_CMSG_CLOEXEC: c_int = 0x4000_0000;
pub const SO_PASSCRED: c_int = 16;
pub const SO_PEERCRED: c_int = 17;

/// `struct sockaddr_un` (110 B jako Linux).
#[repr(C)]
#[derive(Clone, Copy)]
pub struct sockaddr_un {
    pub sun_family: u16,
    pub sun_path: [u8; 108],
}

/// `struct ucred`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ucred {
    pub pid: c_int,
    pub uid: u32,
    pub gid: u32,
}

/// Odkud zacina `sun_path`.
const PATH_AT: usize = 2;

/// Cesta z adresy (`len` = `addrlen`): do prvni nuly nebo konce.
pub fn path_of(addr: &[u8]) -> Result<Vec<u8>, c_int> {
    let p = addr.get(PATH_AT..).unwrap_or(&[]);
    let p = &p[..p.len().min(108)];
    if p.first() == Some(&0) && p.len() > 1 {
        // Linux: abstraktni jmeno.
        return Err(errno::ENOTSUP);
    }
    let end = p.iter().position(|&b| b == 0).unwrap_or(p.len());
    if end == 0 {
        return Err(errno::EINVAL);
    }
    Ok(p[..end].to_vec())
}

/// Adresa `sockaddr_un` pro jmeno (`None` = bez jmena, jen rodina).
pub fn encode(path: Option<&[u8]>) -> Vec<u8> {
    let mut out = (crate::socket::AF_UNIX as u16).to_ne_bytes().to_vec();
    if let Some(p) = path {
        out.extend_from_slice(&p[..p.len().min(107)]);
        out.push(0);
    }
    out
}

/// Zapise adresu do `*p` (oriznute na `*len`), `*len` = skutecna delka.
///
/// # Safety
///
/// `p` ma `*len` bajtu (nebo je NULL).
pub unsafe fn write_addr(path: Option<&[u8]>, p: *mut c_void, len: *mut u32) {
    let a = encode(path);
    if p.is_null() || len.is_null() {
        return;
    }
    // SAFETY: kontrakt C.
    unsafe {
        let room = (*len as usize).min(a.len());
        core::ptr::copy_nonoverlapping(a.as_ptr(), p.cast::<u8>(), room);
        *len = a.len() as u32;
    }
}

/// Zarovnani dat ridici zpravy (`CMSG_ALIGN`).
const fn align(n: usize) -> usize {
    (n + size_of::<size_t>() - 1) & !(size_of::<size_t>() - 1)
}

/// Fd z ridicich zprav (`SCM_RIGHTS`); jine ridici zpravy = `EINVAL`.
pub fn rights_of(ctl: &[u8]) -> Result<Vec<c_int>, c_int> {
    let mut fds = Vec::new();
    let mut at = 0;
    let hdr = size_of::<cmsghdr>();
    while at + hdr <= ctl.len() {
        // SAFETY: aspon jedna hlavicka v bufferu.
        let h = unsafe { ctl.as_ptr().add(at).cast::<cmsghdr>().read_unaligned() };
        if h.cmsg_len < hdr || at + h.cmsg_len > ctl.len() {
            return Err(errno::EINVAL);
        }
        if h.cmsg_level != SOL_SOCKET || h.cmsg_type != SCM_RIGHTS {
            return Err(errno::EINVAL);
        }
        let data = &ctl[at + hdr..at + h.cmsg_len];
        for c in data.chunks_exact(size_of::<c_int>()) {
            fds.push(c_int::from_ne_bytes(c.try_into().unwrap()));
        }
        at += align(h.cmsg_len);
    }
    Ok(fds)
}

/// Zapise `fds` jako jednu zpravu `SCM_RIGHTS` do `ctl`; vraci (pouzita
/// delka, kolik fd se veslo).
pub fn put_rights(ctl: &mut [u8], fds: &[c_int]) -> (usize, usize) {
    let hdr = size_of::<cmsghdr>();
    if fds.is_empty() || ctl.len() < hdr + size_of::<c_int>() {
        return (0, 0);
    }
    let fit = ((ctl.len() - hdr) / size_of::<c_int>()).min(fds.len());
    let len = hdr + fit * size_of::<c_int>();
    let h = cmsghdr { cmsg_len: len, cmsg_level: SOL_SOCKET, cmsg_type: SCM_RIGHTS };
    // SAFETY: ctl ma aspon `hdr` bajtu.
    unsafe { ctl.as_mut_ptr().cast::<cmsghdr>().write_unaligned(h) };
    for (i, f) in fds[..fit].iter().enumerate() {
        ctl[hdr + i * 4..hdr + i * 4 + 4].copy_from_slice(&f.to_ne_bytes());
    }
    (align(len).min(ctl.len()), fit)
}

/// Bajty ridicich zprav `msghdr`.
///
/// # Safety
///
/// Kontrakt C (`msg_control` ma `msg_controllen` bajtu).
unsafe fn control<'a>(m: &msghdr) -> &'a [u8] {
    if m.msg_control.is_null() || m.msg_controllen == 0 {
        return &[];
    }
    // SAFETY: kontrakt C.
    unsafe { core::slice::from_raw_parts(m.msg_control.cast(), m.msg_controllen) }
}

/// `sendmsg` na socketu `AF_UNIX`.
///
/// # Safety
///
/// Kontrakt C.
pub unsafe fn sendmsg(fd: c_int, m: &msghdr, data: &[u8], flags: c_int) -> Result<usize, c_int> {
    // SAFETY: kontrakt C.
    let fds = rights_of(unsafe { control(m) })?;
    sys::unix_send(fd, data, &fds, flags & crate::socket::MSG_DONTWAIT != 0)
}

/// `recvmsg` na socketu `AF_UNIX`: data do `buf`, fd do `msg_control`.
///
/// # Safety
///
/// Kontrakt C.
pub unsafe fn recvmsg(fd: c_int, m: &mut msghdr, buf: &mut [u8], flags: c_int) -> Result<usize, c_int> {
    let room = (m.msg_controllen.saturating_sub(size_of::<cmsghdr>())) / size_of::<c_int>();
    let (n, fds, truncated) = sys::unix_recv(fd, buf, room, flags & crate::socket::MSG_DONTWAIT != 0, flags & MSG_CMSG_CLOEXEC != 0)?;
    m.msg_flags = 0;
    let ctl: &mut [u8] = if m.msg_control.is_null() || m.msg_controllen == 0 {
        &mut []
    } else {
        // SAFETY: kontrakt C.
        unsafe { core::slice::from_raw_parts_mut(m.msg_control.cast::<u8>(), m.msg_controllen) }
    };
    let (used, fit) = put_rights(ctl, &fds);
    // Fd, ktere se do bufferu nevesly, se zavrou (jako Linux).
    for &f in &fds[fit..] {
        let _ = sys::close(f);
    }
    if fit < fds.len() || truncated {
        m.msg_flags |= MSG_CTRUNC;
    }
    m.msg_controllen = used;
    if !m.msg_name.is_null() && m.msg_namelen > 0 {
        // Prijemce spojeneho socketu jmeno odesilatele nedostane.
        m.msg_namelen = 0;
    }
    Ok(n)
}

/// `getsockopt` na socketu `AF_UNIX`.
///
/// # Safety
///
/// `v` ma `*len` bajtu.
pub unsafe fn getsockopt(fd: c_int, level: c_int, name: c_int, v: *mut c_void, len: *mut u32) -> Result<(), c_int> {
    use crate::socket::*;
    if v.is_null() || len.is_null() {
        return Err(errno::EFAULT);
    }
    let put = |bytes: &[u8]| {
        // SAFETY: kontrakt C.
        unsafe {
            if (*len as usize) < bytes.len() {
                return Err(errno::EINVAL);
            }
            core::ptr::copy_nonoverlapping(bytes.as_ptr(), v.cast::<u8>(), bytes.len());
            *len = bytes.len() as u32;
        }
        Ok(())
    };
    let int = |x: c_int| put(&x.to_ne_bytes());
    match (level, name) {
        (SOL_SOCKET, SO_TYPE) => int(SOCK_STREAM),
        (SOL_SOCKET, SO_DOMAIN) => int(AF_UNIX),
        (SOL_SOCKET, SO_PROTOCOL) => int(0),
        (SOL_SOCKET, SO_ERROR) => int(0),
        (SOL_SOCKET, SO_ACCEPTCONN) => int(c_int::from(sys::unix_listening(fd)?)),
        (SOL_SOCKET, SO_SNDBUF | SO_RCVBUF) => int(64 * 1024),
        (SOL_SOCKET, SO_PASSCRED) => int(0),
        (SOL_SOCKET, SO_PEERCRED) => {
            let (pid, uid, gid) = sys::unix_peer(fd)?;
            let c = ucred { pid, uid, gid };
            // SAFETY: ucred je POD.
            let b = unsafe { core::slice::from_raw_parts((&raw const c).cast::<u8>(), size_of::<ucred>()) };
            put(b)
        }
        _ => Err(errno::ENOPROTOOPT),
    }
}

/// `setsockopt` na socketu `AF_UNIX`: velikosti bufferu a `SO_PASSCRED`
/// se prijmou bez ucinku.
pub fn setsockopt(fd: c_int, level: c_int, name: c_int) -> Result<(), c_int> {
    use crate::socket::*;
    sys::unix_listening(fd)?;
    match (level, name) {
        (SOL_SOCKET, SO_SNDBUF | SO_RCVBUF | SO_PASSCRED | SO_REUSEADDR | SO_RCVTIMEO | SO_SNDTIMEO) => Ok(()),
        _ => Err(errno::ENOPROTOOPT),
    }
}
