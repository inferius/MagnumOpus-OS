//! Prenos CTAP2 pres CTAPHID: prideleni kanalu (INIT), rozdeleni zpravy
//! do paketu, cekani na odpoved (KEEPALIVE = klic ceka na dotek).

use alloc::vec::Vec;

use super::client::{CtapError, CtapResult, Transport};
use super::hid::{self, Assembler, BROADCAST, PACKET, cmd};

/// Zarizeni HID FIDO (hlaseni po 64 B). accountd: kanal od usb-hid,
/// testy: simulovany klic.
pub trait HidDevice {
    fn write(&mut self, packet: &[u8; PACKET]) -> abi::Result<()>;
    /// Dalsi hlaseni; `TIMED_OUT` po `timeout_ms`.
    fn read(&mut self, timeout_ms: u64) -> abi::Result<[u8; PACKET]>;
}

pub struct HidTransport<D> {
    pub dev: D,
    cid: u32,
    /// Jak dlouho cekat na odpoved (vcetne doteku uzivatele).
    pub timeout_ms: u64,
    /// Klic hlasil "ceka na dotek" (pro obrazovku: "Dotkni se klice").
    pub up_needed: bool,
}

impl<D: HidDevice> HidTransport<D> {
    /// Prideli kanal (`INIT` na vysilacim kanalu s nahodnym `nonce`).
    pub fn open(mut dev: D, nonce: [u8; 8]) -> CtapResult<Self> {
        for p in hid::fragment(BROADCAST, cmd::INIT, &nonce).map_err(|_| CtapError::Invalid)? {
            dev.write(&p).map_err(CtapError::Transport)?;
        }
        let mut asm = Assembler::default();
        loop {
            let p = dev.read(2000).map_err(CtapError::Transport)?;
            let Ok(Some(m)) = asm.push(&p) else { continue };
            if m.cid != BROADCAST || m.command != cmd::INIT || m.data.len() < 17 || m.data[..8] != nonce {
                continue;
            }
            let cid = u32::from_be_bytes(m.data[8..12].try_into().map_err(|_| CtapError::Invalid)?);
            return Ok(HidTransport { dev, cid, timeout_ms: 30_000, up_needed: false });
        }
    }

    pub fn channel(&self) -> u32 {
        self.cid
    }
}

impl<D: HidDevice> Transport for HidTransport<D> {
    fn transact(&mut self, request: &[u8]) -> CtapResult<Vec<u8>> {
        for p in hid::fragment(self.cid, cmd::CBOR, request).map_err(|_| CtapError::Invalid)? {
            self.dev.write(&p).map_err(CtapError::Transport)?;
        }
        self.up_needed = false;
        let mut asm = Assembler::default();
        loop {
            let p = self.dev.read(self.timeout_ms).map_err(CtapError::Transport)?;
            let m = match asm.push(&p) {
                Ok(Some(m)) => m,
                Ok(None) => continue,
                Err(_) => return Err(CtapError::Invalid),
            };
            if m.cid != self.cid {
                continue;
            }
            match m.command {
                cmd::KEEPALIVE => {
                    if m.data.first() == Some(&hid::keepalive::UP_NEEDED) {
                        self.up_needed = true;
                    }
                }
                cmd::CBOR => return Ok(m.data),
                cmd::ERROR => return Err(CtapError::Transport(abi::Error::IoError)),
                _ => return Err(CtapError::Invalid),
            }
        }
    }
}
