//! Symboly C `mo_*` nad syscally (jen cil magnumopus).
//!
//! Druha kopie `libos` (vedle kopie ve std) nema vlastni start: handle
//! procesu pro mapovani nastavi [`ensure_process`] z behoveho prostredi.

use std::os::magnumopus::runtime as rt;
use std::sync::Once;

use libos::abi::object::WaitItem;
use libos::Error;
use libos::abi::vm::{MAP_READ, MAP_WRITE};
use libos::sys;

use crate::deadline;

fn code(e: Error) -> i32 {
    e.code()
}

fn status(r: libos::Result<()>) -> i32 {
    match r {
        Ok(()) => 0,
        Err(e) => code(e),
    }
}

static PROCESS: Once = Once::new();

fn ensure_process() {
    PROCESS.call_once(|| libos::rt::set_self_process(rt::start::self_process()));
}

#[unsafe(no_mangle)]
pub extern "C" fn mo_now_ns() -> i64 {
    libos::time::now() as i64
}

#[unsafe(no_mangle)]
pub extern "C" fn mo_bootstrap() -> u32 {
    rt::start::bootstrap_channel().map_or(0, |c| c.handle().raw())
}

/// # Safety
///
/// `out` ukazuje na pole aspon `max` handlu (nebo `max` = 0).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mo_take_handles(out: *mut u32, max: usize) -> usize {
    let hs = rt::start::take_handles();
    let mut n = 0;
    for h in hs {
        if n < max && !out.is_null() {
            // SAFETY: `n < max`, volajici dal pole aspon `max` prvku.
            unsafe { out.add(n).write(h) };
            n += 1;
        } else {
            let _ = sys::handle_close(h);
        }
    }
    n
}

#[unsafe(no_mangle)]
pub extern "C" fn mo_handle_close(h: u32) -> i32 {
    status(sys::handle_close(h))
}

/// # Safety
///
/// `out` je platny ukazatel.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mo_handle_dup(h: u32, out: *mut u32) -> i32 {
    match sys::handle_duplicate(h, None) {
        Ok(d) => {
            // SAFETY: kontrakt C.
            unsafe { out.write(d) };
            0
        }
        Err(e) => code(e),
    }
}

/// # Safety
///
/// `data` ma `len` bajtu, `handles` ma `nhandles` prvku (nebo jsou NULL
/// s delkou 0).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mo_channel_write(ch: u32, data: *const u8, len: usize, handles: *const u32, nhandles: usize) -> i32 {
    // SAFETY: kontrakt C (prazdny rez pro NULL s delkou 0).
    let (d, h) = unsafe {
        (
            if len == 0 { &[][..] } else { core::slice::from_raw_parts(data, len) },
            if nhandles == 0 { &[][..] } else { core::slice::from_raw_parts(handles, nhandles) },
        )
    };
    status(sys::channel_write(ch, d, h))
}

/// # Safety
///
/// `buf` ma `cap` bajtu, `handles` ma `hcap` prvku; `len` a `nhandles`
/// jsou platne ukazatele.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mo_channel_read(
    ch: u32,
    buf: *mut u8,
    cap: usize,
    len: *mut usize,
    handles: *mut u32,
    hcap: usize,
    nhandles: *mut usize,
    deadline_ns: i64,
) -> i32 {
    // SAFETY: kontrakt C.
    let (d, h) = unsafe {
        (
            if cap == 0 { &mut [][..] } else { core::slice::from_raw_parts_mut(buf, cap) },
            if hcap == 0 { &mut [][..] } else { core::slice::from_raw_parts_mut(handles, hcap) },
        )
    };
    loop {
        let (r, (dl, hl)) = sys::channel_read(ch, d, h);
        // SAFETY: kontrakt C.
        unsafe {
            len.write(dl);
            nhandles.write(hl);
        }
        match r {
            Err(Error::ShouldWait) if deadline_ns != 0 => {
                let mask = (libos::Signals::READABLE | libos::Signals::PEER_CLOSED).0;
                let (w, _) = sys::object_wait_one(ch, mask, deadline(deadline_ns));
                if let Err(e) = w {
                    return code(e);
                }
            }
            r => return status(r),
        }
    }
}

/// # Safety
///
/// `addr` je platny ukazatel.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mo_vmo_map(vmo: u32, offset: u64, len: u64, write: i32, addr: *mut *mut u8) -> i32 {
    ensure_process();
    let page = libos::vm::page_size();
    let flags = MAP_READ | if write != 0 { MAP_WRITE } else { 0 };
    match sys::vm_map(libos::rt::self_process(), vmo, offset, len.next_multiple_of(page), flags, 0) {
        Ok(a) => {
            // SAFETY: kontrakt C.
            unsafe { addr.write(a as *mut u8) };
            0
        }
        Err(e) => code(e),
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn mo_vmo_unmap(addr: *mut u8, len: u64) -> i32 {
    ensure_process();
    let page = libos::vm::page_size();
    status(sys::vm_unmap(libos::rt::self_process(), addr as u64, len.next_multiple_of(page)))
}

#[unsafe(no_mangle)]
pub extern "C" fn mo_fence_signal(fence: u32, value: u64) -> i32 {
    status(sys::fence_signal(fence, value, 0))
}

/// # Safety
///
/// `value` je platny ukazatel.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mo_fence_value(fence: u32, value: *mut u64) -> i32 {
    match sys::fence_query(fence) {
        Ok(i) => {
            // SAFETY: kontrakt C.
            unsafe { value.write(i.value) };
            0
        }
        Err(e) => code(e),
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn mo_event_reset(ev: u32) -> i32 {
    status(sys::object_signal(ev, libos::Signals::SIGNALED.0, 0))
}

/// # Safety
///
/// `handles`, `masks` a `observed` maji `n` prvku.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mo_wait_many(handles: *const u32, masks: *const u32, observed: *mut u32, n: usize, deadline_ns: i64) -> i32 {
    if n == 0 {
        return Error::InvalidArgs.code();
    }
    // SAFETY: kontrakt C.
    let (hs, ms, obs) =
        unsafe { (core::slice::from_raw_parts(handles, n), core::slice::from_raw_parts(masks, n), core::slice::from_raw_parts_mut(observed, n)) };
    let mut items: Vec<WaitItem> = hs.iter().zip(ms).map(|(&handle, &mask)| WaitItem { handle, mask, observed: 0 }).collect();
    let r = sys::object_wait_many(&mut items, deadline(deadline_ns));
    for (o, it) in obs.iter_mut().zip(&items) {
        *o = it.observed;
    }
    match r {
        Ok(i) => i as i32,
        Err(e) => code(e),
    }
}
