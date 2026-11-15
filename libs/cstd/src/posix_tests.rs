//! Testy doplnku POSIX pro porty a Windows vrstvu na hostiteli: errno,
//! `asctime`/`ctime`, `times`, `statfs`, `fchdir`, atributy vlaken,
//! semafory, `ifaddrs`/`net/if.h`, sockety (`SOCK_RAW`, ridici zpravy).

use core::ffi::c_void;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use crate::errno::{self, EAGAIN, EBADF, EFAULT, EINVAL, ENOENT, ENOSYS, ENOTDIR, ENOTSUP, ENXIO, EOVERFLOW, EPERM, ETIMEDOUT};
use crate::ifaddrs::*;
use crate::pthread::*;
use crate::semaphore::*;
use crate::socket::*;
use crate::stat::timespec;
use crate::statfs::{self as sfs, MOFS_MAGIC};
use crate::sys::{self, Iface};
use crate::time::{asc_text, asctime_r, ctime_r, gmtime_r, times, tm, tms};

fn cs(s: &str) -> std::ffi::CString {
    std::ffi::CString::new(s).unwrap()
}

#[test]
fn errno_table_is_complete() {
    for e in (1..=133).filter(|e| *e != 41 && *e != 58) {
        assert_ne!(errno::message(e), b"Unknown error\0", "errno {e} bez textu");
    }
    let h = include_str!("../include/errno.h");
    for line in ["#define EWOULDBLOCK EAGAIN", "#define EDEADLOCK EDEADLK", "#define EOPNOTSUPP ENOTSUP", "#define EOWNERDEAD 130", "#define ETXTBSY 26"] {
        assert!(h.contains(line), "{line}");
    }
}

#[test]
fn asctime_format() {
    // 1973-09-16 01:03:52 UTC = 116 989 432 s (nedele).
    let mut t = tm::default();
    // SAFETY: platne ukazatele.
    assert!(!unsafe { gmtime_r(&116_989_432, &mut t) }.is_null());
    assert_eq!(asc_text(&t), b"Sun Sep 16 01:03:52 1973\n");
    let mut buf = [0 as crate::c_char; 26];
    // SAFETY: buffer ma 26 bajtu.
    let p = unsafe { asctime_r(&t, buf.as_mut_ptr()) };
    assert_eq!(p, buf.as_mut_ptr());
    // SAFETY: zakonceno nulou.
    assert_eq!(unsafe { crate::c_bytes(p) }, b"Sun Sep 16 01:03:52 1973\n");
    t.tm_mon = 12;
    t.tm_wday = -1;
    t.tm_mday = 5;
    // Format normy C "%.3s %.3s%3d": den v mesici zarovnany na 3 znaky.
    assert_eq!(&asc_text(&t)[..10], b"??? ???  5");
    t.tm_year = 20_000;
    // SAFETY: buffer ma 26 bajtu.
    assert!(unsafe { asctime_r(&t, buf.as_mut_ptr()) }.is_null());
    assert_eq!(errno::get(), EOVERFLOW);
    // SAFETY: platne ukazatele; text ma vzdy 24 znaku + '\n'.
    let p = unsafe { ctime_r(&0, buf.as_mut_ptr()) };
    assert!(!p.is_null());
    // SAFETY: zakonceno nulou.
    assert_eq!(unsafe { crate::c_bytes(p) }.len(), 25);
}

#[test]
fn times_counts_elapsed_ticks() {
    let mut t = tms { tms_utime: 7, tms_stime: 7, tms_cutime: 7, tms_cstime: 7 };
    // SAFETY: platny ukazatel.
    let a = unsafe { times(&mut t) };
    assert!(a >= 0);
    assert_eq!((t.tms_utime, t.tms_stime, t.tms_cutime, t.tms_cstime), (0, 0, 0, 0));
    std::thread::sleep(Duration::from_millis(30));
    // SAFETY: NULL je dovoleny.
    let b = unsafe { times(core::ptr::null_mut()) };
    assert!(b >= a + 2, "{a} -> {b}");
}

