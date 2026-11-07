//! Logovani a cas.

use super::{Args, current_process, user};
use crate::arch;
use crate::error::Result;
use crate::log;

/// `DEBUG_LOG(ptr, len)`: zapise text procesu do logu jadra.
pub fn debug_log(a: Args) -> Result<u64> {
    let p = current_process();
    let len = user::check_len(a[1], abi::syscall::DEBUG_LOG_MAX)?;
    let bytes = user::read_bytes(&p, a[0], len)?;
    let text = alloc::string::String::from_utf8_lossy(&bytes);
    log::info!("[{} {}] {}", p.name, p.id, text.trim_end());
    Ok(0)
}

/// `CLOCK_MONOTONIC()`: ns od startu jadra.
pub fn clock_monotonic() -> Result<u64> {
    Ok(arch::now_ns())
}

/// `CLOCK_REALTIME(out_ptr)`: realny cas (16 B, viz `abi::time::Reading`).
/// `NOT_SUPPORTED`, kdyz jadro realny cas nezna.
pub fn clock_realtime(a: Args) -> Result<u64> {
    let r = crate::clock::now().ok_or(crate::error::Error::NotSupported)?;
    user::write_bytes(&current_process(), a[0], &r.encode())?;
    Ok(0)
}

/// `RANDOM_GET(buf_ptr, len, flags) -> 1 = nasazeny, 0 = jeste ne`.
pub fn random_get(a: Args) -> Result<u64> {
    use abi::syscall::{RANDOM_MAX, random_flags};
    if a[2] & !random_flags::ALL != 0 {
        return Err(crate::error::Error::InvalidArgs);
    }
    let len = user::check_len(a[1], RANDOM_MAX)?;
    if a[2] & random_flags::REQUIRE_SEEDED != 0 && !crate::random::is_seeded() {
        return Err(crate::error::Error::ShouldWait);
    }
    let mut buf = user::zeroed(len);
    let seeded = crate::random::fill(&mut buf);
    user::write_bytes(&current_process(), a[0], &buf)?;
    // Kopie v jadre nenechat lezet v pameti.
    buf.fill(0);
    Ok(seeded as u64)
}
