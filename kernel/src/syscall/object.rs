//! Cekani na objekty, eventy.

use alloc::sync::Arc;
use alloc::vec::Vec;

use abi::object::WaitItem;
use abi::syscall::{DEADLINE_INFINITE, WAIT_MANY_MAX};

use super::handle::arg_handle;
use super::{Args, current_process, user};
use crate::error::{Error, Result};
use crate::object::event::Event;
use crate::object::port::Port;
use crate::object::{KernelObject, ObjectType, Rights, Signals, charged, wait};

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

/// `OBJECT_SIGNAL(handle, clear, set)`: meni signal SIGNALED a uzivatelske
/// bity eventu (`USER_0..USER_7`).
pub fn signal(a: Args) -> Result<u64> {
    let (clear, set) = (mask(a[1])?, mask(a[2])?);
    // Uzivatel smi menit jen SIGNALED a uzivatelske bity; ostatni signaly
    // ridi jadro.
    if !(Signals::SIGNALED | Signals::USER_ALL).contains(clear | set) {
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
    let p = current_process();
    let event = charged(Event::new(), p.job())?;
    let rights = event.default_rights();
    let h = p.handles().insert(event, rights)?;
    Ok(h.0 as u64)
}

/// `PORT_CREATE() -> handle`.
pub fn port_create() -> Result<u64> {
    let p = current_process();
    let port = charged(Port::new(Some(p.job().clone())), p.job())?;
    let rights = port.default_rights();
    let h = p.handles().insert(port, rights)?;
    Ok(h.0 as u64)
}

/// `OBJECT_WAIT_ASYNC(handle, port, key, mask)`: jednorazova registrace.
/// Objekt potrebuje pravo `WAIT`, port `WRITE`; na port samotny nejde.
pub fn wait_async(a: Args) -> Result<u64> {
    let p = current_process();
    let (object, port) = {
        let table = p.handles();
        (
            table.get(arg_handle(a[0])?, Rights::WAIT)?,
            table.get_as::<Port>(arg_handle(a[1])?, Rights::WRITE)?,
        )
    };
    if object.object_type() == ObjectType::Port {
        return Err(Error::NotSupported);
    }
    let m = mask(a[3])?;
    if m == Signals::NONE {
        return Err(Error::InvalidArgs);
    }
    object.core().observe_async(&port, a[2], m)?;
    Ok(0)
}

/// `PORT_WAIT(port, deadline, out_ptr)`: vyzvedne paket (16 B na `out_ptr`).
pub fn port_wait(a: Args) -> Result<u64> {
    let p = current_process();
    let port = p.handles().get_as::<Port>(arg_handle(a[0])?, Rights::READ)?;
    loop {
        if let Some(packet) = port.pop() {
            crate::sched::boost_current(packet.boost);
            let out = abi::object::PortPacket {
                key: packet.key,
                signals: packet.signals.0,
                _reserved: 0,
            };
            // Neplatna adresa vystupu = chyba programu; paket je pak ztraceny.
            user::write_bytes(&p, a[2], &out.encode())?;
            return Ok(0);
        }
        wait::wait_one(&*port, Signals::READABLE, deadline(a[1]))?;
    }
}