#[test]
fn statfs_layout_and_errors() {
    assert_eq!(size_of::<sfs::statfs>(), 120);
    let h = include_str!("../include/sys/statfs.h");
    assert!(h.contains(&format!("#define MOFS_SUPER_MAGIC 0x{MOFS_MAGIC:X}")));
    let mut s = sfs::statfs::default();
    // SAFETY: platne ukazatele.
    unsafe {
        assert_eq!(sfs::statfs(cs(".").as_ptr().cast(), &mut s), 0);
        assert_eq!((s.f_type, s.f_bsize, s.f_frsize, s.f_namelen), (MOFS_MAGIC, 4096, 4096, 255));
        assert_eq!((s.f_blocks, s.f_bfree, s.f_bavail), (1 << 20, 1 << 18, 1 << 18));
        assert_eq!(sfs::statfs(cs("/neni/takova/cesta").as_ptr().cast(), &mut s), -1);
        assert_eq!(errno::get(), ENOENT);
        assert_eq!(sfs::fstatfs(3999, &mut s), -1);
        assert_eq!(errno::get(), EBADF);
        assert_eq!(sfs::fstatfs(1, core::ptr::null_mut()), -1);
        assert_eq!(errno::get(), EFAULT);
    }
    // Volnych bloku nikdy vic nez celkem.
    assert_eq!(sfs::from_space((512, 10, 99)).f_bfree, 10);
}

#[test]
fn fchdir_errors() {
    assert_eq!(crate::unistd::fchdir(3999), -1);
    assert_eq!(errno::get(), EBADF);
    // Stdout neni adresar. (Uspesne `fchdir` se na hostiteli netestuje:
    // menilo by pracovni adresar soubeznym testum.)
    assert_eq!(crate::unistd::fchdir(1), -1);
    assert_eq!(errno::get(), ENOTDIR);
    assert_eq!(sys::fchdir(1), Err(ENOTDIR));
}

#[test]
fn thread_attributes() {
    let mut a = core::mem::MaybeUninit::<pthread_attr_t>::uninit();
    // SAFETY: atribut inicializuje pthread_attr_init.
    unsafe {
        let a = a.as_mut_ptr();
        assert_eq!(pthread_attr_init(a), 0);
        let (mut addr, mut size, mut v) = (core::ptr::null_mut::<c_void>(), 0usize, 0);
        assert_eq!(pthread_attr_getstack(a, &mut addr, &mut size), 0);
        assert!(addr.is_null());
        assert_eq!(pthread_attr_getguardsize(a, &mut size), 0);
        assert_eq!(size, 4096);
        assert_eq!(pthread_attr_setguardsize(a, 0), 0);
        assert_eq!(pthread_attr_getguardsize(a, &mut size), 0);
        assert_eq!(size, 0);
        assert_eq!(pthread_attr_setscope(a, PTHREAD_SCOPE_PROCESS), ENOTSUP);
        assert_eq!(pthread_attr_setscope(a, 7), EINVAL);
        assert_eq!(pthread_attr_setscope(a, PTHREAD_SCOPE_SYSTEM), 0);
        assert_eq!(pthread_attr_getscope(a, &mut v), 0);
        assert_eq!(v, PTHREAD_SCOPE_SYSTEM);
        assert_eq!(pthread_attr_setinheritsched(a, PTHREAD_EXPLICIT_SCHED), 0);
        assert_eq!(pthread_attr_getinheritsched(a, &mut v), 0);
        assert_eq!(v, PTHREAD_EXPLICIT_SCHED);
        assert_eq!(pthread_attr_setschedpolicy(a, SCHED_OTHER), 0);
        assert_eq!(pthread_attr_setschedpolicy(a, SCHED_FIFO), ENOTSUP);
        assert_eq!(pthread_attr_setschedpolicy(a, 99), EINVAL);

        // Vlastni zasobnik: zarovnani a velikost se kontroluji.
        let mut mem = vec![0u64; 64 * 1024 / 8];
        let base = mem.as_mut_ptr().cast::<c_void>();
        assert_eq!(pthread_attr_setstack(a, base.byte_add(4), 64 * 1024 - 16), EINVAL);
        assert_eq!(pthread_attr_setstack(a, base, 1024), EINVAL);
        assert_eq!(pthread_attr_setstack(a, core::ptr::null_mut(), 64 * 1024), EINVAL);
        assert_eq!(pthread_attr_setstack(a, base, 64 * 1024), 0);
        assert_eq!(pthread_attr_getstack(a, &mut addr, &mut size), 0);
        assert_eq!((addr, size), (base, 64 * 1024));
        assert_eq!(pthread_attr_getstacksize(a, &mut size), 0);
        assert_eq!(size, 64 * 1024);

        // Vlakno s atributem vlastniho zasobniku pobezi (na hostiteli na
        // zasobniku std, v QEMU v `mem`: test ctest).
        extern "C" fn body(arg: *mut c_void) -> *mut c_void {
            // SAFETY: argument je AtomicU32 z testu.
            unsafe { &*arg.cast::<AtomicU32>() }.store(5, Ordering::SeqCst);
            arg
        }
        let flag = AtomicU32::new(0);
        let mut t = core::ptr::null_mut();
        assert_eq!(pthread_create(&mut t, a, body, (&raw const flag).cast_mut().cast()), 0);
        let mut res = core::ptr::null_mut();
        assert_eq!(pthread_join(t, &mut res), 0);
        assert_eq!(flag.load(Ordering::SeqCst), 5);
        assert_eq!(pthread_attr_destroy(a), 0);
        drop(mem);
    }
}

