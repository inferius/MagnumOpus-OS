//! Joby: rozpocty prostredku pro skupiny procesu.
//!
//! Kazdy proces patri do jednoho jobu. Joby tvori strom (koren dostane
//! init od jadra). Vse, co proces stoji, se strhava z rozpoctu jeho jobu
//! a zaroven ze vsech predku - limit predka tak plati pro cely jeho podstrom.
//!
//! Dva oddelene rozpocty v bajtech ([`pool`]):
//! - **pamet jadra**: objekty jadra, sloty handlu, registrace portu,
//!   zpravy ve frontach kanalu, mapovani a tabulky stranek, zasobniky
//!   jadra vlaken,
//! - **uzivatelska pamet**: stranky objektu pameti (vcetne pameti pro DMA).
//!
//! Prekroceni rozpoctu = `NO_RESOURCES`. Nedostatek fyzicke pameti systemu
//! je dal `NO_MEMORY` (rezerva jadra se pro programy nepouzije nikdy).

/// Rozpocty jobu (argument `pool` syscallu `JOB_SET_LIMIT`).
pub mod pool {
    /// Pamet jadra (objekty, handly, fronty, tabulky stranek).
    pub const KERNEL: u32 = 0;
    /// Uzivatelska pamet (stranky objektu pameti).
    pub const USER: u32 = 1;
    /// Pocet rozpoctu.
    pub const COUNT: usize = 2;
}

/// Limit "bez vlastniho omezeni" (plati jen limity predku).
pub const UNLIMITED: u64 = u64::MAX;

/// Stav jednoho rozpoctu.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PoolInfo {
    /// Strzeno z tohoto jobu (vcetne potomku).
    pub used: u64,
    /// Vlastni limit jobu ([`UNLIMITED`] = jen limity predku).
    pub limit: u64,
    /// Kolik jeste jde strhnout: nejmensi volne misto pres job a vsechny
    /// predky ([`UNLIMITED`], kdyz nikde neni limit).
    pub available: u64,
}

/// Vystup `JOB_INFO` (48 B, little endian: pro jadro a pak uzivatelskou
/// pamet vzdy `used`, `limit`, `available` jako u64).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct JobInfo {
    pub kernel: PoolInfo,
    pub user: PoolInfo,
}

impl JobInfo {
    pub const SIZE: usize = 48;

    pub fn encode(&self) -> [u8; Self::SIZE] {
        let mut b = [0u8; Self::SIZE];
        let values = [
            self.kernel.used,
            self.kernel.limit,
            self.kernel.available,
            self.user.used,
            self.user.limit,
            self.user.available,
        ];
        for (i, v) in values.iter().enumerate() {
            b[i * 8..i * 8 + 8].copy_from_slice(&v.to_le_bytes());
        }
        b
    }

    pub fn decode(b: &[u8; Self::SIZE]) -> Self {
        let v = |i: usize| u64::from_le_bytes(b[i * 8..i * 8 + 8].try_into().unwrap());
        Self {
            kernel: PoolInfo {
                used: v(0),
                limit: v(1),
                available: v(2),
            },
            user: PoolInfo {
                used: v(3),
                limit: v(4),
                available: v(5),
            },
        }
    }
}
