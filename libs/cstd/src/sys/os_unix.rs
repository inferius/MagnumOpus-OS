//! Sockety `AF_UNIX` nad `osrt::unix` (cisla fd, chyby `errno`).

use std::os::magnumopus::runtime as rt;
use std::sync::Arc;

use rt::fd::{self, Description, Object};
use rt::unix::UnixSocket;

use super::R;
use crate::errno;

fn e(err: rt::Error) -> crate::c_int {
    match err {
        rt::Error::PeerClosed => errno::EPIPE,
        rt::Error::ShouldWait => errno::EAGAIN,
        rt::Error::AlreadyExists => errno::EADDRINUSE,
        rt::Error::BadState => errno::ENOTCONN,
        other => errno::from_os(other as i32),
    }
}

fn status(nonblock: bool) -> u32 {
    super::status::RDWR | if nonblock { super::status::NONBLOCK } else { 0 }
}

fn insert(s: UnixSocket, cloexec: bool, nonblock: bool) -> R<i32> {
    fd::insert(Description::new(Object::Unix(s), status(nonblock)), 0, cloexec).map_err(|err| match err {
        rt::Error::NoResources => errno::EMFILE,
        other => e(other),
    })
}

fn unix<T>(fd: i32, f: impl FnOnce(&UnixSocket, bool) -> rt::Result<T>) -> R<T> {
    let d = fd::get(fd).map_err(|_| errno::EBADF)?;
    let u = d.unix().ok_or(errno::ENOTSOCK)?;
    f(u, d.is_nonblocking()).map_err(e)
}

/// Je cislo socket `AF_UNIX`?
pub fn is_unix(fd: i32) -> bool {
    fd::get(fd).is_ok_and(|d| d.unix().is_some())
}

pub fn unix_socket(cloexec: bool, nonblock: bool) -> R<i32> {
    insert(UnixSocket::new(), cloexec, nonblock)
}

pub fn unix_pair(cloexec: bool, nonblock: bool) -> R<(i32, i32)> {
    let (a, b) = UnixSocket::pair().map_err(e)?;
    let x = insert(a, cloexec, nonblock)?;
    match insert(b, cloexec, nonblock) {
        Ok(y) => Ok((x, y)),
        Err(err) => {
            let _ = fd::close(x);
            Err(err)
        }
    }
}

pub fn unix_bind(fd: i32, path: &[u8]) -> R<()> {
    unix(fd, |u, _| u.bind(path))
}

pub fn unix_listen(fd: i32) -> R<()> {
    unix(fd, |u, _| u.listen())
}

pub fn unix_listening(fd: i32) -> R<bool> {
    unix(fd, |u, _| Ok(u.is_listening()))
}

pub fn unix_connect(fd: i32, path: &[u8]) -> R<()> {
    unix(fd, |u, nb| u.connect(path, nb))
}

pub fn unix_accept(fd: i32, cloexec: bool, nonblock: bool) -> R<i32> {
    let s = unix(fd, |u, nb| u.accept(nb))?;
    insert(s, cloexec, nonblock)
}

/// Odesle data a fd (`SCM_RIGHTS`).
pub fn unix_send(fd: i32, data: &[u8], fds: &[i32], dontwait: bool) -> R<usize> {
    let descs: Vec<Arc<Description>> = fds.iter().map(|&f| fd::get(f).map_err(|_| errno::EBADF)).collect::<R<_>>()?;
    // Druh fd, ktery predat nejde (socket TCP, epoll) = ENOTSUP.
    unix(fd, |u, nb| u.send(data, &descs, nb || dontwait))
}

/// Prijme data a nejvys `max_fds` fd (vlozi je do tabulky); vraci
/// (delka, cisla fd, fd se nevesly).
pub fn unix_recv(fd: i32, buf: &mut [u8], max_fds: usize, dontwait: bool, cloexec: bool) -> R<(usize, Vec<i32>, bool)> {
    let r = unix(fd, |u, nb| u.recv(buf, max_fds, nb || dontwait))?;
    let mut out = Vec::with_capacity(r.fds.len());
    for d in r.fds {
        match fd::insert(d, 0, cloexec) {
            Ok(n) => out.push(n),
            Err(_) => return Ok((r.len, out, true)),
        }
    }
    Ok((r.len, out, r.truncated))
}

pub fn unix_shutdown_write(fd: i32) -> R<()> {
    unix(fd, |u, _| u.shutdown_write())
}

/// (vlastni jmeno, jmeno druhe strany); druha strana `Err(ENOTCONN)`.
pub fn unix_name(fd: i32, peer: bool) -> R<Option<Vec<u8>>> {
    unix(fd, |u, _| if peer { u.peer_path() } else { Ok(u.local_path()) }).map(|p| p.map(String::into_bytes))
}

/// Prihlaseni druhe strany (`SO_PEERCRED`): (pid, uid, gid).
pub fn unix_peer(fd: i32) -> R<(i32, u32, u32)> {
    let p = unix(fd, |u, _| u.peer())?;
    let c = |v: u64| u32::try_from(v).unwrap_or(u32::MAX);
    Ok((p.pid as i32, c(p.user), c(p.group)))
}

/// Cislo uzivatele relace (`getuid`) a skupiny (`getgroups`).
pub fn user_id() -> u32 {
    u32::try_from(rt::fs::user_id()).unwrap_or(u32::MAX)
}

pub fn group_ids() -> Vec<u32> {
    rt::fs::groups().into_iter().map(|g| u32::try_from(g).unwrap_or(u32::MAX)).collect()
}
