//! `ifaddrs.h` (`getifaddrs`, `freeifaddrs`) a `net/if.h` (`if_nameindex`,
//! `if_nametoindex`, `if_indextoname`).
//!
//! Rozhrani jsou dve: smycka `lo` (index 1; 127.0.0.1/8 a ::1/128, netd
//! smycku obslouzi sam) a rozhrani site `eth0` (index 2) ze stavu netd
//! (`INFO`), jen kdyz proces ma relaci site s pravem `INFO`. Rozhrani bez
//! adresy se ve `getifaddrs` objevi s `ifa_addr` = NULL. Polozky
//! `AF_PACKET` (MAC) nejsou.

use core::ffi::c_void;
use std::net::IpAddr;

use crate::socket::{AF_INET, AF_INET6, sockaddr_in, sockaddr_in6};
use crate::sys::{self, Iface};
use crate::{c_bytes, c_char, c_int, errno};

pub const IFF_UP: u32 = 0x1;
pub const IFF_BROADCAST: u32 = 0x2;
pub const IFF_LOOPBACK: u32 = 0x8;
pub const IFF_RUNNING: u32 = 0x40;
pub const IFF_MULTICAST: u32 = 0x1000;
/// `IF_NAMESIZE` (vcetne nuly).
pub const IF_NAMESIZE: usize = 16;

pub const LO_INDEX: u32 = 1;
pub const ETH_INDEX: u32 = 2;

/// `struct ifaddrs` (jako glibc; `ifa_ifu` = broadcast nebo cil).
#[repr(C)]
pub struct ifaddrs {
    pub ifa_next: *mut ifaddrs,
    pub ifa_name: *mut c_char,
    pub ifa_flags: u32,
    pub ifa_addr: *mut c_void,
    pub ifa_netmask: *mut c_void,
    pub ifa_ifu: *mut c_void,
    pub ifa_data: *mut c_void,
}

/// Misto pro `sockaddr_in` i `sockaddr_in6` (zarovnane jako `sockaddr_storage`).
#[repr(C, align(8))]
#[derive(Clone, Copy)]
pub(crate) struct Slot(pub(crate) [u8; 28]);

/// Jedna polozka seznamu (jedna alokace; `ifaddrs` musi byt prvni).
#[repr(C)]
struct Node {
    ifa: ifaddrs,
    name: [u8; IF_NAMESIZE],
    addr: Slot,
    mask: Slot,
    brd: Slot,
}

/// Polozka pred prevodem do C.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub name: &'static str,
    pub flags: u32,
    pub addr: Option<(IpAddr, u8)>,
}

/// Maska z delky prefixu.
pub fn mask(ip: IpAddr, prefix: u8) -> IpAddr {
    match ip {
        IpAddr::V4(_) => {
            let p = u32::from(prefix.min(32));
            let m = if p == 0 { 0 } else { u32::MAX << (32 - p) };
            IpAddr::V4(m.into())
        }
        IpAddr::V6(_) => {
            let p = u32::from(prefix.min(128));
            let m = if p == 0 { 0 } else { u128::MAX << (128 - p) };
            IpAddr::V6(m.into())
        }
    }
}

pub(crate) fn eth_flags(i: &Iface) -> u32 {
    let run = if i.up { IFF_UP | IFF_RUNNING } else { 0 };
    run | IFF_BROADCAST | IFF_MULTICAST
}

/// Seznam polozek: IPv4 (smycka, rozhrani), pak IPv6, jako Linux.
pub fn entries(iface: Option<Iface>) -> Vec<Entry> {
    let lo = IFF_UP | IFF_LOOPBACK | IFF_RUNNING;
    let mut v4 = vec![Entry { name: "lo", flags: lo, addr: Some((IpAddr::V4([127, 0, 0, 1].into()), 8)) }];
    let mut v6 = vec![Entry { name: "lo", flags: lo, addr: Some((IpAddr::V6(std::net::Ipv6Addr::LOCALHOST), 128)) }];
    if let Some(i) = iface {
        let e = Entry { name: "eth0", flags: eth_flags(&i), addr: i.addr };
        match i.addr {
            Some((IpAddr::V6(_), _)) => v6.push(e),
            _ => v4.push(e),
        }
    }
    v4.extend(v6);
    v4
}

pub(crate) fn slot(ip: IpAddr) -> Slot {
    let mut s = Slot([0; 28]);
    match ip {
        IpAddr::V4(a) => {
            let v = sockaddr_in { sin_family: AF_INET as u16, sin_port: 0, sin_addr: u32::from(a).to_be(), sin_zero: [0; 8] };
            // SAFETY: Slot ma 28 >= 16 bajtu.
            unsafe { s.0.as_mut_ptr().cast::<sockaddr_in>().write_unaligned(v) };
        }
        IpAddr::V6(a) => {
            let v = sockaddr_in6 { sin6_family: AF_INET6 as u16, sin6_port: 0, sin6_flowinfo: 0, sin6_addr: a.octets(), sin6_scope_id: 0 };
            // SAFETY: Slot ma 28 bajtu.
            unsafe { s.0.as_mut_ptr().cast::<sockaddr_in6>().write_unaligned(v) };
        }
    }
    s
}

