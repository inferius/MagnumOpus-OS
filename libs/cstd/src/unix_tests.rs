//! Testy adres a ridicich zprav `AF_UNIX` (`unix.rs`) na hostiteli.

use core::mem::size_of;

use crate::errno::{EAFNOSUPPORT, EINVAL, ENOTSUP, EPROTONOSUPPORT};
use crate::socket::{AF_INET, AF_UNIX, SOCK_DGRAM, SOCK_STREAM, cmsghdr};
use crate::unix::*;

fn addr(path: &[u8]) -> Vec<u8> {
    let mut a = (AF_UNIX as u16).to_ne_bytes().to_vec();
    a.extend_from_slice(path);
    a
}

#[test]
fn layout_matches_linux() {
    assert_eq!(size_of::<sockaddr_un>(), 110);
    assert_eq!(size_of::<ucred>(), 12);
    assert_eq!(size_of::<cmsghdr>(), 16);
}

#[test]
fn paths_from_address() {
    assert_eq!(path_of(&addr(b"/tmp/s\0zbytek")), Ok(b"/tmp/s".to_vec()));
    // Bez koncove nuly (addrlen = presna delka) a relativni cesta.
    assert_eq!(path_of(&addr(b"socket")), Ok(b"socket".to_vec()));
    assert_eq!(path_of(&addr(b"")), Err(EINVAL));
    assert_eq!(path_of(&addr(b"\0")), Err(EINVAL));
    assert_eq!(path_of(&addr(b"\0abstraktni")), Err(ENOTSUP));
    let long = vec![b'a'; 200];
    assert_eq!(path_of(&addr(&long)).unwrap().len(), 108);
    let e = encode(Some(b"/run/x"));
    assert_eq!(e, addr(b"/run/x\0"));
    assert_eq!(encode(None), addr(b""));
}

#[test]
fn rights_roundtrip() {
    let mut ctl = [0u8; 64];
    let (used, fit) = put_rights(&mut ctl, &[7, 9, 11]);
    assert_eq!(fit, 3);
    assert_eq!(used, 32, "CMSG_SPACE(3 * int)");
    assert_eq!(rights_of(&ctl[..used]), Ok(vec![7, 9, 11]));
    // Maly buffer: vejde se jen cast (MSG_CTRUNC dela volajici).
    let mut small = [0u8; 20];
    assert_eq!(put_rights(&mut small, &[1, 2]), (20, 1));
    assert_eq!(rights_of(&small), Ok(vec![1]));
    assert_eq!(put_rights(&mut [0u8; 8], &[1]), (0, 0));
    assert_eq!(rights_of(&[]), Ok(vec![]));
    // Spatna delka a jiny druh zpravy.
    let mut bad = ctl;
    bad[0] = 200;
    assert_eq!(rights_of(&bad[..used]), Err(EINVAL));
    let mut other = ctl;
    other[12] = 2; // SCM_CREDENTIALS
    assert_eq!(rights_of(&other[..used]), Err(EINVAL));
}

#[test]
fn socket_types_on_host() {
    // Na hostiteli AF_UNIX neni (sys/host.rs); typy mimo STREAM odmitne
    // uz cstd, stejne jako v systemu.
    assert_eq!(crate::socket::socket(AF_UNIX, SOCK_STREAM, 0), -1);
    assert_eq!(crate::errno::get(), EAFNOSUPPORT);
    assert_eq!(crate::socket::socket(AF_UNIX, SOCK_DGRAM, 0), -1);
    assert_eq!(crate::errno::get(), EPROTONOSUPPORT);
    let mut fds = [0; 2];
    // SAFETY: dve cisla.
    assert_eq!(unsafe { crate::socket::socketpair(AF_INET, SOCK_STREAM, 0, fds.as_mut_ptr()) }, -1);
    assert_eq!(crate::errno::get(), EAFNOSUPPORT);
}
