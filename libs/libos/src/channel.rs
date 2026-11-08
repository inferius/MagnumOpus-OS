//! Kanal (IPC).

use alloc::vec;
use alloc::vec::Vec;

use crate::handle::object_type;
use crate::{Handle, sys, wait_one};
use abi::{Error, Result, Signals};

#[derive(Debug)]
pub struct Channel(Handle);
object_type!(Channel);

/// Prijata zprava.
#[derive(Debug)]
pub struct Message {
    pub data: Vec<u8>,
    pub handles: Vec<Handle>,
}

/// Pocatecni velikost bufferu pro cteni zpravy.
const READ_BUFFER: usize = 4096;

impl Channel {
    /// Dvojice propojenych koncu.
    pub fn create() -> Result<(Self, Self)> {
        let (a, b) = sys::channel_create()?;
        Ok((Self(Handle::from_raw(a)), Self(Handle::from_raw(b))))
    }

    /// Posle zpravu. Handly se predavaji vzdy (i kdyz zapis selze).
    pub fn write(&self, data: &[u8], handles: Vec<Handle>) -> Result<()> {
        let raw: Vec<u32> = handles.into_iter().map(Handle::into_raw).collect();
        sys::channel_write(self.0.raw(), data, &raw)
    }

    /// Precte zpravu, pokud nejaka ceka (jinak `ShouldWait`).
    pub fn try_read(&self) -> Result<Message> {
        // 4 KiB pokryje vetsinu zprav (GUI, terminal) na jedno cteni;
        // s 256 B se kazda vetsi zprava cetla dvakrat (BUFFER_TOO_SMALL).
        let mut data = vec![0u8; READ_BUFFER];
        let mut handles = vec![0u32; 4];
        loop {
            let (r, (dlen, hlen)) = sys::channel_read(self.0.raw(), &mut data, &mut handles);
            match r {
                Ok(()) => {
                    data.truncate(dlen);
                    // Mala zprava nedrzi zbytecne 4 KiB (zpravy se ukladaji
                    // do front).
                    if data.capacity() > 2 * dlen.max(64) {
                        data.shrink_to_fit();
                    }
                    handles.truncate(hlen);
                    return Ok(Message {
                        data,
                        handles: handles.into_iter().map(Handle::from_raw).collect(),
                    });
                }
                Err(Error::BufferTooSmall) => {
                    data.resize(dlen.max(data.len()), 0);
                    handles.resize(hlen.max(handles.len()), 0);
                }
                Err(e) => return Err(e),
            }
        }
    }

    /// Pocka na zpravu (nebo uzavreni druhe strany) a precte ji.
    pub fn read(&self, deadline: Option<u64>) -> Result<Message> {
        loop {
            match self.try_read() {
                Err(Error::ShouldWait) => {
                    wait_one(&self.0, Signals::READABLE | Signals::PEER_CLOSED, deadline)?;
                }
                other => return other,
            }
        }
    }
}
