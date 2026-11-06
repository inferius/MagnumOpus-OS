//! Kontrolni soucty bloku.
//!
//! Kazdy ukazatel na blok nese 128bitovy kontrolni soucet obsahu (Merkleuv
//! strom az ke koreni). Algoritmus se voli pri formatovani:
//! - `Xxh3` (vychozi): XXH3-128, velmi rychly, nekryptograficky,
//! - `Blake3`: BLAKE3 zkraceny na 128 bitu, kryptograficky (deduplikace,
//!   podepisovani korene systemovych snapshotu).
//!
//! Superblok samotny ma vzdy XXH3-128 (musi jit overit driv, nez je znamy
//! zvoleny algoritmus).

use abi::{Error, Result};

/// 128bitovy kontrolni soucet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash, PartialOrd, Ord)]
pub struct Csum(pub [u8; 16]);

/// Algoritmus kontrolnich souctu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Algo {
    Xxh3 = 1,
    Blake3 = 2,
}

impl Algo {
    pub fn from_raw(v: u8) -> Result<Self> {
        match v {
            1 => Ok(Algo::Xxh3),
            2 => Ok(Algo::Blake3),
            _ => Err(Error::NotSupported),
        }
    }

    pub fn sum(self, data: &[u8]) -> Csum {
        match self {
            Algo::Xxh3 => xxh3(data),
            Algo::Blake3 => {
                let h = blake3::hash(data);
                let mut out = [0u8; 16];
                out.copy_from_slice(&h.as_bytes()[..16]);
                Csum(out)
            }
        }
    }
}

/// XXH3-128 (pouziva i superblok).
pub fn xxh3(data: &[u8]) -> Csum {
    Csum(xxhash_rust::xxh3::xxh3_128(data).to_le_bytes())
}

/// XXH3-128 se seminkem (intent log: soucet vazany na epochu a poradi).
pub fn xxh3_seeded(data: &[u8], seed: u64) -> Csum {
    Csum(xxhash_rust::xxh3::xxh3_128_with_seed(data, seed).to_le_bytes())
}
