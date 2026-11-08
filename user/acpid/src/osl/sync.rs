//! Zamky a semafory pro ACPICA.
//!
//! - spinlock (`AcpiOsCreateLock`): kratke kriticke sekce (seznam GPE).
//!   V userspace se misto toceni s vypnutymi prerusenimi prenechava CPU.
//! - semafor (`AcpiOsCreateSemaphore`): pocitadlo pod zamkem a event jadra,
//!   ktery je signalizovany, dokud je pocitadlo nenulove. Cekajici se po
//!   probuzeni znovu podiva na pocitadlo (vic cekajicich = jeden vyhraje,
//!   ostatni cekaji dal). ACPICA z binarnich semaforu sklada i sve mutexy
//!   (`ACPI_MUTEX_TYPE = ACPI_BINARY_SEMAPHORE`).

use alloc::boxed::Box;
use core::ffi::c_void;
use core::sync::atomic::{AtomicBool, Ordering};
use core::time::Duration;

use libos::{Event, Signals, sys, time, wait_one};
use spin::Mutex;

use super::{AE_BAD_PARAMETER, AE_NO_MEMORY, AE_OK, AE_TIME, Status};

#[unsafe(no_mangle)]
pub extern "C" fn AcpiOsCreateLock(out: *mut *mut c_void) -> Status {
    if out.is_null() {
        return AE_BAD_PARAMETER;
    }
    let lock = Box::into_raw(Box::new(AtomicBool::new(false)));
    // SAFETY: ukazatel od ACPICA.
    unsafe { *out = lock as *mut c_void };
    AE_OK
}

#[unsafe(no_mangle)]
pub extern "C" fn AcpiOsDeleteLock(h: *mut c_void) {
    if !h.is_null() {
        // SAFETY: vznikl v AcpiOsCreateLock.
        drop(unsafe { Box::from_raw(h as *mut AtomicBool) });
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn AcpiOsAcquireLock(h: *mut c_void) -> u64 {
    // SAFETY: vznikl v AcpiOsCreateLock a zije do AcpiOsDeleteLock.
    let lock = unsafe { &*(h as *const AtomicBool) };
    let mut spins = 0u32;
    while lock.compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed).is_err() {
        spins += 1;
        if spins > 64 {
            sys::thread_yield();
        } else {
            core::hint::spin_loop();
        }
    }
    0
}

#[unsafe(no_mangle)]
pub extern "C" fn AcpiOsReleaseLock(h: *mut c_void, _flags: u64) {
    // SAFETY: dtto.
    unsafe { &*(h as *const AtomicBool) }.store(false, Ordering::Release);
}

struct Semaphore {
    count: Mutex<u32>,
    max: u32,
    /// Signalizovany, dokud `count > 0`.
    ready: Event,
}

#[unsafe(no_mangle)]
pub extern "C" fn AcpiOsCreateSemaphore(max: u32, initial: u32, out: *mut *mut c_void) -> Status {
    if out.is_null() || initial > max {
        return AE_BAD_PARAMETER;
    }
    let Ok(ready) = Event::create() else {
        return AE_NO_MEMORY;
    };
    if initial > 0 && ready.signal().is_err() {
        return AE_NO_MEMORY;
    }
    let s = Box::into_raw(Box::new(Semaphore { count: Mutex::new(initial), max, ready }));
    // SAFETY: ukazatel od ACPICA.
    unsafe { *out = s as *mut c_void };
    AE_OK
}

#[unsafe(no_mangle)]
pub extern "C" fn AcpiOsDeleteSemaphore(h: *mut c_void) -> Status {
    if h.is_null() {
        return AE_BAD_PARAMETER;
    }
    // SAFETY: vznikl v AcpiOsCreateSemaphore.
    drop(unsafe { Box::from_raw(h as *mut Semaphore) });
    AE_OK
}

/// `timeout` v ms: 0 = nečekat, 0xFFFF = neomezene.
#[unsafe(no_mangle)]
pub extern "C" fn AcpiOsWaitSemaphore(h: *mut c_void, units: u32, timeout: u16) -> Status {
    if h.is_null() || units == 0 {
        return AE_BAD_PARAMETER;
    }
    // SAFETY: semafor zije do AcpiOsDeleteSemaphore.
    let s = unsafe { &*(h as *const Semaphore) };
    let deadline = match timeout {
        0xFFFF => None,
        ms => Some(time::deadline_after(Duration::from_millis(ms as u64))),
    };
    loop {
        {
            let mut c = s.count.lock();
            if *c >= units {
                *c -= units;
                if *c == 0 {
                    let _ = s.ready.reset();
                }
                return AE_OK;
            }
            if *c > 0 {
                // Je neco, ale malo (vic jednotek najednou): kratke cekani.
                drop(c);
                if timeout == 0 || deadline.is_some_and(|d| time::now() >= d) {
                    return AE_TIME;
                }
                sys::thread_yield();
                continue;
            }
        }
        if timeout == 0 {
            return AE_TIME;
        }
        if wait_one(s.ready.handle(), Signals::SIGNALED, deadline).is_err() {
            return AE_TIME;
        }
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn AcpiOsSignalSemaphore(h: *mut c_void, units: u32) -> Status {
    if h.is_null() {
        return AE_BAD_PARAMETER;
    }
    // SAFETY: dtto.
    let s = unsafe { &*(h as *const Semaphore) };
    let mut c = s.count.lock();
    *c = c.saturating_add(units).min(s.max.max(1));
    let _ = s.ready.signal();
    AE_OK
}
