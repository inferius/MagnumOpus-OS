//! Cas pro `std::time`.

/// Monotonni cas (ns od startu systemu) pro `Instant`.
pub fn monotonic_ns() -> u64 {
    libos::time::now()
}

/// Realny cas: (sekundy od 1970 UTC, nanosekundy) pro `SystemTime`;
/// `None`, kdyz ho jadro nezna.
pub fn realtime() -> Option<(i64, u32)> {
    libos::sys::clock_realtime().ok().map(|r| (r.time.secs, r.time.nanos))
}
