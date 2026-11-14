//! Testy socketu a cekani na hostiteli: adresy, getaddrinfo (bez DNS),
//! epoll (ET, ONESHOT), select nad soubory.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};

use crate::poll::*;
use crate::socket::*;

#[test]
fn sockaddr_roundtrip() {
    for a in ["10.0.2.15:8080", "[2001:db8::1]:443"] {
        let a: SocketAddr = a.parse().unwrap();
        let mut buf = [0u8; 28];
        let mut len = 28u32;
        // SAFETY: buffer 28 B.
        unsafe { write_addr(a, buf.as_mut_ptr().cast(), &mut len) };
        assert_eq!(len as usize, if a.is_ipv4() { 16 } else { 28 });
        // SAFETY: prave zapsano.
        assert_eq!(unsafe { read_addr(buf.as_ptr().cast(), len) }, Ok(a));
    }
    // Port v sitovem poradi.
    let mut buf = [0u8; 16];
    let mut len = 16u32;
    // SAFETY: buffer 16 B.
    unsafe { write_addr("1.2.3.4:258".parse().unwrap(), buf.as_mut_ptr().cast(), &mut len) };
    assert_eq!(&buf[..8], &[2, 0, 1, 2, 1, 2, 3, 4]);
    // Oriznuti: kratky buffer, ale len = skutecna delka.
    let mut small = [0u8; 4];
    let mut l = 4u32;
    // SAFETY: buffer 4 B.
    unsafe { write_addr("[::1]:1".parse().unwrap(), small.as_mut_ptr().cast(), &mut l) };
    assert_eq!(l, 28);
}

#[test]
fn addresses_and_services() {
    assert_eq!(parse_ip(b"127.0.0.1", AF_INET), Some(IpAddr::V4(Ipv4Addr::LOCALHOST)));
    assert_eq!(parse_ip(b"::1", AF_INET), None);
    assert!(parse_ip(b"::1", AF_UNSPEC).is_some());
    assert_eq!(service_port(b"8080", true), Some(8080));
    assert_eq!(service_port(b"https", false), Some(443));
    assert_eq!(service_port(b"https", true), None);
    let ips = [IpAddr::V4(Ipv4Addr::LOCALHOST), "::1".parse().unwrap()];
    assert_eq!(addr_list(&ips, 80, AF_UNSPEC, 0).len(), 4, "obe rodiny, TCP i UDP");
    let v4 = addr_list(&ips, 80, AF_INET, SOCK_STREAM);
    assert_eq!(v4, vec![("127.0.0.1:80".parse().unwrap(), SOCK_STREAM)]);
    assert_eq!(htons(0x1234), 0x1234u16.to_be());
}

#[test]
fn epoll_levels_edges_oneshot() {
    let e = Epoll::default();
    e.ctl(EPOLL_CTL_ADD, 5, Some((EPOLLIN, 50))).unwrap();
    e.ctl(EPOLL_CTL_ADD, 6, Some((EPOLLIN | EPOLLET, 60))).unwrap();
    e.ctl(EPOLL_CTL_ADD, 7, Some((EPOLLOUT | EPOLLONESHOT, 70))).unwrap();
    assert_eq!(e.ctl(EPOLL_CTL_ADD, 5, Some((EPOLLIN, 0))), Err(crate::errno::EEXIST));
    let ready = [(5, POLLIN as u16), (6, POLLIN as u16), (7, POLLOUT as u16)];
    assert_eq!(e.collect(&ready, 10), vec![(50, EPOLLIN), (60, EPOLLIN), (70, EPOLLOUT)]);
    // Podruhe: urovnovy znovu, ET bez zmeny ne, ONESHOT vypnuty.
    assert_eq!(e.collect(&ready, 10), vec![(50, EPOLLIN)]);
    // ET: novy bit (HUP) se ohlasi.
    let ready2 = [(6, (POLLIN | POLLHUP) as u16)];
    assert_eq!(e.collect(&ready2, 10), vec![(60, EPOLLHUP)]);
    // MOD znovu zapne ONESHOT, DEL odebere.
    e.ctl(EPOLL_CTL_MOD, 7, Some((EPOLLOUT | EPOLLONESHOT, 71))).unwrap();
    e.ctl(EPOLL_CTL_DEL, 5, None).unwrap();
    assert_eq!(e.collect(&ready, 1), vec![(71, EPOLLOUT)], "max 1 a ET 6 bez zmeny");
    assert_eq!(e.ctl(EPOLL_CTL_DEL, 5, None), Err(crate::errno::ENOENT));
}

#[test]
fn epoll_and_select_over_fd_table() {
    let (r, w) = crate::sys::pipe(false, false).unwrap();
    let ep = epoll_create1(0);
    assert!(ep >= 3);
    let mut ev = epoll_event { events: EPOLLIN, data: 42 };
    // SAFETY: platne ukazatele.
    assert_eq!(unsafe { epoll_ctl(ep, EPOLL_CTL_ADD, r, &mut ev) }, 0);
    let mut out = [epoll_event { events: 0, data: 0 }; 4];
    // SAFETY: platny buffer.
    let n = unsafe { epoll_wait(ep, out.as_mut_ptr(), 4, 0) };
    assert_eq!(n, 1);
    let got = out[0];
    let (events, data) = (got.events, got.data);
    assert_eq!((events, data), (EPOLLIN, 42));
    // select: zapis do roury je pripraveny.
    let mut ws = fd_set { bits: [0; FD_SETSIZE / 64] };
    ws.bits[0] |= 1 << w;
    let mut tv = timeval { tv_sec: 0, tv_usec: 0 };
    // SAFETY: platne mnoziny.
    let n = unsafe { select(w + 1, core::ptr::null_mut(), &mut ws, core::ptr::null_mut(), &mut tv) };
    assert_eq!(n, 1);
    assert_ne!(ws.bits[0] & (1 << w), 0);
    for fd in [r, w, ep] {
        crate::sys::close(fd).unwrap();
    }
}
