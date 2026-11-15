//! `ioctl` na socketu s dotazy na rozhrani (`SIOCGIF*`, `struct ifreq`,
//! `struct ifconf`; `net/if.h`). Rozhrani jsou tataz jako v `getifaddrs`
//! (`ifaddrs.rs`): smycka `lo` a `eth0` ze stavu netd. Adresy jen IPv4
//! (jako Linux); nastavovani (`SIOCSIF*`) neni (`ENOTTY`).

use core::ffi::c_void;
use std::net::{IpAddr, Ipv4Addr};

use crate::ifaddrs::{ETH_INDEX, IF_NAMESIZE, IFF_BROADCAST, IFF_LOOPBACK, IFF_RUNNING, IFF_UP, LO_INDEX, Slot, eth_flags, mask, slot};
use crate::sys::{self, Iface};
use crate::{c_int, errno};

/// `ioctl` socketu: je cteni u znacky dat mimo poradi (`sys/ioctl.h`)?
pub const SIOCATMARK: u64 = 0x8905;
pub const SIOCGIFNAME: u64 = 0x8910;
pub const SIOCGIFCONF: u64 = 0x8912;
pub const SIOCGIFFLAGS: u64 = 0x8913;
pub const SIOCGIFADDR: u64 = 0x8915;
pub const SIOCGIFBRDADDR: u64 = 0x8919;
pub const SIOCGIFNETMASK: u64 = 0x891b;
pub const SIOCGIFMTU: u64 = 0x8921;
pub const SIOCGIFHWADDR: u64 = 0x8927;
pub const SIOCGIFINDEX: u64 = 0x8933;
/// Druh hardwarove adresy (`sa_family` u `SIOCGIFHWADDR`).
pub const ARPHRD_ETHER: u16 = 1;
pub const ARPHRD_LOOPBACK: u16 = 772;
/// MTU smycky (jako Linux).
pub const LO_MTU: i32 = 65536;

/// Je `req` dotaz tohoto modulu? (Jen pak cte `ioctl` argument.)
pub fn is_iface_request(req: u64) -> bool {
    req == SIOCGIFCONF || (SIOCGIFNAME..=SIOCGIFINDEX).contains(&req)
}

/// `struct ifreq` (40 B): jmeno a unie hodnot (adresa, priznaky, cislo).
#[repr(C, align(8))]
#[derive(Clone, Copy)]
pub struct ifreq {
    pub ifr_name: [u8; IF_NAMESIZE],
    pub ifr_ifru: [u8; 24],
}

/// `struct ifconf`: delka bufferu a ukazatel na pole `ifreq`.
#[repr(C)]
pub struct ifconf {
    pub ifc_len: c_int,
    pub ifc_buf: *mut ifreq,
}

/// Udaje jednoho rozhrani pro ioctl.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IfInfo {
    pub name: &'static str,
    pub index: u32,
    pub flags: u32,
    pub mtu: i32,
    pub mac: Option<[u8; 6]>,
    /// Adresa IPv4 a delka prefixu.
    pub v4: Option<(Ipv4Addr, u8)>,
}

pub fn interfaces(iface: Option<Iface>) -> Vec<IfInfo> {
    let mut v = vec![IfInfo {
        name: "lo",
        index: LO_INDEX,
        flags: IFF_UP | IFF_LOOPBACK | IFF_RUNNING,
        mtu: LO_MTU,
        mac: None,
        v4: Some((Ipv4Addr::LOCALHOST, 8)),
    }];
    if let Some(i) = iface {
        let v4 = match i.addr {
            Some((IpAddr::V4(a), p)) => Some((a, p)),
            _ => None,
        };
        v.push(IfInfo { name: "eth0", index: ETH_INDEX, flags: eth_flags(&i), mtu: i32::from(i.mtu), mac: Some(i.mac), v4 });
    }
    v
}

fn name_of(r: &ifreq) -> &[u8] {
    let n = r.ifr_name.iter().position(|&c| c == 0).unwrap_or(IF_NAMESIZE);
    &r.ifr_name[..n]
}

fn put_slot(r: &mut ifreq, s: Slot) {
    r.ifr_ifru[..16].copy_from_slice(&s.0[..16]);
}

fn broadcast(a: Ipv4Addr, prefix: u8) -> Ipv4Addr {
    let m = match mask(IpAddr::V4(a), prefix) {
        IpAddr::V4(m) => u32::from(m),
        IpAddr::V6(_) => 0,
    };
    (u32::from(a) | !m).into()
}

