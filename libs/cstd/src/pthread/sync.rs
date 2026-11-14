//! Synchronizace `pthread` nad futexem: mutex (normalni, rekurzivni,
//! s kontrolou chyb), podminka, rwlock, once, bariera, spinlock.
//!
//! Typy maji vlastni pevne rozlozeni (stejne na obou architekturach,
//! viz `include/pthread.h`); staticky inicializator = same nuly.

use core::time::Duration;
use std::sync::atomic::{AtomicU32, Ordering};

use super::{cancel_pending, testcancel, thread_id};
use crate::stat::timespec;
use crate::sys;
use crate::{c_int, errno};

pub fn wake_all(f: &AtomicU32) {
    sys::futex_wake(f, true);
}

fn wake_one(f: &AtomicU32) {
    sys::futex_wake(f, false);
}

pub const CLOCK_REALTIME: c_int = 0;
pub const CLOCK_MONOTONIC: c_int = 1;

/// Zbyvajici cas do absolutniho terminu `abs` na hodinach `clock`;
/// `None` = termin uz byl, `Err` = neplatny cas.
pub fn until(clock: c_int, abs: &timespec) -> Result<Option<Duration>, c_int> {
    if abs.tv_nsec < 0 || abs.tv_nsec >= 1_000_000_000 {
        return Err(errno::EINVAL);
    }
    let target = i128::from(abs.tv_sec) * 1_000_000_000 + i128::from(abs.tv_nsec);
    let now = if clock == CLOCK_MONOTONIC {
        i128::from(sys::monotonic_ns())
    } else {
        let (s, n) = sys::realtime().unwrap_or((0, 0));
        i128::from(s) * 1_000_000_000 + i128::from(n)
    };
    let left = target - now;
    if left <= 0 { Ok(None) } else { Ok(Some(Duration::from_nanos(left.min(u64::MAX as i128) as u64))) }
}

// --- mutex ---------------------------------------------------------------

pub const PTHREAD_MUTEX_NORMAL: c_int = 0;
pub const PTHREAD_MUTEX_RECURSIVE: c_int = 1;
pub const PTHREAD_MUTEX_ERRORCHECK: c_int = 2;

/// `pthread_mutex_t` (40 B).
#[repr(C)]
pub struct pthread_mutex_t {
    /// 0 volny, 1 zamceny, 2 zamceny s cekajicimi.
    state: AtomicU32,
    kind: c_int,
    owner: AtomicU32,
    count: u32,
    _pad: [u32; 6],
}

/// `pthread_mutexattr_t` (4 B).
#[repr(C)]
pub struct pthread_mutexattr_t {
    pub(crate) kind: c_int,
}

impl pthread_mutex_t {
    fn try_raw(&self) -> bool {
        self.state.compare_exchange(0, 1, Ordering::Acquire, Ordering::Relaxed).is_ok()
    }

    /// Zamkne; `deadline` = `(hodiny, absolutni cas)`.
    fn lock_raw(&self, deadline: Option<(c_int, &timespec)>) -> Result<(), c_int> {
        if self.try_raw() {
            return Ok(());
        }
        loop {
            if self.state.swap(2, Ordering::Acquire) == 0 {
                return Ok(());
            }
            let left = match deadline {
                Some((clock, abs)) => match until(clock, abs)? {
                    Some(d) => Some(d),
                    None => return Err(errno::ETIMEDOUT),
                },
                None => None,
            };
            sys::futex_wait(&self.state, 2, left);
        }
    }

    fn unlock_raw(&self) {
        if self.state.swap(0, Ordering::Release) == 2 {
            wake_one(&self.state);
        }
    }

    fn lock(&mut self, deadline: Option<(c_int, &timespec)>, try_only: bool) -> c_int {
        let me = thread_id();
        if self.kind != PTHREAD_MUTEX_NORMAL && self.owner.load(Ordering::Relaxed) == me {
            if self.kind == PTHREAD_MUTEX_RECURSIVE {
                self.count += 1;
                return 0;
            }
            return errno::EDEADLK;
        }
        if try_only {
            if !self.try_raw() {
                return errno::EBUSY;
            }
        } else if let Err(e) = self.lock_raw(deadline) {
            return e;
        }
        self.owner.store(me, Ordering::Relaxed);
        self.count = 1;
        0
    }

