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

/// `LOG_READ(resource, cursor, buf_ptr, buf_len, out_start_ptr) -> bajty`.
pub fn log_read(a: Args) -> Result<u64> {
    check_diag(a[0])?;
    let p = current_process();
    let len = user::check_len(a[3], LOG_READ_MAX)?;
    let mut buf = user::zeroed(len);
    let (start, n) = crate::log::read_from(a[1], &mut buf);
    user::write_bytes(&p, a[2], &buf[..n])?;
    user::write_u64(&p, a[4], start)?;
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
