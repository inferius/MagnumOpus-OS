//! Cas: monotonni (od startu, pro mereni a deadliny) a realny (hodiny na
//! zdi, pro casy souboru a zobrazeni).

use core::time::Duration;

pub use abi::time::{Civil, Reading, WallTime, flags, source};

use crate::sys;

/// Realny cas (sekundy a nanosekundy od 1970 UTC); `None` = system ho nezna.
/// Neni monotonni: muze skocit pri nastaveni hodin.
pub fn wall() -> Option<WallTime> {
    sys::clock_realtime().ok().map(|r| r.time)
}

/// Realny cas i se zdrojem a priznaky (napr. pasmo bylo jen predpokladane).
pub fn wall_reading() -> crate::Result<Reading> {
    sys::clock_realtime()
}

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
