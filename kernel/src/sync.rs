//! Synchronizacni primitiva jadra.

use core::cell::UnsafeCell;
use core::ops::{Deref, DerefMut};
use core::sync::atomic::{AtomicBool, Ordering};

use crate::arch::interrupts;

/// Spinlock, ktery po dobu drzeni zakaze preruseni na aktualnim CPU.
///
/// Diky tomu ho muze bezpecne pouzivat i obsluha preruseni (preruseni nemuze
/// prijit na CPU, ktere zamek drzi, takze nevznikne deadlock) a drzitel
/// nemuze byt preempcne vystridan.
pub struct SpinLock<T: ?Sized> {
    locked: AtomicBool,
    value: UnsafeCell<T>,
}

// SAFETY: pristup k hodnote je serializovany zamkem; hodnota se mezi vlakny
// jen predava (T: Send).
unsafe impl<T: ?Sized + Send> Sync for SpinLock<T> {}
// SAFETY: viz vyse.
unsafe impl<T: ?Sized + Send> Send for SpinLock<T> {}

pub struct SpinLockGuard<'a, T: ?Sized> {
    lock: &'a SpinLock<T>,
    irq_was_enabled: bool,
}

impl<T> SpinLock<T> {
    pub const fn new(value: T) -> Self {
        Self {
            locked: AtomicBool::new(false),
            value: UnsafeCell::new(value),
        }
    }
}

impl<T: ?Sized> SpinLock<T> {
    pub fn lock(&self) -> SpinLockGuard<'_, T> {
        let irq_was_enabled = interrupts::disable();
        while self
            .locked
            .compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            while self.locked.load(Ordering::Relaxed) {
                core::hint::spin_loop();
            }
        }
        SpinLockGuard {
            lock: self,
            irq_was_enabled,
        }
    }

    /// Pristup bez zamku, kdyz mame vyhradni referenci (napr. v Drop).
    pub fn get_mut(&mut self) -> &mut T {
        self.value.get_mut()
    }

    pub fn try_lock(&self) -> Option<SpinLockGuard<'_, T>> {
        let irq_was_enabled = interrupts::disable();
        if self
            .locked
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_ok()
        {
            Some(SpinLockGuard {
                lock: self,
                irq_was_enabled,
            })
        } else {
            interrupts::restore(irq_was_enabled);
            None
        }
    }
}

impl<T: ?Sized> Deref for SpinLockGuard<'_, T> {
    type Target = T;
    fn deref(&self) -> &T {
        // SAFETY: drzime zamek, takze mame vyhradni pristup.
        unsafe { &*self.lock.value.get() }
    }
}

impl<T: ?Sized> DerefMut for SpinLockGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        // SAFETY: drzime zamek, takze mame vyhradni pristup.
        unsafe { &mut *self.lock.value.get() }
    }
}

impl<T: ?Sized> Drop for SpinLockGuard<'_, T> {
    fn drop(&mut self) {
        self.lock.locked.store(false, Ordering::Release);
        interrupts::restore(self.irq_was_enabled);
    }
}