fn new_sem(v: u32) -> Box<sem_t> {
    let mut s = Box::new(core::mem::MaybeUninit::<sem_t>::uninit());
    // SAFETY: pamet pro sem_t.
    assert_eq!(unsafe { sem_init(s.as_mut_ptr(), 0, v) }, 0);
    // SAFETY: inicializovano.
    unsafe { Box::from_raw(Box::into_raw(s).cast()) }
}

#[test]
fn semaphores() {
    assert_eq!(size_of::<sem_t>(), 32);
    let s = Box::leak(new_sem(1));
    let sp: *mut sem_t = s;
    let mut v = 0;
    // SAFETY: platny semafor.
    unsafe {
        assert_eq!(sem_trywait(sp), 0);
        assert_eq!(sem_trywait(sp), -1);
        assert_eq!(errno::get(), EAGAIN);
        assert_eq!(sem_getvalue(sp, &mut v), 0);
        assert_eq!(v, 0);
        // Termin v minulosti (realny cas 1 s po 1970).
        let past = timespec { tv_sec: 1, tv_nsec: 0 };
        assert_eq!(sem_timedwait(sp, &past), -1);
        assert_eq!(errno::get(), ETIMEDOUT);
        let bad = timespec { tv_sec: 1, tv_nsec: 2_000_000_000 };
        assert_eq!(sem_timedwait(sp, &bad), -1);
        assert_eq!(errno::get(), EINVAL);
        assert_eq!(sem_clockwait(sp, 77, &past), -1);
        assert_eq!(errno::get(), EINVAL);
        // Kratky termin na monotonnich hodinach vyprsi.
        let now = sys::monotonic_ns() + 20_000_000;
        let soon = timespec { tv_sec: (now / 1_000_000_000) as i64, tv_nsec: (now % 1_000_000_000) as i64 };
        assert_eq!(sem_clockwait(sp, CLOCK_MONOTONIC, &soon), -1);
        assert_eq!(errno::get(), ETIMEDOUT);
    }
    // Cekajici vlakno vzbudi post z jineho vlakna.
    let addr = sp as usize;
    let waiter = std::thread::spawn(move || {
        // SAFETY: semafor zije do konce testu (leak).
        unsafe { sem_wait(addr as *mut sem_t) }
    });
    std::thread::sleep(Duration::from_millis(20));
    // SAFETY: platny semafor.
    unsafe {
        assert_eq!(sem_post(sp), 0);
    }
    assert_eq!(waiter.join().unwrap(), 0);
    // SAFETY: platny semafor.
    unsafe {
        let big = Box::leak(new_sem(SEM_VALUE_MAX));
        assert_eq!(sem_post(big), -1);
        assert_eq!(errno::get(), EOVERFLOW);
        assert!(sem_open(c"/x".as_ptr().cast(), 0).is_null());
        assert_eq!(errno::get(), ENOSYS);
        assert_eq!(sem_unlink(c"/x".as_ptr().cast()), -1);
        assert_eq!(sem_destroy(sp), 0);
        let mut m = core::mem::MaybeUninit::<sem_t>::uninit();
        assert_eq!(sem_init(m.as_mut_ptr(), 1, SEM_VALUE_MAX + 1), -1);
        assert_eq!(errno::get(), EINVAL);
    }
}

