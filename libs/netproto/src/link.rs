//! Protokol mezi ovladacem sitove karty a sluzbou `netd` (vrstva linky).
//!
//! Kanal je obousmerny a bez odpovedi: kazda zprava zacina typem (u8).
//!
//! | Typ | Smer | Obsah |
//! |---|---|---|
//! | `INFO` | ovladac -> netd, prvni zprava | MAC (6 B), MTU u16, priznaky u8 |
//! | `FRAME` | oba | ramec Ethernetu bez FCS |
//! | `LINK` | ovladac -> netd | priznaky u8 (zmena stavu linky) |
//!
//! Ramec, ktery se nevejde do fronty druhe strany (`SHOULD_WAIT`), se
//! zahodi - jako na skutecne siti. Spolehlivost resi TCP.

use abi::{Error, Result};

pub const INFO: u8 = 1;
pub const FRAME: u8 = 2;
pub const LINK: u8 = 3;

/// Priznak: linka je pripojena.
pub const FLAG_UP: u8 = 1;

/// Nejvetsi ramec Ethernetu bez FCS (MTU 1500 + hlavicka 14 B).
pub const MAX_FRAME: usize = 1514;

/// Popis karty.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Info {
    pub mac: [u8; 6],
    pub mtu: u16,
    pub flags: u8,
}

impl Info {
    pub fn encode(&self) -> [u8; 10] {
        let mut b = [0u8; 10];
        b[0] = INFO;
        b[1..7].copy_from_slice(&self.mac);
        b[7..9].copy_from_slice(&self.mtu.to_le_bytes());
        b[9] = self.flags;
        b
    }

    pub fn decode(b: &[u8]) -> Result<Self> {
        if b.len() < 10 || b[0] != INFO {
            return Err(Error::InvalidArgs);
        }
        Ok(Self {
            mac: b[1..7].try_into().unwrap(),
            mtu: u16::from_le_bytes([b[7], b[8]]),
            flags: b[9],
        })
    }
}
