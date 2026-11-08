//! Nahoda z jadra pro klice a bezpecnostni klic za kanalem od usb-hid.

use core::time::Duration;

use accountproto::ctap::hid::PACKET;
use accountproto::ctap::hidclient::HidDevice;
use libos::{Channel, Error, time};
use rand_core::{CryptoRng, RngCore};

/// Generator jadra (ChaCha20). Klice se tvori jen z nasazeneho
/// generatoru: dokud nema 256 bitu entropie, ceka se.
pub struct OsRng;

impl RngCore for OsRng {
    fn next_u32(&mut self) -> u32 {
        let mut b = [0u8; 4];
        self.fill_bytes(&mut b);
        u32::from_le_bytes(b)
    }

    fn next_u64(&mut self) -> u64 {
        let mut b = [0u8; 8];
        self.fill_bytes(&mut b);
        u64::from_le_bytes(b)
    }

    fn fill_bytes(&mut self, dest: &mut [u8]) {
        loop {
            match libos::random::fill_seeded(dest) {
                Ok(()) => return,
                Err(Error::ShouldWait) => time::sleep(Duration::from_millis(20)),
                Err(e) => panic!("accountd: generator nahody: {e}"),
            }
        }
    }

    fn try_fill_bytes(&mut self, dest: &mut [u8]) -> Result<(), rand_core::Error> {
        self.fill_bytes(dest);
        Ok(())
    }
}

impl CryptoRng for OsRng {}

/// Bezpecnostni klic: kazda zprava kanalu = jedno hlaseni HID (64 B).
/// Kanal dava init (od `usb-hid`, rozhrani s usage page 0xF1D0).
pub struct ChannelHid<'a>(pub &'a Channel);

impl HidDevice for ChannelHid<'_> {
    fn write(&mut self, packet: &[u8; PACKET]) -> libos::Result<()> {
        self.0.write(packet, alloc::vec::Vec::new())
    }

    fn read(&mut self, timeout_ms: u64) -> libos::Result<[u8; PACKET]> {
        let m = self.0.read(Some(time::deadline_after(Duration::from_millis(timeout_ms))))?;
        m.data.as_slice().try_into().map_err(|_| Error::InvalidArgs)
    }
}
