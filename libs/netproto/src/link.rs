//! Protokol mezi ovladacem sitove karty a sluzbou `netd` (vrstva linky).
//!
//! Kanal je obousmerny a bez odpovedi: kazda zprava zacina typem (u8).
//!
//! | Typ | Smer | Obsah |
//! |---|---|---|
//! | `INFO` | ovladac -> netd, prvni zprava | MAC (6 B), MTU u16, priznaky u8, rychlost u32 |
//! | `FRAME` | oba | ramec Ethernetu bez FCS |
//! | `LINK` | ovladac -> netd | priznaky u8, rychlost u32 (zmena stavu linky) |
//!
//! Cisla jsou little-endian. Rychlost je v Mb/s, 0 = neznama (virtio,
//! linka odpojena). Starsi kratsi zpravy (bez rychlosti) se ctou dal:
//! `INFO` o 10 B, `LINK` o 2 B.
//!
//! Offloady (kontrolni soucty, TSO) protokol zatim nenese: smoltcp
//! v netd overuje a pocita soucty pro cele zarizeni, ne pro jednotlivy
//! ramec, a karta, ktera overi jen nektere ramce (IP volby, fragmenty),
//! by soucet neusetrila. Vzdy 1 ramec = 1 zprava, MTU 1500.
//!
//! Ramec, ktery se nevejde do fronty druhe strany (`SHOULD_WAIT`), se
//! zahodi - jako na skutecne siti. Spolehlivost resi TCP.

use abi::{Error, Result};

pub const INFO: u8 = 1;
pub const FRAME: u8 = 2;
pub const LINK: u8 = 3;

/// Priznak: linka je pripojena.
pub const FLAG_UP: u8 = 1;
/// Priznak: plny duplex (jen s `FLAG_UP`).
pub const FLAG_FULL_DUPLEX: u8 = 2;

/// Nejvetsi ramec Ethernetu bez FCS (MTU 1500 + hlavicka 14 B).
pub const MAX_FRAME: usize = 1514;

/// Popis karty.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Info {
    pub mac: [u8; 6],
    pub mtu: u16,
    pub flags: u8,
    /// Rychlost linky v Mb/s, 0 = neznama.
    pub speed_mbps: u32,
}

impl Info {
    pub fn encode(&self) -> [u8; 14] {
        let mut b = [0u8; 14];
        b[0] = INFO;
        b[1..7].copy_from_slice(&self.mac);
        b[7..9].copy_from_slice(&self.mtu.to_le_bytes());
        b[9] = self.flags;
        b[10..14].copy_from_slice(&self.speed_mbps.to_le_bytes());
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
            speed_mbps: b.get(10..14).map_or(0, |s| u32::from_le_bytes(s.try_into().unwrap())),
        })
    }
}

/// Stav linky ve zprave `LINK`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Status {
    pub flags: u8,
    pub speed_mbps: u32,
}

impl Status {
    pub fn encode(&self) -> [u8; 6] {
        let mut b = [0u8; 6];
        b[0] = LINK;
        b[1] = self.flags;
        b[2..6].copy_from_slice(&self.speed_mbps.to_le_bytes());
        b
    }

    pub fn decode(b: &[u8]) -> Result<Self> {
        if b.len() < 2 || b[0] != LINK {
            return Err(Error::InvalidArgs);
        }
        Ok(Self {
            flags: b[1],
            speed_mbps: b.get(2..6).map_or(0, |s| u32::from_le_bytes(s.try_into().unwrap())),
        })
    }
}
