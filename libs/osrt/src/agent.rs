//! Kanal podpisu klienta SSH (`abi::params::tag::SSH_AGENT`, protokol
//! `accountproto::agent`): dostanou ho jen systemove programy `ssh`, `scp`
//! a `sftp` od initu. Runtime zpravy nekoduje, jen je prenasi; kanal si
//! vezme pri prvnim pouziti z nepouzitych tagovanych handlu startovni
//! zpravy.

use alloc::vec::Vec;

use libos::sync::Mutex;
use libos::{Channel, Handle};

use crate::{Error, Result};

/// `None` = jeste se nehledal; `Some(None)` = proces kanal nema.
static AGENT: Mutex<Option<Option<Channel>>> = Mutex::new(None);

fn with<T>(f: impl FnOnce(&Channel) -> Result<T>) -> Result<T> {
    let mut g = AGENT.lock();
    if g.is_none() {
        let ch = crate::start::take_tagged(abi::params::tag::SSH_AGENT).map(|h| Channel::from_handle(Handle::from_raw(h)));
        *g = Some(ch);
    }
    match g.as_ref().and_then(Option::as_ref) {
        Some(ch) => f(ch),
        None => Err(Error::NotSupported),
    }
}

/// Ma proces kanal podpisu?
pub fn available() -> bool {
    with(|_| Ok(())).is_ok()
}

/// Pozadavek a odpoved (pod zamkem, jeden rozpracovany). Odpoved muze
/// prijit az po systemove vyzve cloveku (sluzba uctu ceka nejvys 300 s).
pub fn call(req: &[u8]) -> Result<Vec<u8>> {
    with(|ch| {
        ch.write(req, Vec::new())?;
        Ok(ch.read(None)?.data)
    })
}