#[test]
fn interface_entries() {
    use std::net::IpAddr;
    let lo = entries(None);
    assert_eq!(lo.len(), 2);
    assert_eq!((lo[0].name, lo[0].addr), ("lo", Some((IpAddr::V4([127, 0, 0, 1].into()), 8))));
    assert_eq!(lo[1].addr.unwrap().1, 128);
    let v6 = Iface { addr: Some((IpAddr::V6("2001:db8::5".parse().unwrap()), 64)), up: false, ..sys::HOST_IFACE };
    let e = entries(Some(v6));
    assert_eq!(e.iter().map(|x| x.name).collect::<Vec<_>>(), ["lo", "lo", "eth0"]);
    assert_eq!(e[2].flags & IFF_UP, 0);
    let none = Iface { addr: None, ..sys::HOST_IFACE };
    assert_eq!(entries(Some(none))[1].addr, None);
    assert_eq!(mask(IpAddr::V4([0; 4].into()), 0), IpAddr::V4([0; 4].into()));
    assert_eq!(mask(IpAddr::V4([0; 4].into()), 20), IpAddr::V4([255, 255, 240, 0].into()));
    assert_eq!(mask(IpAddr::V6(std::net::Ipv6Addr::UNSPECIFIED), 64), IpAddr::V6("ffff:ffff:ffff:ffff::".parse().unwrap()));
}

/// IPv4 adresa z `sockaddr *` (sin_addr v poradi site).
///
/// # Safety
///
/// `p` je `sockaddr_in`.
unsafe fn v4(p: *mut c_void) -> [u8; 4] {
    // SAFETY: kontrakt.
    let a = unsafe { p.cast::<sockaddr_in>().read_unaligned() };
    assert_eq!(c_int::from(a.sin_family), AF_INET);
    u32::from_be(a.sin_addr).to_be_bytes()
}

use crate::c_int;

#[test]
fn getifaddrs_list_and_names() {
    let mut head = core::ptr::null_mut();
    // SAFETY: platne ukazatele; seznam se projde a uvolni.
    unsafe {
        assert_eq!(getifaddrs(&mut head), 0);
        let mut seen = Vec::new();
        let mut p = head;
        while !p.is_null() {
            let n = &*p;
            let name = String::from_utf8(crate::c_bytes(n.ifa_name).to_vec()).unwrap();
            if name == "eth0" {
                assert_eq!(v4(n.ifa_addr), [192, 0, 2, 10]);
                assert_eq!(v4(n.ifa_netmask), [255, 255, 255, 0]);
                assert_eq!(v4(n.ifa_ifu), [192, 0, 2, 255]);
                assert_eq!(n.ifa_flags & (IFF_UP | IFF_BROADCAST | IFF_RUNNING), IFF_UP | IFF_BROADCAST | IFF_RUNNING);
            }
            if name == "lo" && c_int::from(n.ifa_addr.cast::<u16>().read()) == AF_INET {
                assert_eq!(v4(n.ifa_addr), [127, 0, 0, 1]);
                assert!(n.ifa_ifu.is_null());
                assert_ne!(n.ifa_flags & IFF_LOOPBACK, 0);
            }
            seen.push(name);
            p = n.ifa_next;
        }
        assert_eq!(seen, ["lo", "eth0", "lo"]);
        freeifaddrs(head);
        freeifaddrs(core::ptr::null_mut());

        assert_eq!(if_nametoindex(c"lo".as_ptr().cast()), LO_INDEX);
        assert_eq!(if_nametoindex(c"eth0".as_ptr().cast()), ETH_INDEX);
        assert_eq!(if_nametoindex(c"wlan9".as_ptr().cast()), 0);
        assert_eq!(errno::get(), ENXIO);
        let mut buf = [0 as crate::c_char; IF_NAMESIZE];
        assert!(!if_indextoname(2, buf.as_mut_ptr()).is_null());
        assert_eq!(crate::c_bytes(buf.as_ptr()), b"eth0");
        assert!(if_indextoname(9, buf.as_mut_ptr()).is_null());
        let arr = if_nameindex();
        assert_eq!(((*arr).if_index, (*arr.add(1)).if_index, (*arr.add(2)).if_index), (1, 2, 0));
        assert!((*arr.add(2)).if_name.is_null());
        if_freenameindex(arr);
    }
}

