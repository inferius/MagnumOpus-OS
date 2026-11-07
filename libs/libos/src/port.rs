//! Port: cekani na libovolne mnozstvi objektu (viz `OBJECT_WAIT_ASYNC`,
//! `PORT_WAIT` v `docs/cs/syscally.md`).
//!
//! ```ignore
//! let port = Port::create()?;
//! port.wait_async(channel.handle(), 7, Signals::READABLE | Signals::PEER_CLOSED)?;
//! let p = port.wait(None)?;          // p.key == 7
//! // ... obslouzit kanal, pak znovu zaregistrovat:
//! port.wait_async(channel.handle(), 7, Signals::READABLE | Signals::PEER_CLOSED)?;
//! ```
//!
//! Registrace je jednorazova: po doruceni paketu je treba ji obnovit.
//! Kazda registrace = jeden paket, takze objekt, ktery se hlida porad,
//! ma ve fronte nejvys jeden paket.

pub use abi::object::PortPacket;
use abi::syscall::DEADLINE_INFINITE;
use abi::{Result, Signals};

use crate::handle::object_type;
use crate::{Handle, sys};

#[derive(Debug)]
pub struct Port(Handle);
object_type!(Port);

impl Port {
    pub fn create() -> Result<Self> {
        sys::port_create().map(|h| Self(Handle::from_raw(h)))
    }

    /// Az signaly objektu `h` protnou `mask`, port dostane paket s `key`.
    pub fn wait_async(&self, h: &Handle, key: u64, mask: Signals) -> Result<()> {
        sys::object_wait_async(h.raw(), self.0.raw(), key, mask.0)
    }

    /// Dalsi paket; `deadline` v ns od startu (`None` = bez limitu).
    pub fn wait(&self, deadline: Option<u64>) -> Result<PortPacket> {
        sys::port_wait(self.0.raw(), deadline.unwrap_or(DEADLINE_INFINITE))
    }
}
