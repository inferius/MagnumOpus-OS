//! `poll.h`, `sys/select.h` a `sys/epoll.h` nad `sys::poll` (signaly
//! kanalu a udalosti pripravenosti socketu, osrt::poll).
//!
//! `epoll` je mnozina v tabulce fd (objekt knihovny); `epoll_wait` je
//! `poll` nad jejimi polozkami. Urovnovy rezim presne; `EPOLLET` hlasi jen
//! nove nastavene bity (zmenu proti minule ohlasenym), `EPOLLONESHOT`
//! polozku po ohlaseni vypne do `EPOLL_CTL_MOD`.

use core::time::Duration;
use std::sync::Mutex;

use crate::sys::{self, PollItem};
use crate::{c_int, errno};

pub const POLLIN: i16 = 0x001;
pub const POLLPRI: i16 = 0x002;
pub const POLLOUT: i16 = 0x004;
pub const POLLERR: i16 = 0x008;
pub const POLLHUP: i16 = 0x010;
pub const POLLNVAL: i16 = 0x020;

/// `struct pollfd`.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct pollfd {
    pub fd: c_int,
    pub events: i16,
    pub revents: i16,
}

fn timeout_ms(ms: c_int) -> Option<Duration> {
    (ms >= 0).then(|| Duration::from_millis(ms as u64))
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn poll(fds: *mut pollfd, n: u64, timeout: c_int) -> c_int {
    crate::pthread::testcancel();
    if n > 0 && fds.is_null() {
        return errno::ret(Err(errno::EFAULT));
    }
    if n > 4096 {
        return errno::ret(Err(errno::EINVAL));
    }
    // SAFETY: `fds` ma `n` polozek (kontrakt C).
    let v = if n == 0 { &mut [][..] } else { unsafe { core::slice::from_raw_parts_mut(fds, n as usize) } };
    // Zaporne fd se ignoruje (POSIX), revents = 0.
    let mut items: Vec<PollItem> = v
        .iter()
        .filter(|p| p.fd >= 0)
        .map(|p| PollItem { fd: p.fd, events: p.events as u16, revents: 0 })
        .collect();
    match sys::poll(&mut items, timeout_ms(timeout)) {
        Ok(count) => {
            let mut it = items.iter();
            for p in v.iter_mut() {
                p.revents = if p.fd >= 0 { it.next().map_or(0, |x| x.revents as i16) } else { 0 };
            }
            count as c_int
        }
        Err(e) => errno::ret(Err(e)),
    }
}

// --- select ----------------------------------------------------------------

pub const FD_SETSIZE: usize = 1024;

/// `fd_set` (1024 bitu).
#[repr(C)]
pub struct fd_set {
    pub bits: [u64; FD_SETSIZE / 64],
}

fn has(s: *const fd_set, fd: usize) -> bool {
    // SAFETY: kontrakt C (NULL = prazdna mnozina).
    unsafe { s.as_ref() }.is_some_and(|s| s.bits[fd / 64] & (1 << (fd % 64)) != 0)
}

/// `struct timeval`.
#[repr(C)]
pub struct timeval {
    pub tv_sec: i64,
    pub tv_usec: i64,
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn select(
    nfds: c_int,
    r: *mut fd_set,
    w: *mut fd_set,
    e: *mut fd_set,
    tv: *mut timeval,
) -> c_int {
    crate::pthread::testcancel();
    if nfds < 0 || nfds as usize > FD_SETSIZE {
        return errno::ret(Err(errno::EINVAL));
    }
    let mut items = Vec::new();
    for fd in 0..nfds as usize {
        let mut ev = 0u16;
        if has(r, fd) {
            ev |= POLLIN as u16;
        }
        if has(w, fd) {
            ev |= POLLOUT as u16;
        }
        if has(e, fd) {
            ev |= POLLPRI as u16;
        }
        if ev != 0 {
            items.push(PollItem { fd: fd as i32, events: ev, revents: 0 });
        }
    }
    // SAFETY: kontrakt C.
    let timeout = unsafe { tv.as_ref() }
        .map(|t| Duration::from_secs(t.tv_sec.max(0) as u64) + Duration::from_micros(t.tv_usec.max(0) as u64));
    if let Err(err) = sys::poll(&mut items, timeout) {
        return errno::ret(Err(err));
    }
    if items.iter().any(|p| p.revents & POLLNVAL as u16 != 0) {
        return errno::ret(Err(errno::EBADF));
    }
    // Vysledek: mnoziny prepsat jen na pripravena fd.
    for s in [r, w, e] {
        // SAFETY: kontrakt C.
        if let Some(s) = unsafe { s.as_mut() } {
            s.bits = [0; FD_SETSIZE / 64];
        }
    }
    let mut count = 0;
    let set = |s: *mut fd_set, fd: usize| {
        // SAFETY: kontrakt C.
        if let Some(s) = unsafe { s.as_mut() } {
            s.bits[fd / 64] |= 1 << (fd % 64);
        }
    };
    let rd = (POLLIN | POLLHUP | POLLERR) as u16;
    for p in &items {
        let fd = p.fd as usize;
        if p.events & POLLIN as u16 != 0 && p.revents & rd != 0 {
            set(r, fd);
            count += 1;
        }
        if p.events & POLLOUT as u16 != 0 && p.revents & (POLLOUT | POLLERR) as u16 != 0 {
            set(w, fd);
            count += 1;
        }
        if p.events & POLLPRI as u16 != 0 && p.revents & POLLPRI as u16 != 0 {
            set(e, fd);
            count += 1;
        }
    }
    count
}

// --- epoll -----------------------------------------------------------------

pub const EPOLLIN: u32 = 0x001;
pub const EPOLLPRI: u32 = 0x002;
pub const EPOLLOUT: u32 = 0x004;
pub const EPOLLERR: u32 = 0x008;
pub const EPOLLHUP: u32 = 0x010;
pub const EPOLLRDHUP: u32 = 0x2000;
pub const EPOLLONESHOT: u32 = 1 << 30;
pub const EPOLLET: u32 = 1 << 31;
pub const EPOLL_CTL_ADD: c_int = 1;
pub const EPOLL_CTL_DEL: c_int = 2;
pub const EPOLL_CTL_MOD: c_int = 3;
pub const EPOLL_CLOEXEC: c_int = 0o2000000;

/// `struct epoll_event`: na x86_64 zabaleny (12 B) jako Linux.
#[cfg_attr(target_arch = "x86_64", repr(C, packed))]
#[cfg_attr(not(target_arch = "x86_64"), repr(C))]
#[derive(Clone, Copy)]
pub struct epoll_event {
    pub events: u32,
    pub data: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Watch {
    pub fd: i32,
    pub events: u32,
    pub data: u64,
    /// Naposledy ohlasene bity (EPOLLET).
    pub last: u32,
    /// Vypnuto po ohlaseni (EPOLLONESHOT).
    pub off: bool,
}

/// Mnozina `epoll` (objekt v tabulce fd).
#[derive(Default)]
pub struct Epoll {
    pub watches: Mutex<Vec<Watch>>,
}

impl Epoll {
    pub fn ctl(&self, op: c_int, fd: i32, ev: Option<(u32, u64)>) -> Result<(), c_int> {
        let mut w = self.watches.lock().unwrap_or_else(|e| e.into_inner());
        let pos = w.iter().position(|x| x.fd == fd);
        match (op, pos, ev) {
            (EPOLL_CTL_ADD, None, Some((events, data))) => {
                w.push(Watch { fd, events, data, last: 0, off: false });
                Ok(())
            }
            (EPOLL_CTL_ADD, Some(_), _) => Err(errno::EEXIST),
            (EPOLL_CTL_MOD, Some(i), Some((events, data))) => {
                w[i] = Watch { fd, events, data, last: 0, off: false };
                Ok(())
            }
            (EPOLL_CTL_DEL, Some(i), _) => {
                w.remove(i);
                Ok(())
            }
            (EPOLL_CTL_MOD | EPOLL_CTL_DEL, None, _) => Err(errno::ENOENT),
            _ => Err(errno::EINVAL),
        }
    }

    /// Vybere ohlasitelne udalosti z vysledku `poll` (`ready[i]` patri
    /// polozce `i`) a upravi stav ET/ONESHOT. Vraci (data, udalosti).
    pub fn collect(&self, ready: &[(i32, u16)], max: usize) -> Vec<(u64, u32)> {
        let mut w = self.watches.lock().unwrap_or_else(|e| e.into_inner());
        let mut out = Vec::new();
        for x in w.iter_mut() {
            if out.len() >= max {
                break;
            }
            let Some(&(_, rev)) = ready.iter().find(|r| r.0 == x.fd) else { continue };
            if x.off {
                continue;
            }
            let mut bits = u32::from(rev) & (x.events | EPOLLERR | EPOLLHUP);
            if x.events & EPOLLRDHUP != 0 && rev & POLLHUP as u16 != 0 {
                bits |= EPOLLRDHUP;
            }
            if x.events & EPOLLET != 0 {
                let new = bits & !x.last;
                x.last = bits;
                bits = new;
            }
            if bits == 0 {
                continue;
            }
            if x.events & EPOLLONESHOT != 0 {
                x.off = true;
            }
            out.push((x.data, bits));
        }
        out
    }

    fn items(&self) -> Vec<PollItem> {
        let w = self.watches.lock().unwrap_or_else(|e| e.into_inner());
        w.iter()
            .filter(|x| !x.off)
            .map(|x| PollItem { fd: x.fd, events: (x.events & 0xffff) as u16, revents: 0 })
            .collect()
    }
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub extern "C" fn epoll_create1(flags: c_int) -> c_int {
    if flags & !EPOLL_CLOEXEC != 0 {
        return errno::ret(Err(errno::EINVAL));
    }
    errno::ret_val(sys::insert_user(Box::new(Epoll::default()), flags & EPOLL_CLOEXEC != 0)) as c_int
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub extern "C" fn epoll_create(size: c_int) -> c_int {
    if size <= 0 {
        return errno::ret(Err(errno::EINVAL));
    }
    epoll_create1(0)
}

fn with_epoll<T>(epfd: c_int, f: impl FnOnce(&Epoll) -> Result<T, c_int>) -> Result<T, c_int> {
    sys::with_user(epfd, |o| match o.downcast_ref::<Epoll>() {
        Some(e) => f(e),
        None => Err(errno::EINVAL),
    })
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn epoll_ctl(epfd: c_int, op: c_int, fd: c_int, ev: *mut epoll_event) -> c_int {
    if fd == epfd {
        return errno::ret(Err(errno::EINVAL));
    }
    // Cil musi byt platne fd.
    if let Err(e) = sys::status(fd) {
        return errno::ret(Err(e));
    }
    // SAFETY: kontrakt C (NULL jen pro DEL).
    let e = unsafe { ev.as_ref() }.map(|x| {
        let x = *x;
        (x.events, x.data)
    });
    if op != EPOLL_CTL_DEL && e.is_none() {
        return errno::ret(Err(errno::EFAULT));
    }
    errno::ret(with_epoll(epfd, |s| s.ctl(op, fd, e)))
}

/// Krok cekani, kdyz jsou pripravene jen uz ohlasene polozky ET.
const ET_RETRY: Duration = Duration::from_millis(1);
/// Krok cekani prazdne mnoziny.
const EMPTY_RETRY: Duration = Duration::from_millis(50);

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn epoll_wait(epfd: c_int, out: *mut epoll_event, max: c_int, timeout: c_int) -> c_int {
    crate::pthread::testcancel();
    if max <= 0 || out.is_null() {
        return errno::ret(Err(errno::EINVAL));
    }
    let deadline = timeout_ms(timeout).map(|t| sys::monotonic_ns() + t.as_nanos() as u64);
    loop {
        let items = match with_epoll(epfd, |s| Ok(s.items())) {
            Ok(i) => i,
            Err(e) => return errno::ret(Err(e)),
        };
        let mut items = items;
        let left = deadline.map(|d| Duration::from_nanos(d.saturating_sub(sys::monotonic_ns())));
        if items.is_empty() {
            // Prazdna mnozina: polozky muze pridat jine vlakno, cekat po kusech.
            if deadline.is_some_and(|d| sys::monotonic_ns() >= d) {
                return 0;
            }
            std::thread::sleep(left.map_or(EMPTY_RETRY, |l| l.min(EMPTY_RETRY)));
            continue;
        }
        if let Err(e) = sys::poll(&mut items, left) {
            return errno::ret(Err(e));
        }
        let ready: Vec<(i32, u16)> = items.iter().filter(|p| p.revents != 0).map(|p| (p.fd, p.revents)).collect();
        let got = match with_epoll(epfd, |s| Ok(s.collect(&ready, max as usize))) {
            Ok(g) => g,
            Err(e) => return errno::ret(Err(e)),
        };
        if !got.is_empty() {
            for (i, (data, events)) in got.iter().enumerate() {
                // SAFETY: `out` ma `max` polozek.
                unsafe { out.add(i).write_unaligned(epoll_event { events: *events, data: *data }) };
            }
            return got.len() as c_int;
        }
        if deadline.is_some_and(|d| sys::monotonic_ns() >= d) {
            return 0;
        }
        if !ready.is_empty() {
            // Pripravene jsou jen polozky ET bez zmeny: kratce pockat.
            std::thread::sleep(ET_RETRY);
        }
    }
}
