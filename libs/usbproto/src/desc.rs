//! Standardni pozadavky a deskriptory USB 2.0 (kapitola 9 specifikace).

use alloc::vec::Vec;

/// Typy deskriptoru.
pub mod kind {
    pub const DEVICE: u8 = 1;
    pub const CONFIGURATION: u8 = 2;
    pub const STRING: u8 = 3;
    pub const INTERFACE: u8 = 4;
    pub const ENDPOINT: u8 = 5;
    /// Deskriptor HID (v konfiguraci za rozhranim).
    pub const HID: u8 = 0x21;
    /// Deskriptor reportu HID.
    pub const HID_REPORT: u8 = 0x22;
    /// Deskriptor hubu USB 2.0.
    pub const HUB: u8 = 0x29;
    /// Deskriptor hubu USB 3.x.
    pub const SS_HUB: u8 = 0x2A;
}

/// Standardni pozadavky (`bRequest`).
pub mod request {
    pub const GET_STATUS: u8 = 0;
    pub const CLEAR_FEATURE: u8 = 1;
    pub const SET_FEATURE: u8 = 3;
    pub const SET_ADDRESS: u8 = 5;
    pub const GET_DESCRIPTOR: u8 = 6;
    pub const GET_CONFIGURATION: u8 = 8;
    pub const SET_CONFIGURATION: u8 = 9;
    pub const SET_INTERFACE: u8 = 11;
}

/// Tridy (`bInterfaceClass`, `bDeviceClass`).
pub mod class {
    /// Trida podle rozhrani.
    pub const PER_INTERFACE: u8 = 0x00;
    pub const HID: u8 = 0x03;
    pub const MASS_STORAGE: u8 = 0x08;
    pub const HUB: u8 = 0x09;
}

/// `bmRequestType`: smer, typ a prijemce.
pub mod rt {
    pub const DEVICE_TO_HOST: u8 = 0x80;
    pub const HOST_TO_DEVICE: u8 = 0x00;
    pub const STANDARD: u8 = 0x00;
    pub const CLASS: u8 = 0x20;
    pub const VENDOR: u8 = 0x40;
    pub const TO_DEVICE: u8 = 0x00;
    pub const TO_INTERFACE: u8 = 0x01;
    pub const TO_ENDPOINT: u8 = 0x02;
    pub const TO_OTHER: u8 = 0x03;
}

/// Setup paket kontrolniho prenosu (8 B).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Setup {
    pub request_type: u8,
    pub request: u8,
    pub value: u16,
    pub index: u16,
    pub length: u16,
}

impl Setup {
    pub const SIZE: usize = 8;

    pub fn to_bytes(self) -> [u8; 8] {
        let mut b = [0u8; 8];
        b[0] = self.request_type;
        b[1] = self.request;
        b[2..4].copy_from_slice(&self.value.to_le_bytes());
        b[4..6].copy_from_slice(&self.index.to_le_bytes());
        b[6..8].copy_from_slice(&self.length.to_le_bytes());
        b
    }

    pub fn from_bytes(b: &[u8]) -> Option<Self> {
        let b: &[u8; 8] = b.get(..8)?.try_into().ok()?;
        Some(Self {
            request_type: b[0],
            request: b[1],
            value: u16::from_le_bytes([b[2], b[3]]),
            index: u16::from_le_bytes([b[4], b[5]]),
            length: u16::from_le_bytes([b[6], b[7]]),
        })
    }

    /// Smer dat: od zarizeni k hostiteli (IN)?
    pub const fn is_in(&self) -> bool {
        self.request_type & rt::DEVICE_TO_HOST != 0
    }

    /// GET_DESCRIPTOR zarizeni.
    pub const fn get_descriptor(kind: u8, index: u8, length: u16) -> Self {
        Self {
            request_type: rt::DEVICE_TO_HOST | rt::STANDARD | rt::TO_DEVICE,
            request: request::GET_DESCRIPTOR,
            value: (kind as u16) << 8 | index as u16,
            index: 0,
            length,
        }
    }

    pub const fn set_configuration(value: u8) -> Self {
        Self {
            request_type: rt::HOST_TO_DEVICE | rt::STANDARD | rt::TO_DEVICE,
            request: request::SET_CONFIGURATION,
            value: value as u16,
            index: 0,
            length: 0,
        }
    }

    /// SET_INTERFACE: alternativni nastaveni `alternate` rozhrani
    /// `interface`.
    pub const fn set_interface(interface: u8, alternate: u8) -> Self {
        Self {
            request_type: rt::HOST_TO_DEVICE | rt::STANDARD | rt::TO_INTERFACE,
            request: request::SET_INTERFACE,
            value: alternate as u16,
            index: interface as u16,
            length: 0,
        }
    }
}

/// Deskriptor zarizeni (18 B).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DeviceDescriptor {
    pub usb_version: u16,
    pub class: u8,
    pub subclass: u8,
    pub protocol: u8,
    pub max_packet0: u8,
    pub vendor: u16,
    pub product: u16,
    pub device_version: u16,
    pub manufacturer_index: u8,
    pub product_index: u8,
    pub serial_index: u8,
    pub configurations: u8,
}

impl DeviceDescriptor {
    pub const SIZE: usize = 18;

