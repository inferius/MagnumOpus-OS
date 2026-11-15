//! `semaphore.h`: nepojmenovane semafory nad futexem (`sem_init`,
//! `sem_wait`, `sem_post` ...). `sem_t` ma 32 B jako Linux na 64 bitech.
//!
//! `pshared` = 1 se prijme: semafor funguje mezi vlakny procesu; mezi
//! procesy ho sdilet nejde (system nema sdilenou pamet mezi procesy,
//! futex je v adresovem prostoru procesu). Pojmenovane semafory
//! (`sem_open`) proto vraci `SEM_FAILED` s `ENOSYS`.
//!
//! Funkce vraci 0, nebo -1 s `errno` (ne kod chyby jako `pthread_*`).
//! `sem_wait`, `sem_timedwait` a `sem_clockwait` jsou body zruseni vlakna.

use core::time::Duration;
use std::sync::atomic::{AtomicU32, Ordering};

use crate::pthread::{testcancel, until};
use crate::stat::timespec;
use crate::sys;
use crate::{c_char, c_int, errno};

/// Nejvetsi hodnota (`SEM_VALUE_MAX`, jako glibc).
pub const SEM_VALUE_MAX: u32 = i32::MAX as u32;

/// `sem_t`: hodnota, pocet cekajicich, zbytek do 32 B.
#[repr(C, align(8))]
pub struct sem_t {
    value: AtomicU32,
    waiters: AtomicU32,
    _rest: [u8; 24],
}

impl sem_t {
    fn try_take(&self) -> bool {
        let mut v = self.value.load(Ordering::Relaxed);
        while v > 0 {
            match self.value.compare_exchange_weak(v, v - 1, Ordering::Acquire, Ordering::Relaxed) {
                Ok(_) => return true,
                Err(x) => v = x,
            }
        }
        false
    }

    /// Ceka na hodnotu > 0. `deadline` vraci zbyvajici cas (`None` =
    /// navzdy); `Some(None)` = termin uplynul.
    fn wait(&self, deadline: impl Fn() -> Result<Option<Option<Duration>>, c_int>) -> Result<(), c_int> {
        loop {
            if self.try_take() {
                return Ok(());
            }
            let timeout = match deadline()? {
                None => None,
                Some(None) => return Err(errno::ETIMEDOUT),
                Some(t) => t,
            };
            self.waiters.fetch_add(1, Ordering::SeqCst);
            // Hodnota 0 se mohla mezitim zvysit: futex pak nespi.
            sys::futex_wait(&self.value, 0, timeout);
            self.waiters.fetch_sub(1, Ordering::SeqCst);
            testcancel();
        }
    }

    fn post(&self) -> Result<(), c_int> {
        let mut v = self.value.load(Ordering::Relaxed);
        loop {
            if v >= SEM_VALUE_MAX {
                return Err(errno::EOVERFLOW);
            }
            match self.value.compare_exchange_weak(v, v + 1, Ordering::Release, Ordering::Relaxed) {
                Ok(_) => break,
                Err(x) => v = x,
            }
        }
        if self.waiters.load(Ordering::SeqCst) > 0 {
            sys::futex_wake(&self.value, false);
        }
        Ok(())
    }
}

/// # Safety
///
/// `s` je NULL nebo platny ukazatel na `sem_t`.
unsafe fn sem<'a>(s: *mut sem_t) -> Result<&'a sem_t, c_int> {
    // SAFETY: kontrakt C.
    unsafe { s.as_ref() }.ok_or(errno::EINVAL)
}

/// # Safety
///
/// `s` ukazuje na pamet pro `sem_t`.
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn sem_init(s: *mut sem_t, _pshared: c_int, value: u32) -> c_int {
    if s.is_null() || value > SEM_VALUE_MAX {
        return errno::ret(Err(errno::EINVAL));
    }
    // SAFETY: kontrakt C.
    unsafe { s.write(sem_t { value: AtomicU32::new(value), waiters: AtomicU32::new(0), _rest: [0; 24] }) };
    0
}

