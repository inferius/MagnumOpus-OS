//! Trida hub (USB 2.0 kapitola 11). Stejne pozadavky umi i virtualni root
//! hub radice xHCI.

use crate::desc::{Setup, kind, request, rt};

/// Vlastnosti portu (`SET_FEATURE` / `CLEAR_FEATURE` s prijemcem "other").
pub mod feature {
    pub const PORT_CONNECTION: u16 = 0;
    pub const PORT_ENABLE: u16 = 1;
    pub const PORT_SUSPEND: u16 = 2;
    pub const PORT_OVER_CURRENT: u16 = 3;
    pub const PORT_RESET: u16 = 4;
    pub const PORT_POWER: u16 = 8;
    pub const PORT_LOW_SPEED: u16 = 9;
    pub const C_PORT_CONNECTION: u16 = 16;
    pub const C_PORT_ENABLE: u16 = 17;
    pub const C_PORT_SUSPEND: u16 = 18;
    pub const C_PORT_OVER_CURRENT: u16 = 19;
    pub const C_PORT_RESET: u16 = 20;
}

/// Bity `wPortStatus` (stav) a `wPortChange` (zmeny) z `GET_STATUS` portu.
pub mod port {
    pub const CONNECTION: u16 = 1 << 0;
    pub const ENABLE: u16 = 1 << 1;
    pub const SUSPEND: u16 = 1 << 2;
    pub const OVER_CURRENT: u16 = 1 << 3;
    pub const RESET: u16 = 1 << 4;
    pub const POWER: u16 = 1 << 8;
    pub const LOW_SPEED: u16 = 1 << 9;
    pub const HIGH_SPEED: u16 = 1 << 10;
    /// Rozsireni virtualniho root hubu (v USB 2.0 rezervovany bit): port
    /// je USB 3 a zarizeni na nem super speed. Externi hub USB 2 ho nikdy
    /// nenastavi.
    pub const SUPER_SPEED: u16 = 1 << 15;

    pub const C_CONNECTION: u16 = 1 << 0;
    pub const C_ENABLE: u16 = 1 << 1;
    pub const C_SUSPEND: u16 = 1 << 2;
    pub const C_OVER_CURRENT: u16 = 1 << 3;
    pub const C_RESET: u16 = 1 << 4;
}

/// Stav portu: (`wPortStatus`, `wPortChange`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PortStatus {
    pub status: u16,
    pub change: u16,
}

impl PortStatus {
    pub fn to_bytes(self) -> [u8; 4] {
        let s = self.status.to_le_bytes();
        let c = self.change.to_le_bytes();
        [s[0], s[1], c[0], c[1]]
    }

    pub fn from_bytes(b: &[u8]) -> Option<Self> {
        let b: &[u8; 4] = b.get(..4)?.try_into().ok()?;
        Some(Self {
            status: u16::from_le_bytes([b[0], b[1]]),
            change: u16::from_le_bytes([b[2], b[3]]),
        })
    }

    /// Rychlost pripojeneho zarizeni podle bitu stavu.
    pub fn speed(&self) -> crate::Speed {
        use crate::Speed;
        if self.status & port::SUPER_SPEED != 0 {
            Speed::Super
        } else if self.status & port::HIGH_SPEED != 0 {
            Speed::High
        } else if self.status & port::LOW_SPEED != 0 {
            Speed::Low
        } else {
            Speed::Full
        }
    }
}

/// Deskriptor hubu (cast, kterou ovladac pouziva).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HubDescriptor {
    pub ports: u8,
    pub characteristics: u16,
    /// Doba od zapnuti napajeni portu do stabilniho stavu, v jednotkach 2 ms.
    pub power_on_2ms: u8,
}

impl HubDescriptor {
    /// Zakodovani (pro virtualni root hub); `DeviceRemovable` vsechny 0.
    pub fn to_bytes(self) -> alloc::vec::Vec<u8> {
        let bitmap = (self.ports as usize + 1).div_ceil(8);
        let len = 7 + 2 * bitmap;
        let mut v = alloc::vec![0u8; len];
        v[0] = len as u8;
        v[1] = kind::HUB;
        v[2] = self.ports;
        v[3..5].copy_from_slice(&self.characteristics.to_le_bytes());
        v[5] = self.power_on_2ms;
        // Maska napajeni (PortPwrCtrlMask): vsechny jednicky.
        for b in &mut v[7 + bitmap..] {
            *b = 0xFF;
        }
        v
    }

    pub fn parse(b: &[u8]) -> Option<Self> {
        if b.len() < 7 || (b[1] != kind::HUB && b[1] != kind::SS_HUB) {
            return None;
        }
        Some(Self {
            ports: b[2],
            characteristics: u16::from_le_bytes([b[3], b[4]]),
            power_on_2ms: b[5],
        })
    }
}

/// `GET_DESCRIPTOR` hubu (trida, prijemce zarizeni).
pub const fn get_hub_descriptor(length: u16) -> Setup {
    Setup {
        request_type: rt::DEVICE_TO_HOST | rt::CLASS | rt::TO_DEVICE,
        request: request::GET_DESCRIPTOR,
        value: (kind::HUB as u16) << 8,
        index: 0,
        length,
    }
}

/// `GET_STATUS` portu `port` (od 1).
pub const fn get_port_status(port: u8) -> Setup {
    Setup {
        request_type: rt::DEVICE_TO_HOST | rt::CLASS | rt::TO_OTHER,
        request: request::GET_STATUS,
        value: 0,
        index: port as u16,
        length: 4,
    }
}

/// `SET_FEATURE` portu.
pub const fn set_port_feature(port: u8, feature: u16) -> Setup {
    Setup {
        request_type: rt::HOST_TO_DEVICE | rt::CLASS | rt::TO_OTHER,
        request: request::SET_FEATURE,
        value: feature,
        index: port as u16,
        length: 0,
    }
}

/// `CLEAR_FEATURE` portu.
pub const fn clear_port_feature(port: u8, feature: u16) -> Setup {
    Setup {
        request_type: rt::HOST_TO_DEVICE | rt::CLASS | rt::TO_OTHER,
        request: request::CLEAR_FEATURE,
        value: feature,
        index: port as u16,
        length: 0,
    }
}

/// Endpoint stavu hubu: interrupt IN 1, bitmapa zmen (bit 0 hub, bit N
/// port N).
pub const STATUS_ENDPOINT: u8 = 0x81;