    pub fn parse(b: &[u8]) -> Option<Self> {
        if b.len() < Self::SIZE || b[1] != kind::DEVICE {
            return None;
        }
        let u16_at = |o: usize| u16::from_le_bytes([b[o], b[o + 1]]);
        Some(Self {
            usb_version: u16_at(2),
            class: b[4],
            subclass: b[5],
            protocol: b[6],
            max_packet0: b[7],
            vendor: u16_at(8),
            product: u16_at(10),
            device_version: u16_at(12),
            manufacturer_index: b[14],
            product_index: b[15],
            serial_index: b[16],
            configurations: b[17],
        })
    }
}

/// Typ prenosu endpointu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Transfer {
    Control,
    Isochronous,
    Bulk,
    Interrupt,
}

/// Endpoint z deskriptoru konfigurace.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Endpoint {
    /// Adresa: cislo (0-15) a bit 0x80 = IN.
    pub address: u8,
    pub transfer: Transfer,
    pub max_packet: u16,
    /// Interval dotazovani (`bInterval`, vyznam podle rychlosti).
    pub interval: u8,
}

impl Endpoint {
    pub const fn is_in(&self) -> bool {
        self.address & 0x80 != 0
    }

    pub const fn number(&self) -> u8 {
        self.address & 0x0F
    }
}

/// Rozhrani z deskriptoru konfigurace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Interface {
    pub number: u8,
    pub alternate: u8,
    pub class: u8,
    pub subclass: u8,
    pub protocol: u8,
    pub endpoints: Vec<Endpoint>,
    /// Delka deskriptoru reportu HID (z deskriptoru HID), 0 = neni HID.
    pub hid_report_len: u16,
}

/// Rozebrana konfigurace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Configuration {
    /// Hodnota pro SET_CONFIGURATION.
    pub value: u8,
    /// Odber v jednotkach 2 mA (USB 2) / 8 mA (USB 3).
    pub max_power: u8,
    /// Jen zakladni nastaveni rozhrani (alternate 0).
    pub interfaces: Vec<Interface>,
    /// Alternativni nastaveni rozhrani (alternate != 0), v poradi
    /// deskriptoru.
    pub alternates: Vec<Interface>,
}

impl Configuration {
    /// Rozebere celou konfiguraci (vsechny deskriptory za sebou). Nezname
    /// deskriptory preskoci; poskozena delka = `None`.
    pub fn parse(b: &[u8]) -> Option<Self> {
        if b.len() < 9 || b[1] != kind::CONFIGURATION {
            return None;
        }
        let total = (u16::from_le_bytes([b[2], b[3]]) as usize).min(b.len());
        let mut cfg = Configuration {
            value: b[5],
            max_power: b[8],
            interfaces: Vec::new(),
            alternates: Vec::new(),
        };
        let mut off = b[0] as usize;
        // Rozhrani s alternate != 0 jdou do `alternates`.
        let mut current: Option<Interface> = None;
        while off + 2 <= total {
            let len = b[off] as usize;
            if len < 2 || off + len > total {
                return None;
            }
            let d = &b[off..off + len];
            match d[1] {
                kind::INTERFACE if len >= 9 => {
                    if let Some(i) = current.take() {
                        cfg.push(i);
                    }
                    current = Some(Interface {
                        number: d[2],
                        alternate: d[3],
                        class: d[5],
                        subclass: d[6],
                        protocol: d[7],
                        endpoints: Vec::new(),
                        hid_report_len: 0,
                    });
                }
                kind::ENDPOINT if len >= 7 => {
                    if let Some(i) = &mut current {
                        i.endpoints.push(Endpoint {
                            address: d[2],
                            transfer: match d[3] & 3 {
                                0 => Transfer::Control,
                                1 => Transfer::Isochronous,
                                2 => Transfer::Bulk,
                                _ => Transfer::Interrupt,
                            },
                            max_packet: u16::from_le_bytes([d[4], d[5]]) & 0x7FF,
                            interval: d[6],
                        });
                    }
                }
                kind::HID if len >= 9 => {
                    if let Some(i) = &mut current {
                        // Prvni podrizeny deskriptor byva report (typ 0x22).
                        if d[6] == kind::HID_REPORT {
                            i.hid_report_len = u16::from_le_bytes([d[7], d[8]]);
                        }
                    }
                }
                _ => {}
            }
            off += len;
        }
        if let Some(i) = current {
            cfg.push(i);
        }
        Some(cfg)
    }

    fn push(&mut self, i: Interface) {
        if i.alternate == 0 {
            self.interfaces.push(i);
        } else {
            self.alternates.push(i);
        }
    }

    /// Alternativni nastaveni `alternate` rozhrani `number`.
    pub fn alternate(&self, number: u8, alternate: u8) -> Option<&Interface> {
        self.alternates
            .iter()
            .find(|i| i.number == number && i.alternate == alternate)
    }

    /// Kopie konfigurace, kde rozhrani `number` pouziva alternativni
    /// nastaveni `alternate` (pro endpointy po SET_INTERFACE).
    pub fn with_alternate(&self, number: u8, alternate: u8) -> Option<Self> {
        let alt = self.alternate(number, alternate)?.clone();
        let mut c = self.clone();
        let slot = c.interfaces.iter_mut().find(|i| i.number == number)?;
        *slot = alt;
        Some(c)
    }

    /// Celkova delka konfigurace z prvnich 9 B (pro druhe cteni).
    pub fn total_length(head: &[u8]) -> Option<u16> {
        (head.len() >= 4 && head[1] == kind::CONFIGURATION)
            .then(|| u16::from_le_bytes([head[2], head[3]]))
    }
}