    fn unlock(&mut self) -> c_int {
        if self.kind != PTHREAD_MUTEX_NORMAL {
            if self.owner.load(Ordering::Relaxed) != thread_id() {
                return errno::EPERM;
            }
            if self.count > 1 {
                self.count -= 1;
                return 0;
            }
        }
        self.owner.store(0, Ordering::Relaxed);
        self.count = 0;
        self.unlock_raw();
        0
    }
}

/// # Safety
///
/// Ukazatel z C (kontrakt C: inicializovany objekt).
unsafe fn obj<'a, T>(p: *mut T) -> Option<&'a mut T> {
    // SAFETY: predano volajicim.
    unsafe { p.as_mut() }
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn pthread_mutexattr_init(a: *mut pthread_mutexattr_t) -> c_int {
    // SAFETY: kontrakt C.
    match unsafe { obj(a) } {
        Some(a) => {
            a.kind = PTHREAD_MUTEX_NORMAL;
            0
        }
        None => errno::EINVAL,
    }
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub extern "C" fn pthread_mutexattr_destroy(_a: *mut pthread_mutexattr_t) -> c_int {
    0
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn pthread_mutexattr_settype(a: *mut pthread_mutexattr_t, kind: c_int) -> c_int {
    if !(PTHREAD_MUTEX_NORMAL..=PTHREAD_MUTEX_ERRORCHECK).contains(&kind) {
        return errno::EINVAL;
    }
    // SAFETY: kontrakt C.
    match unsafe { obj(a) } {
        Some(a) => {
            a.kind = kind;
            0
        }
        None => errno::EINVAL,
    }
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn pthread_mutexattr_gettype(a: *const pthread_mutexattr_t, kind: *mut c_int) -> c_int {
    // SAFETY: kontrakt C.
    unsafe { *kind = (*a).kind };
    0
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn pthread_mutex_init(m: *mut pthread_mutex_t, a: *const pthread_mutexattr_t) -> c_int {
    // SAFETY: kontrakt C.
    let kind = unsafe { a.as_ref() }.map_or(PTHREAD_MUTEX_NORMAL, |a| a.kind);
    if m.is_null() {
        return errno::EINVAL;
    }
    // SAFETY: `m` ukazuje na misto pro mutex.
    unsafe {
        m.write(pthread_mutex_t { state: AtomicU32::new(0), kind, owner: AtomicU32::new(0), count: 0, _pad: [0; 6] })
    };
    0
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn pthread_mutex_destroy(m: *mut pthread_mutex_t) -> c_int {
    // SAFETY: kontrakt C.
    match unsafe { obj(m) } {
        Some(m) if m.state.load(Ordering::Relaxed) != 0 => errno::EBUSY,
        Some(_) => 0,
        None => errno::EINVAL,
    }
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn pthread_mutex_lock(m: *mut pthread_mutex_t) -> c_int {
    // SAFETY: kontrakt C.
    unsafe { obj(m) }.map_or(errno::EINVAL, |m| m.lock(None, false))
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn pthread_mutex_trylock(m: *mut pthread_mutex_t) -> c_int {
    // SAFETY: kontrakt C.
    unsafe { obj(m) }.map_or(errno::EINVAL, |m| m.lock(None, true))
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn pthread_mutex_timedlock(m: *mut pthread_mutex_t, abs: *const timespec) -> c_int {
    // SAFETY: kontrakt C.
    match (unsafe { obj(m) }, unsafe { abs.as_ref() }) {
        (Some(m), Some(abs)) => m.lock(Some((CLOCK_REALTIME, abs)), false),
        _ => errno::EINVAL,
    }
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn pthread_mutex_unlock(m: *mut pthread_mutex_t) -> c_int {
    // SAFETY: kontrakt C.
    unsafe { obj(m) }.map_or(errno::EINVAL, |m| m.unlock())
}

// --- podminka ------------------------------------------------------------

/// `pthread_cond_t` (48 B).
#[repr(C)]
pub struct pthread_cond_t {
    seq: AtomicU32,
    clock: c_int,
    _pad: [u32; 10],
}

/// `pthread_condattr_t` (4 B).
#[repr(C)]
pub struct pthread_condattr_t {
    clock: c_int,
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn pthread_condattr_init(a: *mut pthread_condattr_t) -> c_int {
    // SAFETY: kontrakt C.
    unsafe { a.write(pthread_condattr_t { clock: CLOCK_REALTIME }) };
    0
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub extern "C" fn pthread_condattr_destroy(_a: *mut pthread_condattr_t) -> c_int {
    0
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn pthread_condattr_setclock(a: *mut pthread_condattr_t, clock: c_int) -> c_int {
    if clock != CLOCK_REALTIME && clock != CLOCK_MONOTONIC {
        return errno::EINVAL;
    }
    // SAFETY: kontrakt C.
    unsafe { (*a).clock = clock };
    0
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn pthread_condattr_getclock(a: *const pthread_condattr_t, clock: *mut c_int) -> c_int {
    // SAFETY: kontrakt C.
    unsafe { *clock = (*a).clock };
    0
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn pthread_cond_init(c: *mut pthread_cond_t, a: *const pthread_condattr_t) -> c_int {
    // SAFETY: kontrakt C.
    let clock = unsafe { a.as_ref() }.map_or(CLOCK_REALTIME, |a| a.clock);
    // SAFETY: `c` ukazuje na misto pro podminku.
    unsafe { c.write(pthread_cond_t { seq: AtomicU32::new(0), clock, _pad: [0; 10] }) };
    0
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub extern "C" fn pthread_cond_destroy(_c: *mut pthread_cond_t) -> c_int {
    0
}

/// Odlozene zruseni: cekani se deli na kusy, aby vlakno zruseni videlo.
const CANCEL_SLICE: Duration = Duration::from_millis(100);

fn cond_wait(c: &pthread_cond_t, m: &mut pthread_mutex_t, abs: Option<&timespec>) -> c_int {
    let seq = c.seq.load(Ordering::Acquire);
    // Rekurzivni mutex se uvolni cely a pak vrati na stejnou hloubku.
    let depth = m.count;
    let owner = m.owner.load(Ordering::Relaxed);
    m.owner.store(0, Ordering::Relaxed);
    m.count = 0;
    m.unlock_raw();
    let mut result = 0;
    loop {
        let left = match abs {
            Some(abs) => match until(c.clock, abs) {
                Ok(Some(d)) => Some(d),
                Ok(None) => {
                    result = errno::ETIMEDOUT;
                    break;
                }
                Err(e) => {
                    result = e;
                    break;
                }
            },
            None => None,
        };
        let slice = Some(left.map_or(CANCEL_SLICE, |d| d.min(CANCEL_SLICE)));
        let woke = sys::futex_wait(&c.seq, seq, slice);
        if c.seq.load(Ordering::Acquire) != seq || cancel_pending() {
            break;
        }
        // Vyprsel jen kus cekani: znovu (termin hlida `until`).
        let _ = woke;
    }
    // Zpet se zamyka se stavem 2 (mohou cekat dalsi probuzeni).
    loop {
        if m.state.swap(2, Ordering::Acquire) == 0 {
            break;
        }
        sys::futex_wait(&m.state, 2, None);
    }
    m.owner.store(owner, Ordering::Relaxed);
    m.count = depth;
    // Bod zruseni: s drzenym mutexem (POSIX).
    testcancel();
    result
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn pthread_cond_wait(c: *mut pthread_cond_t, m: *mut pthread_mutex_t) -> c_int {
    // SAFETY: kontrakt C.
    match (unsafe { obj(c) }, unsafe { obj(m) }) {
        (Some(c), Some(m)) => cond_wait(c, m, None),
        _ => errno::EINVAL,
    }
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn pthread_cond_timedwait(
    c: *mut pthread_cond_t,
    m: *mut pthread_mutex_t,
    abs: *const timespec,
) -> c_int {
    // SAFETY: kontrakt C.
    match (unsafe { obj(c) }, unsafe { obj(m) }, unsafe { abs.as_ref() }) {
        (Some(c), Some(m), Some(abs)) => cond_wait(c, m, Some(abs)),
        _ => errno::EINVAL,
    }
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn pthread_cond_signal(c: *mut pthread_cond_t) -> c_int {
    // SAFETY: kontrakt C.
    let Some(c) = (unsafe { obj(c) }) else { return errno::EINVAL };
    c.seq.fetch_add(1, Ordering::Release);
    wake_one(&c.seq);
    0
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn pthread_cond_broadcast(c: *mut pthread_cond_t) -> c_int {
    // SAFETY: kontrakt C.
    let Some(c) = (unsafe { obj(c) }) else { return errno::EINVAL };
    c.seq.fetch_add(1, Ordering::Release);
    wake_all(&c.seq);
    0
}

// --- rwlock --------------------------------------------------------------

const WRITER: u32 = 1 << 31;

/// `pthread_rwlock_t` (56 B): pocet ctenaru nebo bit zapisovatele,
/// citac zmen pro cekani.
#[repr(C)]
pub struct pthread_rwlock_t {
    state: AtomicU32,
    seq: AtomicU32,
    writer: AtomicU32,
    _pad: [u32; 11],
}

impl pthread_rwlock_t {
    fn try_read(&self) -> bool {
        let s = self.state.load(Ordering::Relaxed);
        s & WRITER == 0 && s < WRITER - 1 && self.state.compare_exchange(s, s + 1, Ordering::Acquire, Ordering::Relaxed).is_ok()
    }

    fn try_write(&self) -> bool {
        self.state.compare_exchange(0, WRITER, Ordering::Acquire, Ordering::Relaxed).is_ok()
    }

    fn wait(&self, try_: impl Fn(&Self) -> bool, abs: Option<&timespec>) -> c_int {
        loop {
            let seq = self.seq.load(Ordering::Acquire);
            if try_(self) {
                return 0;
            }
            let left = match abs {
                Some(abs) => match until(CLOCK_REALTIME, abs) {
                    Ok(Some(d)) => Some(d),
                    Ok(None) => return errno::ETIMEDOUT,
                    Err(e) => return e,
                },
                None => None,
            };
            sys::futex_wait(&self.seq, seq, left);
        }
    }

    fn unlock(&self) -> c_int {
        let s = self.state.load(Ordering::Relaxed);
        if s == WRITER {
            if self.writer.load(Ordering::Relaxed) != thread_id() {
                return errno::EPERM;
            }
            self.writer.store(0, Ordering::Relaxed);
            self.state.store(0, Ordering::Release);
        } else if s == 0 {
            return errno::EPERM;
        } else {
            self.state.fetch_sub(1, Ordering::Release);
        }
        self.seq.fetch_add(1, Ordering::Release);
        wake_all(&self.seq);
        0
    }
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn pthread_rwlock_init(l: *mut pthread_rwlock_t, _a: *const c_int) -> c_int {
    // SAFETY: kontrakt C.
    unsafe {
        l.write(pthread_rwlock_t { state: AtomicU32::new(0), seq: AtomicU32::new(0), writer: AtomicU32::new(0), _pad: [0; 11] })
    };
    0
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub extern "C" fn pthread_rwlock_destroy(_l: *mut pthread_rwlock_t) -> c_int {
    0
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn pthread_rwlock_rdlock(l: *mut pthread_rwlock_t) -> c_int {
    // SAFETY: kontrakt C.
    unsafe { obj(l) }.map_or(errno::EINVAL, |l| l.wait(pthread_rwlock_t::try_read, None))
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn pthread_rwlock_tryrdlock(l: *mut pthread_rwlock_t) -> c_int {
    // SAFETY: kontrakt C.
    unsafe { obj(l) }.map_or(errno::EINVAL, |l| if l.try_read() { 0 } else { errno::EBUSY })
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn pthread_rwlock_timedrdlock(l: *mut pthread_rwlock_t, abs: *const timespec) -> c_int {
    // SAFETY: kontrakt C.
    match (unsafe { obj(l) }, unsafe { abs.as_ref() }) {
        (Some(l), Some(abs)) => l.wait(pthread_rwlock_t::try_read, Some(abs)),
        _ => errno::EINVAL,
    }
}

fn write_locked(l: &pthread_rwlock_t, r: c_int) -> c_int {
    if r == 0 {
        l.writer.store(thread_id(), Ordering::Relaxed);
    }
    r
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn pthread_rwlock_wrlock(l: *mut pthread_rwlock_t) -> c_int {
    // SAFETY: kontrakt C.
    let Some(l) = (unsafe { obj(l) }) else { return errno::EINVAL };
    if l.state.load(Ordering::Relaxed) == WRITER && l.writer.load(Ordering::Relaxed) == thread_id() {
        return errno::EDEADLK;
    }
    write_locked(l, l.wait(pthread_rwlock_t::try_write, None))
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn pthread_rwlock_trywrlock(l: *mut pthread_rwlock_t) -> c_int {
    // SAFETY: kontrakt C.
    let Some(l) = (unsafe { obj(l) }) else { return errno::EINVAL };
    write_locked(l, if l.try_write() { 0 } else { errno::EBUSY })
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn pthread_rwlock_timedwrlock(l: *mut pthread_rwlock_t, abs: *const timespec) -> c_int {
    // SAFETY: kontrakt C.
    match (unsafe { obj(l) }, unsafe { abs.as_ref() }) {
        (Some(l), Some(abs)) => write_locked(l, l.wait(pthread_rwlock_t::try_write, Some(abs))),
        _ => errno::EINVAL,
    }
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn pthread_rwlock_unlock(l: *mut pthread_rwlock_t) -> c_int {
    // SAFETY: kontrakt C.
    unsafe { obj(l) }.map_or(errno::EINVAL, |l| l.unlock())
}

// --- once, bariera, spinlock ---------------------------------------------

/// `pthread_once_t`: 0 = jeste ne, 1 = bezi, 2 = hotovo.
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn pthread_once(once: *mut AtomicU32, f: unsafe extern "C" fn()) -> c_int {
    // SAFETY: kontrakt C (int zarovnany na 4 B).
    let Some(o) = (unsafe { once.as_ref() }) else { return errno::EINVAL };
    loop {
        match o.compare_exchange(0, 1, Ordering::Acquire, Ordering::Acquire) {
            Ok(_) => {
                // SAFETY: funkce od volajiciho.
                unsafe { f() };
                o.store(2, Ordering::Release);
                wake_all(o);
                return 0;
            }
            Err(2) => return 0,
            Err(_) => {
                sys::futex_wait(o, 1, None);
            }
        }
    }
}

pub const PTHREAD_BARRIER_SERIAL_THREAD: c_int = -1;

/// `pthread_barrier_t` (32 B).
#[repr(C)]
pub struct pthread_barrier_t {
    count: u32,
    waiting: AtomicU32,
    seq: AtomicU32,
    _pad: [u32; 5],
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn pthread_barrier_init(b: *mut pthread_barrier_t, _a: *const c_int, count: u32) -> c_int {
    if count == 0 || b.is_null() {
        return errno::EINVAL;
    }
    // SAFETY: kontrakt C.
    unsafe {
        b.write(pthread_barrier_t { count, waiting: AtomicU32::new(0), seq: AtomicU32::new(0), _pad: [0; 5] })
    };
    0
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub extern "C" fn pthread_barrier_destroy(_b: *mut pthread_barrier_t) -> c_int {
    0
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn pthread_barrier_wait(b: *mut pthread_barrier_t) -> c_int {
    // SAFETY: kontrakt C.
    let Some(b) = (unsafe { obj(b) }) else { return errno::EINVAL };
    let seq = b.seq.load(Ordering::Acquire);
    if b.waiting.fetch_add(1, Ordering::AcqRel) + 1 == b.count {
        b.waiting.store(0, Ordering::Relaxed);
        b.seq.fetch_add(1, Ordering::Release);
        wake_all(&b.seq);
        return PTHREAD_BARRIER_SERIAL_THREAD;
    }
    while b.seq.load(Ordering::Acquire) == seq {
        sys::futex_wait(&b.seq, seq, None);
    }
    0
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn pthread_spin_init(s: *mut AtomicU32, _shared: c_int) -> c_int {
    // SAFETY: kontrakt C.
    unsafe { s.write(AtomicU32::new(0)) };
    0
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub extern "C" fn pthread_spin_destroy(_s: *mut AtomicU32) -> c_int {
    0
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn pthread_spin_lock(s: *mut AtomicU32) -> c_int {
    // SAFETY: kontrakt C.
    let Some(s) = (unsafe { s.as_ref() }) else { return errno::EINVAL };
    let mut spins = 0u32;
    while s.compare_exchange_weak(0, 1, Ordering::Acquire, Ordering::Relaxed).is_err() {
        spins += 1;
        if spins % 64 == 0 {
            std::thread::yield_now();
        } else {
            core::hint::spin_loop();
        }
    }
    0
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn pthread_spin_trylock(s: *mut AtomicU32) -> c_int {
    // SAFETY: kontrakt C.
    let Some(s) = (unsafe { s.as_ref() }) else { return errno::EINVAL };
    if s.compare_exchange(0, 1, Ordering::Acquire, Ordering::Relaxed).is_ok() { 0 } else { errno::EBUSY }
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn pthread_spin_unlock(s: *mut AtomicU32) -> c_int {
    // SAFETY: kontrakt C.
    let Some(s) = (unsafe { s.as_ref() }) else { return errno::EINVAL };
    s.store(0, Ordering::Release);
    0
}