#[test]
fn raw_sockets_and_control_messages() {
    assert_eq!(socket(AF_INET, SOCK_RAW, 1), -1);
    assert_eq!(errno::get(), EPERM);
    assert_eq!(socket(AF_PACKET, SOCK_DGRAM, 0), -1);
    assert_eq!(errno::get(), EPERM);
    assert_eq!(socket(AF_NETLINK, SOCK_DGRAM, 0), -1);
    assert_eq!(errno::get(), crate::errno::EAFNOSUPPORT);
    // SCM_RIGHTS s jednim fd: CMSG_LEN(4) = 20, CMSG_SPACE(4) = 24.
    let mut ctl = [0u64; 3];
    let h = cmsghdr { cmsg_len: 20, cmsg_level: SOL_SOCKET, cmsg_type: SCM_RIGHTS };
    // SAFETY: ctl ma 24 bajtu.
    unsafe { ctl.as_mut_ptr().cast::<cmsghdr>().write(h) };
    let mut m = msghdr {
        msg_name: core::ptr::null_mut(),
        msg_namelen: 0,
        msg_iov: core::ptr::null_mut(),
        msg_iovlen: 0,
        msg_control: ctl.as_mut_ptr().cast(),
        msg_controllen: 24,
        msg_flags: 0,
    };
    // SAFETY: platna zprava.
    unsafe {
        assert_eq!(control_supported(&m), Err(ENOTSUP));
        m.msg_controllen = 8;
        assert_eq!(control_supported(&m), Err(EINVAL));
        m.msg_controllen = 0;
        assert_eq!(control_supported(&m), Ok(()));
    }
    let hdr = include_str!("../include/sys/socket.h");
    for d in ["#define SOCK_RAW 3", "#define SCM_RIGHTS 1", "#define AF_PACKET 17", "#define SO_DOMAIN 39", "#define MSG_TRUNC 0x20"] {
        assert!(hdr.contains(d), "{d}");
    }
    let inh = include_str!("../include/netinet/in.h");
    for d in ["#define IP_TTL 2", "#define IPPROTO_ICMPV6 58", "#define IPV6_V6ONLY 26"] {
        assert!(inh.contains(d), "{d}");
    }
}

