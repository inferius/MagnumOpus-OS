//! Futex pro `std::sync` (`Mutex`, `Condvar`, `RwLock`, `Once`, `park`):
//! std pouziva sve vlastni implementace nad temito funkcemi.

use core::sync::atomic::AtomicU32;
use core::time::Duration;

pub type Futex = AtomicU32;
pub type Primitive = u32;
pub type SmallFutex = AtomicU32;
pub type SmallPrimitive = u32;

/// Usne, dokud je `*futex == expected` a nikdo nezavola wake, nejdele
/// `timeout`. Vraci `false` jen pri vyprseni.
pub fn futex_wait(futex: &AtomicU32, expected: u32, timeout: Option<Duration>) -> bool {
    let deadline = match timeout {
        Some(t) => libos::time::deadline_after(t),
        None => abi::syscall::DEADLINE_INFINITE,
    };
    !matches!(libos::sys::futex_wait(futex, expected, deadline), Err(abi::Error::TimedOut))
}

/// Probudi jedno cekajici vlakno; `true` = nekoho probudil.
pub fn futex_wake(futex: &AtomicU32) -> bool {
    libos::sys::futex_wake(futex, 1) > 0
}

/// Probudi vsechna cekajici vlakna.
pub fn futex_wake_all(futex: &AtomicU32) {
    libos::sys::futex_wake(futex, u32::MAX);
}
