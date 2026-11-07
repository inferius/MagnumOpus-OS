//! Virtualni root hub: porty radice predstavene ovladaci hubu jako
//! obycejny hub USB 2.0 (pozadavky tridy hub, endpoint zmen 0x81). Ovladac
//! hubu tak obsluhuje root porty i externi huby stejne.
//!
//! Prevod `PORTSC` <-> `wPortStatus`/`wPortChange`: pripojeni (CCS),
//! povoleni (PED), nadproud (OCA), reset (PR), napajeni (PP), rychlost
//! (low / high / super speed v rezervovanem bitu 15) a zmeny CSC, PEC, OCC,
//! PRC (+ WRC u USB 3).

use alloc::vec::Vec;

use libos::{Error, Result};
use usbproto::desc::{DeviceDescriptor, Setup, class, kind, request, rt};
use usbproto::hub::{HubDescriptor, PortStatus, feature, port};

use crate::hc::Hc;
use crate::regs::*;

/// Deskriptor zarizeni virtualniho root hubu (trida hub).
pub const DEVICE: [u8; DeviceDescriptor::SIZE] = [
    18,
    kind::DEVICE,
    0x00,
    0x02,
    class::HUB,
    0,
    0,
    64,
    0x00,
    0x00, // vyrobce 0 = virtualni
    0x01,
    0x00,
    0x00,
    0x01,
    0,
    0,
    0,
    1,
];

impl Hc {
    pub fn port_status(&self, p: u8) -> PortStatus {
        let sc = self.regs.portsc(p);
        let mut status = 0;
        let mut change = 0;
        for (bit, s) in [
            (PORT_CCS, port::CONNECTION),
            (PORT_PED, port::ENABLE),
            (PORT_OCA, port::OVER_CURRENT),
            (PORT_PR, port::RESET),
            (PORT_PP, port::POWER),
        ] {
            if sc & bit != 0 {
                status |= s;
            }
        }
        if sc & PORT_CCS != 0 {
            status |= match (sc >> PORT_SPEED_SHIFT) & 0xF {
                2 => port::LOW_SPEED,
                3 => port::HIGH_SPEED,
                4.. => port::SUPER_SPEED,
                _ => 0,
            };
        }
        for (bit, c) in [
            (PORT_CSC, port::C_CONNECTION),
            (PORT_PEC, port::C_ENABLE),
            (PORT_OCC, port::C_OVER_CURRENT),
            (PORT_PRC | PORT_WRC, port::C_RESET),
        ] {
            if sc & bit != 0 {
                change |= c;
            }
        }
        PortStatus { status, change }
    }

    /// Bitmapa zmen (bit N = port N) jako na endpointu stavu hubu.
    pub fn port_changes(&self) -> Vec<u8> {
        let mut map = alloc::vec![0u8; (self.ports as usize + 1).div_ceil(8)];
        for p in 1..=self.ports {
            if self.regs.portsc(p) & PORT_CHANGES != 0 {
                map[p as usize / 8] |= 1 << (p % 8);
            }
        }
        map
    }

    /// Zapis do PORTSC: zachovat napajeni a nastavit `set` (RW1C bity smazou
    /// zmeny, PED port vypne).
    fn port_write(&self, p: u8, set: u32) {
        let sc = self.regs.portsc(p);
        self.regs.set_portsc(p, (sc & PORT_PRESERVE) | set);
    }

    /// Kontrolni pozadavek na root hub.
    pub fn root_control(&mut self, s: Setup) -> Result<Vec<u8>> {
        let class_req = s.request_type & 0x60 == rt::CLASS;
        let to_port = s.request_type & 0x1F == rt::TO_OTHER;
        let p = s.index as u8;
        let port_ok = (1..=self.ports).contains(&p);
        let take = |v: &[u8]| v[..v.len().min(s.length as usize)].to_vec();
        match (class_req, s.request) {
            (false, request::GET_DESCRIPTOR) if s.value >> 8 == kind::DEVICE as u16 => {
                Ok(take(&DEVICE))
            }
            (true, request::GET_DESCRIPTOR) => {
                let d = HubDescriptor {
                    ports: self.ports,
                    // Napajeni a nadproud po portech.
                    characteristics: 0x0009,
                    power_on_2ms: 10,
                };
                Ok(take(&d.to_bytes()))
            }
            (true, request::GET_STATUS) if to_port && port_ok => {
                Ok(take(&self.port_status(p).to_bytes()))
            }
            // Stav hubu samotneho: vse v poradku.
            (true, request::GET_STATUS) => Ok(take(&[0, 0, 0, 0])),
            (true, request::SET_FEATURE) if to_port && port_ok => {
                match s.value {
                    feature::PORT_POWER => self.port_write(p, PORT_PP),
                    feature::PORT_RESET => self.port_write(p, PORT_PR),
                    // Uspani portu zatim ne; ostatni vlastnosti nic nedelaji.
                    _ => {}
                }
                Ok(Vec::new())
            }
            (true, request::CLEAR_FEATURE) if to_port && port_ok => {
                let bits = match s.value {
                    feature::PORT_ENABLE => PORT_PED,
                    feature::C_PORT_CONNECTION => PORT_CSC,
                    feature::C_PORT_ENABLE => PORT_PEC,
                    feature::C_PORT_OVER_CURRENT => PORT_OCC,
                    feature::C_PORT_RESET => PORT_PRC | PORT_WRC,
                    feature::PORT_POWER => {
                        let sc = self.regs.portsc(p);
                        self.regs.set_portsc(p, sc & PORT_PRESERVE & !PORT_PP);
                        return Ok(Vec::new());
                    }
                    _ => 0,
                };
                if bits != 0 {
                    self.port_write(p, bits);
                }
                Ok(Vec::new())
            }
            (false, request::SET_CONFIGURATION) => Ok(Vec::new()),
            _ => Err(Error::NotSupported),
        }
    }

    /// Rychlost zarizeni na root portu (z PORTSC).
    pub fn port_speed(&self, p: u8) -> usbproto::Speed {
        crate::device::speed_from_id((self.regs.portsc(p) >> PORT_SPEED_SHIFT) & 0xF)
    }
}
