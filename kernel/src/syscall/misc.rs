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
