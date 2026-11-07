//! Nahodna cisla z generatoru jadra (ChaCha20, viz `docs/cs/syscally.md`,
//! `RANDOM_GET`).
//!
//! [`fill`] vraci data vzdy a rika, zda je generator nasazeny (ma aspon
//! 256 bitu entropie od hardwaru). Pro klice a tajemstvi pouzivejte
//! [`fill_seeded`]: dokud generator nasazeny neni, vraci `SHOULD_WAIT`.

use abi::syscall::{RANDOM_MAX, random_flags};
use abi::{Error, Result};

use crate::hw::Resource;
use crate::sys;

/// Naplni `buf`; `true` = generator nasazeny.
pub fn fill(buf: &mut [u8]) -> bool {
    let mut seeded = true;
    for chunk in buf.chunks_mut(RANDOM_MAX) {
        match sys::random_get(chunk, 0) {
            Ok(s) => seeded &= s,
            Err(_) => {
                seeded = false;
            }
        }
    }
    seeded
}

/// Naplni `buf`, jen kdyz je generator nasazeny (jinak `SHOULD_WAIT`).
pub fn fill_seeded(buf: &mut [u8]) -> Result<()> {
    for chunk in buf.chunks_mut(RANDOM_MAX) {
        sys::random_get(chunk, random_flags::REQUIRE_SEEDED)?;
    }
    Ok(())
}

/// Nahodne `u64` (`true` = generator nasazeny).
pub fn u64() -> (u64, bool) {
    let mut b = [0u8; 8];
    let s = fill(&mut b);
    (u64::from_le_bytes(b), s)
}

/// Entropie od ovladace zdroje nahody (vyzaduje Resource s pravem
/// `MANAGE`); `credit_bits` = odhad nahody v datech.
pub fn add(resource: &Resource, data: &[u8], credit_bits: u32) -> Result<()> {
    if data.len() > RANDOM_MAX {
        return Err(Error::OutOfRange);
    }
    sys::random_add(resource.handle().raw(), data, credit_bits)
}