fn node(e: &Entry, next: *mut ifaddrs) -> *mut ifaddrs {
    let mut name = [0u8; IF_NAMESIZE];
    name[..e.name.len()].copy_from_slice(e.name.as_bytes());
    let empty = Slot([0; 28]);
    let null: *mut c_void = core::ptr::null_mut();
    let ifa = ifaddrs { ifa_next: next, ifa_name: null.cast(), ifa_flags: e.flags, ifa_addr: null, ifa_netmask: null, ifa_ifu: null, ifa_data: null };
    let n = Box::into_raw(Box::new(Node { ifa, name, addr: empty, mask: empty, brd: empty }));
    // SAFETY: cerstva alokace; ukazatele miri do ni (Box se nehybe).
    unsafe {
        (*n).ifa.ifa_name = (*n).name.as_mut_ptr().cast();
        if let Some((ip, prefix)) = e.addr {
            let m = mask(ip, prefix);
            (*n).addr = slot(ip);
            (*n).mask = slot(m);
            (*n).ifa.ifa_addr = (&raw mut (*n).addr).cast();
            (*n).ifa.ifa_netmask = (&raw mut (*n).mask).cast();
            if let (IpAddr::V4(a), IpAddr::V4(m), true) = (ip, m, e.flags & IFF_BROADCAST != 0) {
                (*n).brd = slot(IpAddr::V4((u32::from(a) | !u32::from(m)).into()));
                (*n).ifa.ifa_ifu = (&raw mut (*n).brd).cast();
            }
        }
    }
    n.cast()
}

/// # Safety
///
/// `out` je platny ukazatel.
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn getifaddrs(out: *mut *mut ifaddrs) -> c_int {
    if out.is_null() {
        return errno::ret(Err(errno::EFAULT));
    }
    let mut head = core::ptr::null_mut();
    for e in entries(sys::net_interface()).iter().rev() {
        head = node(e, head);
    }
    // SAFETY: kontrakt C.
    unsafe { *out = head };
    0
}

/// # Safety
///
/// `p` je seznam z `getifaddrs` (nebo NULL).
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn freeifaddrs(mut p: *mut ifaddrs) {
    while !p.is_null() {
        // SAFETY: polozka z `node` (Box<Node>, `ifaddrs` je prvni).
        let n = unsafe { Box::from_raw(p.cast::<Node>()) };
        p = n.ifa.ifa_next;
    }
}

/// Rozhrani (index, jmeno) jako `if_nameindex`.
pub fn names(iface: Option<Iface>) -> Vec<(u32, &'static str)> {
    let mut v = vec![(LO_INDEX, "lo")];
    if iface.is_some() {
        v.push((ETH_INDEX, "eth0"));
    }
    v
}

/// `struct if_nameindex`.
#[repr(C)]
pub struct if_nameindex {
    pub if_index: u32,
    pub if_name: *mut c_char,
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn if_nametoindex(name: *const c_char) -> u32 {
    if name.is_null() {
        return 0;
    }
    // SAFETY: retezec C.
    let n = unsafe { c_bytes(name) };
    match names(sys::net_interface()).into_iter().find(|(_, x)| x.as_bytes() == n) {
        Some((i, _)) => i,
        None => {
            errno::set(errno::ENXIO);
            0
        }
    }
}

/// # Safety
///
/// `buf` ma aspon `IF_NAMESIZE` bajtu.
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn if_indextoname(index: u32, buf: *mut c_char) -> *mut c_char {
    match names(sys::net_interface()).into_iter().find(|(i, _)| *i == index) {
        Some((_, n)) if !buf.is_null() => {
            // SAFETY: `buf` ma IF_NAMESIZE bajtu, jmena jsou kratsi.
            unsafe {
                core::ptr::copy_nonoverlapping(n.as_ptr(), buf, n.len());
                *buf.add(n.len()) = 0;
            }
            buf
        }
        _ => {
            errno::set(errno::ENXIO);
            core::ptr::null_mut()
        }
    }
}

/// Pole konci polozkou `{0, NULL}`; jmena jsou samostatne alokace
/// (uvolni je `if_freenameindex`).
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub extern "C" fn if_nameindex() -> *mut if_nameindex {
    let list = names(sys::net_interface());
    let mut arr: Vec<if_nameindex> = Vec::with_capacity(list.len() + 1);
    for (i, n) in &list {
        let s = std::ffi::CString::new(*n).unwrap_or_default();
        arr.push(if_nameindex { if_index: *i, if_name: s.into_raw().cast() });
    }
    arr.push(if_nameindex { if_index: 0, if_name: core::ptr::null_mut() });
    let b = arr.into_boxed_slice();
    Box::into_raw(b).cast()
}

/// # Safety
///
/// `p` je pole z `if_nameindex` (nebo NULL).
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn if_freenameindex(p: *mut if_nameindex) {
    if p.is_null() {
        return;
    }
    let mut n = 0;
    // SAFETY: pole konci polozkou s NULL jmenem; jmena z CString::into_raw.
    unsafe {
        while !(*p.add(n)).if_name.is_null() {
            drop(std::ffi::CString::from_raw((*p.add(n)).if_name.cast()));
            n += 1;
        }
        drop(Box::from_raw(core::ptr::slice_from_raw_parts_mut(p, n + 1)));
    }
}
