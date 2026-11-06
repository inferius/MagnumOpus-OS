//! Cekani na objekty, eventy.

use alloc::sync::Arc;
use alloc::vec::Vec;

use abi::object::WaitItem;
use abi::syscall::{DEADLINE_INFINITE, WAIT_MANY_MAX};

use super::handle::arg_handle;
use super::{Args, current_process, user};
use crate::error::{Error, Result};
use crate::object::event::Event;
use crate::object::{KernelObject, Rights, Signals, wait};

fn deadline(v: u64) -> Option<u64> {
    (v != DEADLINE_INFINITE).then_some(v)
}

fn mask(v: u64) -> Result<Signals> {
    u32::try_from(v)
        .map(Signals)
        .map_err(|_| Error::InvalidArgs)
}

/// `OBJECT_WAIT_ONE(handle, mask, deadline, out_signals_ptr)`.
pub fn wait_one(a: Args) -> Result<u64> {
    let p = current_process();
    let object = p.handles().get(arg_handle(a[0])?, Rights::WAIT)?;
    let result = wait::wait_one(&*object, mask(a[1])?, deadline(a[2]));
    // Signaly zapsat i pri timeoutu (aktualni stav), pokud o ne proces stoji.
    let observed = match result {
        Ok(s) => s,
        Err(_) => object.core().signals(),
    };
    if a[3] != 0 {
        user::write_u32s(&p, a[3], &[observed.0])?;
    }
    result.map(|_| 0)
}

/// `OBJECT_WAIT_MANY(items_ptr, count, deadline) -> index`. Do kazde polozky
/// zapise `observed` = signaly objektu v okamziku navratu.
pub fn wait_many(a: Args) -> Result<u64> {
    let p = current_process();
    let count = user::check_len(a[1], WAIT_MANY_MAX)?;
    let raw = user::read_u32s(&p, a[0], count * 3)?;
    let mut objects: Vec<(Arc<dyn KernelObject>, Signals)> = Vec::with_capacity(count);
    {
        let table = p.handles();
        for item in raw.as_chunks::<3>().0 {
            objects.push((
                table.get(abi::Handle(item[0]), Rights::WAIT)?,
                Signals(item[1]),
            ));
        }
    }
    let refs: Vec<(&dyn KernelObject, Signals)> = objects.iter().map(|(o, m)| (&**o, *m)).collect();
    let result = wait::wait_many(&refs, deadline(a[2]));
    let out: Vec<u32> = objects
        .iter()
        .zip(raw.as_chunks::<3>().0)
        .flat_map(|((o, _), item)| {
            let w = WaitItem {
                handle: item[0],
                mask: item[1],
                observed: o.core().signals().0,
            };
            [w.handle, w.mask, w.observed]
        })
        .collect();
    user::write_u32s(&p, a[0], &out)?;
    result.map(|(i, _)| i as u64)
}

/// `OBJECT_SIGNAL(handle, clear, set)`: meni signal SIGNALED eventu.
pub fn signal(a: Args) -> Result<u64> {
    let (clear, set) = (mask(a[1])?, mask(a[2])?);
    // Uzivatel smi menit jen SIGNALED; ostatni signaly ridi jadro.
    if !(Signals::SIGNALED).contains(clear | set) {
        return Err(Error::InvalidArgs);
    }
    let p = current_process();
    let event = p
        .handles()
        .get_as::<Event>(arg_handle(a[0])?, Rights::SIGNAL)?;
    event.core().update(clear, set);
    Ok(0)
}

/// `EVENT_CREATE() -> handle`.
pub fn event_create() -> Result<u64> {
    let event = Event::new();
    let rights = event.default_rights();
    let h = current_process().handles().insert(event, rights)?;
    Ok(h.0 as u64)
}
