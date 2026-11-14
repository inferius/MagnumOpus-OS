//! Syscally plotu (`FENCE_*`, `abi::fence`).

use alloc::sync::Arc;
use alloc::vec::Vec;

use abi::fence::{Info, WaitItem, signal, wait as wait_flags};
use abi::syscall::{DEADLINE_INFINITE, WAIT_MANY_MAX};

use super::handle::arg_handle;
use super::{Args, current_process, user};
use crate::error::{Error, Result};
use crate::object::fence::{self, Fence, FenceSignaler};
use crate::object::port::Port;
use crate::object::{KernelObject, Rights, charged};

fn fence_handle(a: u64, rights: Rights) -> Result<Arc<Fence>> {
    let p = current_process();
    let object = p.handles().get(arg_handle(a)?, rights)?;
    fence::fence_of(object)
}

/// `FENCE_CREATE(initial, flags) -> handle` s pravy `BASIC | SIGNAL`.
pub fn create(a: Args) -> Result<u64> {
    if a[1] != 0 {
        return Err(Error::InvalidArgs);
    }
    let p = current_process();
    let fence = charged(Fence::new(a[0]), p.job())?;
    let rights = fence.default_rights();
    let h = p.handles().insert(FenceSignaler::new(fence), rights)?;
    Ok(h.0 as u64)
}

/// `FENCE_SIGNAL(fence, value, flags)`, pravo `SIGNAL`.
pub fn signal(a: Args) -> Result<u64> {
    if a[2] & !signal::ALL != 0 {
        return Err(Error::InvalidArgs);
    }
    let fence = fence_handle(a[0], Rights::SIGNAL)?;
    fence.signal(a[1], a[2] & signal::FAILED != 0)?;
    Ok(0)
}

/// `FENCE_WAIT(items_ptr, count, flags, deadline, out_index_ptr) -> index`.
pub fn wait(a: Args) -> Result<u64> {
    if a[2] & !wait_flags::KNOWN != 0 {
        return Err(Error::InvalidArgs);
    }
    let count = user::check_len(a[1], WAIT_MANY_MAX)?;
    if count == 0 {
        return Err(Error::InvalidArgs);
    }
    let p = current_process();
    let raw = user::read_bytes(&p, a[0], count * WaitItem::SIZE)?;
    let mut items: Vec<(Arc<Fence>, u64)> = Vec::with_capacity(count);
    {
        let table = p.handles();
        for chunk in raw.as_chunks::<{ WaitItem::SIZE }>().0 {
            let item = WaitItem::decode(chunk);
            let object = table.get(abi::Handle(item.handle), Rights::WAIT)?;
            items.push((fence::fence_of(object)?, item.value));
        }
    }
    let deadline = (a[3] != DEADLINE_INFINITE).then_some(a[3]);
    match fence::wait(&items, a[2] & wait_flags::ALL != 0, deadline) {
        Ok(i) => Ok(i as u64),
        Err((e, index)) => {
            if let Some(i) = index
                && a[4] != 0
            {
                user::write_u32s(&p, a[4], &[i as u32])?;
            }
            Err(e)
        }
    }
}

/// `FENCE_WAIT_ASYNC(fence, value, port, key)`: plot `WAIT`, port `WRITE`.
pub fn wait_async(a: Args) -> Result<u64> {
    let fence = fence_handle(a[0], Rights::WAIT)?;
    let port = current_process()
        .handles()
        .get_as::<Port>(arg_handle(a[2])?, Rights::WRITE)?;
    fence.observe_async(&port, a[3], a[1])?;
    Ok(0)
}

/// `FENCE_QUERY(fence, out_ptr)`: zapise `fence::Info`, pravo `WAIT`.
pub fn query(a: Args) -> Result<u64> {
    let fence = fence_handle(a[0], Rights::WAIT)?;
    let info: Info = fence.info();
    user::write_bytes(&current_process(), a[1], &info.encode())?;
    Ok(0)
}
