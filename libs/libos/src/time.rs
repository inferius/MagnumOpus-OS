//! Cas.

use core::time::Duration;

use crate::sys;

/// Nanosekundy od startu systemu (monotonicky).
pub fn now() -> u64 {
    sys::clock_monotonic()
}

/// Deadline (ns od startu) za `d` od ted.
pub fn deadline_after(d: Duration) -> u64 {
    now().saturating_add(d.as_nanos() as u64)
}

/// Uspi vlakno alespon na `d`.
pub fn sleep(d: Duration) {
    let _ = sys::object_wait_many(&mut [], deadline_after(d));
}