#[test]
fn interface_ioctls() {
    use crate::ifreq::*;
    assert_eq!(size_of::<ifreq>(), 40);
    let list = interfaces(Some(sys::HOST_IFACE));
    let req = |name: &str| {
        let mut r = ifreq { ifr_name: [0; IF_NAMESIZE], ifr_ifru: [0; 24] };
        r.ifr_name[..name.len()].copy_from_slice(name.as_bytes());
        r
    };
    let int = |r: &ifreq| i32::from_ne_bytes([r.ifr_ifru[0], r.ifr_ifru[1], r.ifr_ifru[2], r.ifr_ifru[3]]);
    let mut r = req("eth0");
    assert_eq!(ifreq_query(&list, SIOCGIFMTU, &mut r), Some(Ok(())));
    assert_eq!(int(&r), 1500);
    assert_eq!(ifreq_query(&list, SIOCGIFFLAGS, &mut r), Some(Ok(())));
    assert_eq!(u32::from(u16::from_ne_bytes([r.ifr_ifru[0], r.ifr_ifru[1]])), IFF_UP | IFF_BROADCAST | IFF_RUNNING | IFF_MULTICAST);
    assert_eq!(ifreq_query(&list, SIOCGIFINDEX, &mut r), Some(Ok(())));
    assert_eq!(int(&r), 2);
    assert_eq!(ifreq_query(&list, SIOCGIFHWADDR, &mut r), Some(Ok(())));
    assert_eq!(r.ifr_ifru[..8], [1, 0, 0x52, 0x54, 0, 0x12, 0x34, 0x56]);
    assert_eq!(ifreq_query(&list, SIOCGIFNETMASK, &mut r), Some(Ok(())));
    // SAFETY: unie zacina sockaddr_in.
    assert_eq!(unsafe { v4(r.ifr_ifru.as_mut_ptr().cast()) }, [255, 255, 255, 0]);
    assert_eq!(ifreq_query(&list, SIOCGIFBRDADDR, &mut r), Some(Ok(())));
    // SAFETY: unie zacina sockaddr_in.
    assert_eq!(unsafe { v4(r.ifr_ifru.as_mut_ptr().cast()) }, [192, 0, 2, 255]);
    let mut lo = req("lo");
    assert_eq!(ifreq_query(&list, SIOCGIFMTU, &mut lo), Some(Ok(())));
    assert_eq!(int(&lo), LO_MTU);
    assert_eq!(ifreq_query(&list, SIOCGIFBRDADDR, &mut lo), Some(Err(crate::errno::EADDRNOTAVAIL)));
    assert_eq!(ifreq_query(&list, SIOCGIFFLAGS, &mut req("wlan0")), Some(Err(crate::errno::ENODEV)));
    let mut by_index = req("");
    by_index.ifr_ifru[..4].copy_from_slice(&2i32.to_ne_bytes());
    assert_eq!(ifreq_query(&list, SIOCGIFNAME, &mut by_index), Some(Ok(())));
    assert_eq!(&by_index.ifr_name[..5], b"eth0\0");
    assert_eq!(ifreq_query(&list, 0x8914, &mut r), None, "SIOCSIFFLAGS neni");
    // SIOCGIFCONF: nejdriv delka, pak obsah.
    let mut c = ifconf { ifc_len: 0, ifc_buf: core::ptr::null_mut() };
    // SAFETY: NULL buffer = jen delka.
    unsafe { ifconf_query(&list, &mut c) };
    assert_eq!(c.ifc_len, 80);
    let mut buf = [ifreq { ifr_name: [0; IF_NAMESIZE], ifr_ifru: [0; 24] }; 1];
    c = ifconf { ifc_len: 40, ifc_buf: buf.as_mut_ptr() };
    // SAFETY: buffer ma 40 bajtu.
    unsafe { ifconf_query(&list, &mut c) };
    assert_eq!(c.ifc_len, 40);
    assert_eq!(&buf[0].ifr_name[..3], b"lo\0");
}

#[test]
fn legacy_netdb_and_headers() {
    assert!(gethostbyname(c"example.org".as_ptr().cast()).is_null());
    // SAFETY: adresa h_errno vlakna.
    assert_eq!(unsafe { *__h_errno_location() }, NO_RECOVERY);
    // SAFETY: staticky retezec.
    assert_eq!(unsafe { crate::c_bytes(hstrerror(HOST_NOT_FOUND)) }, b"Unknown host");
    let stat = include_str!("../include/sys/stat.h");
    for d in ["#define S_ISUID 04000", "#define S_ISGID 02000", "#define S_ISVTX 01000"] {
        assert!(stat.contains(d), "{d}");
    }
    let sig = include_str!("../include/signal.h");
    for d in ["#define SIGIO 29", "#define SIGXFSZ 25", "#define SIGXCPU 24", "#define SIGSYS 31"] {
        assert!(sig.contains(d), "{d}");
    }
    assert!(include_str!("../include/sys/time.h").contains("#include <sys/select.h>"));
    assert!(include_str!("../include/unistd.h").contains("void _exit(int code);"));
    // Skupiny procesu: proces vede vlastni relaci i skupinu.
    use crate::unistd::{getpgid, getpgrp, getsid, setpgid, setsid};
    let me = crate::signal::getpid();
    assert_eq!((setsid(), getsid(0), getpgid(0), getpgrp()), (me, me, me, me));
    assert_eq!(setpgid(0, 0), 0);
    assert_eq!(setpgid(0, me + 1), -1);
    assert_eq!(errno::get(), EPERM);
    assert_eq!(getsid(me + 1), -1);
    assert_eq!(errno::get(), crate::errno::ESRCH);
}
