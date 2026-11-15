//! Ridici kanal terminalu (`abi::params::tag::TTY`, protokol `ttyproto`):
//! C `termios.h` v cstd. Runtime zpravy nekoduje, jen je prenasi; kanal si
//! vezme pri prvnim pouziti z nepouzitych tagovanych handlu startovni
//! zpravy (`start::take_tagged`).

use alloc::vec::Vec;

use libos::sync::Mutex;
use libos::{Channel, Event, Handle, Signals};

use crate::{Error, Result};

/// `None` = jeste se nehledal; `Some(None)` = proces terminal nema.
static TTY: Mutex<Option<Option<Channel>>> = Mutex::new(None);

fn with<T>(f: impl FnOnce(&Channel) -> Result<T>) -> Result<T> {
    let mut g = TTY.lock();
    if g.is_none() {
        let ch = crate::start::take_tagged(abi::params::tag::TTY).map(|h| Channel::from_handle(Handle::from_raw(h)));
        *g = Some(ch);
    }
    match g.as_ref().and_then(Option::as_ref) {
        Some(ch) => f(ch),
        None => Err(Error::NotSupported),
    }
}

/// Ma proces terminal (C `isatty` u stdio)?
pub fn available() -> bool {
    with(|_| Ok(())).is_ok()
}

/// Pozadavek a odpoved na ridicim kanalu: (data, syrove handly odpovedi,
/// ktere volajici vlastni). Pod zamkem: na kanalu je jeden pozadavek.
pub fn call(req: &[u8]) -> Result<(Vec<u8>, Vec<u32>)> {
    with(|ch| {
        ch.write(req, Vec::new())?;
        let msg = ch.read(None)?;
        let handles = msg.handles.into_iter().map(Handle::into_raw).collect();
        Ok((msg.data, handles))
    })
}

/// Zavre syrove handly z odpovedi, ktere volajici nepotrebuje.
pub fn close_handles(h: Vec<u32>) {
    for x in h {
        drop(Handle::from_raw(x));
    }
}

/// Pocka na udalost zmeny velikosti okna (handle z `WATCH_SIZE`) a shodi
/// ji. Volajici handle vlastni (nezavira se tady).
pub fn wait_resize(raw: u32) -> Result<()> {
    let ev = Event::from_handle(Handle::from_raw(raw));
    let r = libos::wait_one(ev.handle(), Signals::SIGNALED, None).and_then(|_| ev.reset());
    // Handle patri volajicimu.
    core::mem::forget(ev);
    r.map(|_| ())
}