/// Jeden dotaz `SIOCGIF*` (krome `SIOCGIFCONF`) nad seznamem rozhrani;
/// `None` = jiny pozadavek.
pub fn ifreq_query(list: &[IfInfo], req: u64, r: &mut ifreq) -> Option<Result<(), c_int>> {
    if req == SIOCGIFNAME {
        let idx = i32::from_ne_bytes([r.ifr_ifru[0], r.ifr_ifru[1], r.ifr_ifru[2], r.ifr_ifru[3]]);
        return Some(match list.iter().find(|i| i.index as i32 == idx) {
            Some(i) => {
                r.ifr_name = [0; IF_NAMESIZE];
                r.ifr_name[..i.name.len()].copy_from_slice(i.name.as_bytes());
                Ok(())
            }
            None => Err(errno::ENODEV),
        });
    }
    if !matches!(req, SIOCGIFFLAGS | SIOCGIFADDR | SIOCGIFBRDADDR | SIOCGIFNETMASK | SIOCGIFMTU | SIOCGIFHWADDR | SIOCGIFINDEX) {
        return None;
    }
    let name = name_of(r);
    let Some(i) = list.iter().find(|i| i.name.as_bytes() == name) else { return Some(Err(errno::ENODEV)) };
    r.ifr_ifru = [0; 24];
    let no_addr = Err(errno::EADDRNOTAVAIL);
    Some(match req {
        SIOCGIFFLAGS => {
            r.ifr_ifru[..2].copy_from_slice(&(i.flags as u16).to_ne_bytes());
            Ok(())
        }
        SIOCGIFMTU => {
            r.ifr_ifru[..4].copy_from_slice(&i.mtu.to_ne_bytes());
            Ok(())
        }
        SIOCGIFINDEX => {
            r.ifr_ifru[..4].copy_from_slice(&(i.index as i32).to_ne_bytes());
            Ok(())
        }
        SIOCGIFHWADDR => {
            let fam = if i.mac.is_some() { ARPHRD_ETHER } else { ARPHRD_LOOPBACK };
            r.ifr_ifru[..2].copy_from_slice(&fam.to_ne_bytes());
            r.ifr_ifru[2..8].copy_from_slice(&i.mac.unwrap_or_default());
            Ok(())
        }
        SIOCGIFADDR => match i.v4 {
            Some((a, _)) => {
                put_slot(r, slot(IpAddr::V4(a)));
                Ok(())
            }
            None => no_addr,
        },
        SIOCGIFNETMASK => match i.v4 {
            Some((a, p)) => {
                put_slot(r, slot(mask(IpAddr::V4(a), p)));
                Ok(())
            }
            None => no_addr,
        },
        _ => match (i.v4, i.flags & IFF_BROADCAST != 0) {
            (Some((a, p)), true) => {
                put_slot(r, slot(IpAddr::V4(broadcast(a, p))));
                Ok(())
            }
            _ => no_addr,
        },
    })
}

/// `SIOCGIFCONF`: rozhrani s adresou IPv4 do bufferu volajiciho
/// (`ifc_buf` NULL = jen potrebna delka).
///
/// # Safety
///
/// `c.ifc_buf` je NULL nebo ma `c.ifc_len` bajtu.
pub unsafe fn ifconf_query(list: &[IfInfo], c: &mut ifconf) {
    let all: Vec<&IfInfo> = list.iter().filter(|i| i.v4.is_some()).collect();
    let size = size_of::<ifreq>();
    if c.ifc_buf.is_null() {
        c.ifc_len = (all.len() * size) as c_int;
        return;
    }
    let room = usize::try_from(c.ifc_len).unwrap_or(0) / size;
    let mut n = 0;
    for i in all.iter().take(room) {
        let mut r = ifreq { ifr_name: [0; IF_NAMESIZE], ifr_ifru: [0; 24] };
        r.ifr_name[..i.name.len()].copy_from_slice(i.name.as_bytes());
        if let Some((a, _)) = i.v4 {
            put_slot(&mut r, slot(IpAddr::V4(a)));
        }
        // SAFETY: buffer volajiciho ma `room` polozek (kontrakt C).
        unsafe { c.ifc_buf.add(n).write_unaligned(r) };
        n += 1;
    }
    c.ifc_len = (n * size) as c_int;
}

/// `ioctl` na socketu: `SIOCGIF*`; `None` = jiny pozadavek.
///
/// # Safety
///
/// `p` ukazuje na `ifreq` (nebo `ifconf` u `SIOCGIFCONF`).
pub unsafe fn ioctl(req: u64, p: *mut c_void) -> Option<Result<(), c_int>> {
    if !is_iface_request(req) {
        return None;
    }
    if p.is_null() {
        return Some(Err(errno::EFAULT));
    }
    let list = interfaces(sys::net_interface());
    if req == SIOCGIFCONF {
        // SAFETY: kontrakt C (ifconf).
        let c = unsafe { &mut *p.cast::<ifconf>() };
        // SAFETY: kontrakt C.
        unsafe { ifconf_query(&list, c) };
        return Some(Ok(()));
    }
    // SAFETY: kontrakt C (ifreq, muze byt nezarovnany).
    let mut r = unsafe { p.cast::<ifreq>().read_unaligned() };
    let out = ifreq_query(&list, req, &mut r)?;
    if out.is_ok() {
        // SAFETY: kontrakt C.
        unsafe { p.cast::<ifreq>().write_unaligned(r) };
    }
    Some(out)
}
