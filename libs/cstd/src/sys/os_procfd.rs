//! Procesovy fd (`<magnumopus/process.h>`, Windows vrstva W2): handle
//! vlastniho procesu jen s pravy pameti pro server aplikace (wineserver)
//! a `PROCESS_MEMORY` nad nim.

use std::os::magnumopus::runtime as rt;

use rt::fd::{self, Description, Object};

use super::R;
use crate::errno;

fn e(err: rt::Error) -> crate::c_int {
    errno::from_os(err as i32)
}

/// Novy fd s handlem vlastniho procesu jen s pravy `READ_MEMORY`
/// a `WRITE_MEMORY` (a `DUPLICATE`/`TRANSFER`, aby sel predat); vzdy
/// s `FD_CLOEXEC` (nededi se omylem do deti).
pub fn self_process_fd() -> R<i32> {
    let obj = rt::process::self_memory_object().map_err(e)?;
    fd::insert(Description::new(obj, super::status::RDWR), 0, true).map_err(|err| match err {
        rt::Error::NoResources => errno::EMFILE,
        err => e(err),
    })
}

/// `PROCESS_MEMORY` nad procesovym fd: `write` = zapis `buf` do `addr`,
/// jinak cteni z `addr` do `buf`.
pub fn process_memory(fd: i32, write: bool, addr: u64, buf: *mut u8, len: usize) -> R<usize> {
    let d = fd::get(fd).map_err(|_| errno::EBADF)?;
    let Object::Process(p) = &d.object else { return Err(errno::EINVAL) };
    if len == 0 {
        return Ok(0);
    }
    if buf.is_null() {
        return Err(errno::EFAULT);
    }
    if write {
        // SAFETY: `buf` ukazuje na `len` bajtu volajiciho (kontrakt C).
        let data = unsafe { core::slice::from_raw_parts(buf as *const u8, len) };
        p.write_memory(addr, data).map_err(e)
    } else {
        // SAFETY: viz vyse.
        let data = unsafe { core::slice::from_raw_parts_mut(buf, len) };
        p.read_memory(addr, data).map_err(e)
    }
}
