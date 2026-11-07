//! Klient kanalu USB pro tridni ovladace (`usb-hub`, `usb-hid`):
//! synchronni pozadavky nad protokolem `usbproto::msg`.
//!
//! Klient posila jeden pozadavek po druhem a ceka na odpoved se stejnym
//! `id`; odpovedi na drive zrusene pozadavky (po vyprseni casu) zahodi.

#![no_std]

extern crate alloc;

use alloc::vec::Vec;
use core::time::Duration;

use libos::{Channel, Error, Result, time};
use usbproto::desc::Setup;
use usbproto::msg::{self, Info, Reply, Request, op};

/// Vysledek prenosu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Transfer {
    pub completion: u32,
    pub data: Vec<u8>,
}

/// Chyba prenosu: kod `abi::Error` a kod dokonceni xHCI (STALL ...).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UsbError {
    pub error: Error,
    pub completion: u32,
}

impl From<Error> for UsbError {
    fn from(error: Error) -> Self {
        Self {
            error,
            completion: 0,
        }
    }
}

impl core::fmt::Display for UsbError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        if self.completion != 0 {
            write!(f, "{} (kod xHCI {})", self.error, self.completion)
        } else {
            write!(f, "{}", self.error)
        }
    }
}

pub type UsbResult<T> = core::result::Result<T, UsbError>;

/// Jak dlouho cekat na kontrolni prenos.
pub const CONTROL_TIMEOUT: Duration = Duration::from_secs(5);

pub struct UsbClient {
    pub channel: Channel,
    next_id: u32,
}

impl UsbClient {
    pub fn new(channel: Channel) -> Self {
        Self {
            channel,
            next_id: 1,
        }
    }

    /// Posle pozadavek a vrati jeho id (odpoved precte [`UsbClient::wait`]).
    pub fn send(&mut self, op: u32, a: u32, b: u32, data: &[u8]) -> Result<u32> {
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1).max(1);
        let raw = Request {
            op,
            id,
            a,
            b,
            data,
        }
        .encode();
        self.channel.write(&raw, Vec::new())?;
        Ok(id)
    }

    /// Pocka na odpoved `id` (do terminu `deadline`, `None` = neomezene).
    pub fn wait(&self, id: u32, deadline: Option<u64>) -> UsbResult<Transfer> {
        loop {
            let m = self.channel.read(deadline)?;
            let Some(r) = Reply::decode(&m.data) else {
                return Err(Error::Corrupted.into());
            };
            if r.id != id {
                continue;
            }
            if r.status != 0 {
                let error = Error::from_code(r.status as i64).unwrap_or(Error::IoError);
                return Err(UsbError {
                    error,
                    completion: r.completion,
                });
            }
            return Ok(Transfer {
                completion: r.completion,
                data: r.data.to_vec(),
            });
        }
    }

    /// Pozadavek a cekani na odpoved.
    pub fn call(&mut self, op: u32, a: u32, b: u32, data: &[u8], timeout: Option<Duration>) -> UsbResult<Transfer> {
        let id = self.send(op, a, b, data)?;
        self.wait(id, timeout.map(time::deadline_after))
    }

    /// Kontrolni prenos (IN vraci data).
    pub fn control(&mut self, setup: Setup, out: &[u8]) -> UsbResult<Vec<u8>> {
        let mut data = Vec::with_capacity(Setup::SIZE + out.len());
        data.extend_from_slice(&setup.to_bytes());
        data.extend_from_slice(out);
        self.call(op::CONTROL, 0, 0, &data, Some(CONTROL_TIMEOUT))
            .map(|t| t.data)
    }

    pub fn info(&mut self) -> UsbResult<Info> {
        let t = self.call(op::INFO, 0, 0, &[], Some(CONTROL_TIMEOUT))?;
        Info::decode(&t.data).ok_or(Error::Corrupted.into())
    }
}

/// Zkratka: je chyba STALL (zarizeni pozadavek nepodporuje)?
pub fn is_stall(e: &UsbError) -> bool {
    e.completion == msg::completion::STALL
}
