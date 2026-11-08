//! Diagnostika: cteni logu jadra a zaznam padu (Resource s pravem `DIAG`).

use alloc::vec::Vec;

use super::handle::arg_handle;
use super::{Args, current_process, user};
use crate::error::{Error, Result};
use crate::object::Rights;
use crate::object::resource::Resource;

/// Nejvic bajtu logu v jednom `LOG_READ`.
const LOG_READ_MAX: usize = 64 * 1024;

fn check_diag(a0: u64) -> Result<()> {
    current_process()
        .handles()
        .get_as::<Resource>(arg_handle(a0)?, Rights::DIAG)
        .map(|_| ())
}

/// `LOG_READ(resource, cursor, buf_ptr, buf_len, out_ptr) -> bajty`;
/// na `out_ptr` zapise (zacatek u64, ztracene radky u64).
pub fn log_read(a: Args) -> Result<u64> {
    check_diag(a[0])?;
    let p = current_process();
    let len = user::check_len(a[3], LOG_READ_MAX)?;
    let mut buf = user::zeroed(len);
    let (start, n, lost) = crate::log::read_from(a[1], &mut buf);
    user::write_bytes(&p, a[2], &buf[..n])?;
    user::write_u64(&p, a[4], start)?;
    user::write_u64(&p, a[4] + 8, lost)?;
    Ok(n as u64)
}

/// `CRASH_RECORD(resource, op, buf_ptr, buf_len) -> bajty`.
pub fn crash_record(a: Args) -> Result<u64> {
    use abi::syscall::crash_op;
    check_diag(a[0])?;
    match a[1] {
        crash_op::READ => {
            let mut rec = Vec::new();
            let n = crate::pstore::read(&mut rec);
            if n == 0 {
                return Ok(0);
            }
            if (a[3] as usize) < n {
                return Err(Error::BufferTooSmall);
            }
            user::write_bytes(&current_process(), a[2], &rec)?;
            Ok(n as u64)
        }
        crash_op::CLEAR => {
            crate::pstore::clear();
            Ok(0)
        }
        crash_op::CAPACITY => Ok(crate::pstore::capacity() as u64),
        crash_op::TEST_PANIC if cfg!(feature = "qemu-test") => {
            crate::pstore::mark_test_panic();
            panic!("test zaznamu padu: panika vyvolana pres CRASH_RECORD TEST_PANIC");
        }
        crash_op::TEST_PANIC => Err(Error::NotSupported),
        _ => Err(Error::InvalidArgs),
    }
}

/// Uroven z textu.
fn parse_level(s: &str) -> Option<crate::log::Level> {
    use crate::log::Level;
    Some(match s {
        "error" => Level::Error,
        "warn" => Level::Warn,
        "info" => Level::Info,
        "debug" => Level::Debug,
        "trace" => Level::Trace,
        _ => return None,
    })
}

/// `LOG_CONTROL(resource, op, buf_ptr, buf_len) -> bajty`.
pub fn log_control(a: Args) -> Result<u64> {
    use abi::syscall::log_op;
    check_diag(a[0])?;
    let p = current_process();
    match a[1] {
        log_op::SET | log_op::CLEAR => {
            let len = user::check_len(a[3], crate::log::MAX_TARGET + 8)?;
            let raw = user::read_bytes(&p, a[2], len)?;
            let text = core::str::from_utf8(&raw).map_err(|_| Error::InvalidArgs)?.trim();
            if a[1] == log_op::CLEAR {
                crate::log::clear_target_level(text);
                crate::log::info!("uroven logu pro {text}: vychozi (proces {})", p.name);
                return Ok(0);
            }
            let (target, level) = text.split_once('=').ok_or(Error::InvalidArgs)?;
            let (target, level) = (target.trim(), level.trim());
            let level = parse_level(level).ok_or(Error::InvalidArgs)?;
            if !crate::log::set_target_level(target, level) {
                return Err(Error::InvalidArgs);
            }
            crate::log::info!("uroven logu pro {target}: {level:?} (proces {})", p.name);
            Ok(0)
        }
        log_op::LIST => {
            let mut out = alloc::string::String::new();
            crate::log::for_each_override(|t, l| {
                out.push_str(&alloc::format!("{t}={}\n", alloc::format!("{l:?}").to_lowercase()));
            });
            out.push_str(&alloc::format!(
                "potlaceno={}\n",
                crate::log::limit::SUPPRESSED.load(core::sync::atomic::Ordering::Relaxed)
            ));
            if (a[3] as usize) < out.len() {
                return Err(Error::BufferTooSmall);
            }
            user::write_bytes(&p, a[2], out.as_bytes())?;
            Ok(out.len() as u64)
        }
        _ => Err(Error::InvalidArgs),
    }
}