/// # Safety
///
/// `s` je inicializovany semafor, na ktery nikdo neceka.
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn sem_destroy(s: *mut sem_t) -> c_int {
    // SAFETY: kontrakt C.
    errno::ret(unsafe { sem(s) }.map(|_| ()))
}

/// # Safety
///
/// `s` je inicializovany semafor.
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn sem_post(s: *mut sem_t) -> c_int {
    // SAFETY: kontrakt C.
    errno::ret(unsafe { sem(s) }.and_then(sem_t::post))
}

/// # Safety
///
/// `s` je inicializovany semafor.
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn sem_wait(s: *mut sem_t) -> c_int {
    testcancel();
    // SAFETY: kontrakt C.
    errno::ret(unsafe { sem(s) }.and_then(|s| s.wait(|| Ok(None))))
}

/// # Safety
///
/// `s` je inicializovany semafor.
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn sem_trywait(s: *mut sem_t) -> c_int {
    // SAFETY: kontrakt C.
    errno::ret(unsafe { sem(s) }.and_then(|s| if s.try_take() { Ok(()) } else { Err(errno::EAGAIN) }))
}

/// Cekani s absolutnim terminem na hodinach `clock`.
///
/// # Safety
///
/// `s` je inicializovany semafor, `abs` NULL nebo platny ukazatel.
unsafe fn timed(s: *mut sem_t, clock: c_int, abs: *const timespec) -> c_int {
    testcancel();
    // SAFETY: kontrakt C.
    let r = unsafe { sem(s) }.and_then(|s| {
        // Volny semafor se vezme i s neplatnym terminem (POSIX).
        if s.try_take() {
            return Ok(());
        }
        // SAFETY: kontrakt C.
        let abs = unsafe { abs.as_ref() }.ok_or(errno::EINVAL)?;
        s.wait(|| until(clock, abs).map(Some))
    });
    errno::ret(r)
}

/// # Safety
///
/// `s` je inicializovany semafor, `abs` platny ukazatel (`CLOCK_REALTIME`).
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn sem_timedwait(s: *mut sem_t, abs: *const timespec) -> c_int {
    // SAFETY: kontrakt C.
    unsafe { timed(s, crate::pthread::CLOCK_REALTIME, abs) }
}

/// `sem_clockwait` (glibc 2.30, POSIX 2024): termin na `CLOCK_REALTIME`
/// nebo `CLOCK_MONOTONIC`.
///
/// # Safety
///
/// `s` je inicializovany semafor, `abs` platny ukazatel.
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn sem_clockwait(s: *mut sem_t, clock: c_int, abs: *const timespec) -> c_int {
    if clock != crate::pthread::CLOCK_REALTIME && clock != crate::pthread::CLOCK_MONOTONIC {
        return errno::ret(Err(errno::EINVAL));
    }
    // SAFETY: kontrakt C.
    unsafe { timed(s, clock, abs) }
}

/// # Safety
///
/// `s` je inicializovany semafor, `out` platny ukazatel.
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn sem_getvalue(s: *mut sem_t, out: *mut c_int) -> c_int {
    // SAFETY: kontrakt C.
    let r = unsafe { sem(s) }.and_then(|s| {
        // SAFETY: kontrakt C.
        let o = unsafe { out.as_mut() }.ok_or(errno::EINVAL)?;
        *o = s.value.load(Ordering::Relaxed) as c_int;
        Ok(())
    });
    errno::ret(r)
}

/// Pojmenovane semafory: bez sdilene pameti mezi procesy nejdou
/// (`SEM_FAILED` = NULL, `ENOSYS`).
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn sem_open(_name: *const c_char, _flags: c_int, _args: ...) -> *mut sem_t {
    errno::set(errno::ENOSYS);
    core::ptr::null_mut()
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub extern "C" fn sem_close(_s: *mut sem_t) -> c_int {
    errno::ret(Err(errno::ENOSYS))
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub extern "C" fn sem_unlink(_name: *const c_char) -> c_int {
    errno::ret(Err(errno::ENOSYS))
}
