//! Zamek nad futexem jadra (`FUTEX_WAIT`/`FUTEX_WAKE`).
//!
//! Na rozdil od spin zamku cekajici vlakno spi a nebere CPU vlaknu, ktere
//! zamek drzi. Stav (jako futex mutex v `std`): 0 = volno, 1 = zamceno,
//! 2 = zamceno a nekdo ceka. Odemceni vola jadro jen ve stavu 2.

use core::cell::UnsafeCell;
use core::ops::{Deref, DerefMut};
use core::sync::atomic::{AtomicU32, Ordering};

use crate::sys;

const UNLOCKED: u32 = 0;
const LOCKED: u32 = 1;
const CONTENDED: u32 = 2;

/// Pocet otocek, nez vlakno usne (kratke kriticke sekce se casto uvolni
/// driv, nez by se vyplatil syscall).
const SPIN: u32 = 100;

/// Vzajemne vylouceni s daty (API jako `spin::Mutex`: `lock` bez chyby).
pub struct Mutex<T: ?Sized> {
    state: AtomicU32,
    value: UnsafeCell<T>,
}

// SAFETY: k hodnote pristupuje jen drzitel zamku.
unsafe impl<T: ?Sized + Send> Sync for Mutex<T> {}
// SAFETY: viz vyse.
unsafe impl<T: ?Sized + Send> Send for Mutex<T> {}

impl<T> Mutex<T> {
    pub const fn new(value: T) -> Self {
        Self { state: AtomicU32::new(UNLOCKED), value: UnsafeCell::new(value) }
    }

    pub fn into_inner(self) -> T {
        self.value.into_inner()
    }
}

impl<T: ?Sized> Mutex<T> {
    /// Zamkne (pripadne usne, dokud zamek nedrzi nikdo jiny).
    pub fn lock(&self) -> MutexGuard<'_, T> {
        if self
            .state
            .compare_exchange(UNLOCKED, LOCKED, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            self.lock_contended();
        }
        MutexGuard { mutex: self }
    }

    /// Zamkne, jen kdyz je volno.
    pub fn try_lock(&self) -> Option<MutexGuard<'_, T>> {
        self.state
            .compare_exchange(UNLOCKED, LOCKED, Ordering::Acquire, Ordering::Relaxed)
            .ok()
            .map(|_| MutexGuard { mutex: self })
    }

    #[cold]
    fn lock_contended(&self) {
        let mut state = self.spin();
        if state == UNLOCKED {
            match self.state.compare_exchange(UNLOCKED, LOCKED, Ordering::Acquire, Ordering::Relaxed) {
                Ok(_) => return,
                Err(s) => state = s,
            }
        }
        loop {
            // Oznacit "nekdo ceka" a pokud bylo volno, zamek je nas (ve
            // stavu 2 - odemceni pak zbytecne zavola jadro, ale nic se
            // neztrati).
            if state != CONTENDED && self.state.swap(CONTENDED, Ordering::Acquire) == UNLOCKED {
                return;
            }
            let _ = sys::futex_wait(&self.state, CONTENDED, abi::syscall::DEADLINE_INFINITE);
            state = self.spin();
        }
    }

    fn spin(&self) -> u32 {
        let mut n = SPIN;
        loop {
            let state = self.state.load(Ordering::Relaxed);
            if state != LOCKED || n == 0 {
                return state;
            }
            core::hint::spin_loop();
            n -= 1;
        }
    }

    fn unlock(&self) {
        if self.state.swap(UNLOCKED, Ordering::Release) == CONTENDED {
            sys::futex_wake(&self.state, 1);
        }
    }

    pub fn get_mut(&mut self) -> &mut T {
        self.value.get_mut()
    }
}

impl<T: Default> Default for Mutex<T> {
    fn default() -> Self {
        Self::new(T::default())
    }
}

/// Drzeny zamek; uvolni se pri zahozeni.
pub struct MutexGuard<'a, T: ?Sized> {
    mutex: &'a Mutex<T>,
}

impl<T: ?Sized> Deref for MutexGuard<'_, T> {
    type Target = T;
    fn deref(&self) -> &T {
        // SAFETY: zamek drzime, nikdo jiny k hodnote nepristupuje.
        unsafe { &*self.mutex.value.get() }
    }
}

impl<T: ?Sized> DerefMut for MutexGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        // SAFETY: viz `deref`.
        unsafe { &mut *self.mutex.value.get() }
    }
}

impl<T: ?Sized> Drop for MutexGuard<'_, T> {
    fn drop(&mut self) {
        self.mutex.unlock();
    }
}
