//! USB: protokol mezi ovladacem radice (`xhci`) a tridnimi ovladaci
//! (`usb-hub`, `usb-hid`), deskriptory USB, trida hub a HID v boot
//! protokolu.
//!
//! Kazde rozhrani zarizeni (a virtualni root hub radice) dostane vlastni
//! kanal; tridni ovladac na nem posila pozadavky ([`msg`]) a nevidi radic,
//! pamet ani ostatni zarizeni. Root porty radice jsou pro ovladac hubu
//! obycejny hub: stejne pozadavky tridy hub jako u externiho hubu, takze
//! vyhledani zarizeni i hotplug funguji pro oba stejne.
//!
//! Knihovna nezavisi na jadre a testuje se na hostiteli
//! (`cargo test -p usbproto`).
//!
//! Dokumentace: `docs/cs/usb.md`, `docs/en/usb.md`.

#![cfg_attr(not(test), no_std)]

extern crate alloc;

pub mod desc;
pub mod hid;
pub mod hid_report;
pub mod hub;
pub mod msg;
pub mod report;
pub mod storage;

/// Rychlost zarizeni.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Speed {
    /// 1,5 Mbit/s (USB 1.x low speed).
    Low,
    /// 12 Mbit/s (USB 1.x full speed).
    Full,
    /// 480 Mbit/s (USB 2.0).
    High,
    /// 5 Gbit/s a vic (USB 3.x).
    Super,
}

impl Speed {
    pub const fn to_u8(self) -> u8 {
        match self {
            Speed::Low => 0,
            Speed::Full => 1,
            Speed::High => 2,
            Speed::Super => 3,
        }
    }

    pub const fn from_u8(v: u8) -> Option<Self> {
        match v {
            0 => Some(Speed::Low),
            1 => Some(Speed::Full),
            2 => Some(Speed::High),
            3 => Some(Speed::Super),
            _ => None,
        }
    }

    /// Vychozi velikost paketu endpointu 0 pred prectenim deskriptoru.
    pub const fn default_ep0_packet(self) -> u16 {
        match self {
            Speed::Low | Speed::Full => 8,
            Speed::High => 64,
            Speed::Super => 512,
        }
    }

    pub const fn name(self) -> &'static str {
        match self {
            Speed::Low => "low speed",
            Speed::Full => "full speed",
            Speed::High => "high speed",
            Speed::Super => "super speed",
        }
    }
}

#[cfg(test)]
mod report_tests;
#[cfg(test)]
mod storage_tests;
#[cfg(test)]
mod tests;
