//! usb-hub: ovladac tridy hub (USB 2.0 kapitola 11).
//!
//! Obsluhuje virtualni root hub radice i externi huby stejne: zapne
//! napajeni portu, najde pripojena zarizeni, port resetuje a radici ohlasi
//! nove zarizeni (`ATTACH`); pak ceka na zmeny na endpointu stavu 0x81
//! (hotplug: pripojeni i odpojeni).
//!
//! Startovni zprava (kind USER): kanal rozhrani hubu (protokol
//! `usbproto::msg`). U externiho hubu radic sam nastavi route string a TT
//! (zarizeni low/full speed za hubem high speed); hub USB 3 (deskriptor
//! 0x2A, jiny format stavu portu) zatim ne.

#![no_std]
#![no_main]

extern crate alloc;

use alloc::vec::Vec;
use core::time::Duration;

use libos::{Channel, Error, Startup, println, time};
use usbclient::{UsbClient, UsbResult};
use usbproto::Speed;
use usbproto::hub::{self, HubDescriptor, PortStatus, feature, port};
use usbproto::msg::op;

libos::entry!(main);

/// Jak dlouho cekat na dokonceni resetu portu.
const RESET_TIMEOUT: Duration = Duration::from_millis(500);
/// Zotaveni zarizeni po resetu (USB 2.0: TRSTRCY 10 ms).
const RESET_RECOVERY: Duration = Duration::from_millis(10);

struct Hub {
    usb: UsbClient,
    ports: u8,
    /// Virtualni root hub radice: porty adresuje cislo root portu, ne
    /// route string.
    root: bool,
    /// Porty, na kterych je nastavene zarizeni.
    attached: Vec<bool>,
}

fn main(startup: Startup) -> i64 {
    let Some(ch) = startup.handles.into_iter().next() else {
        println!("usb-hub: chybi kanal rozhrani");
        return 1;
    };
    let mut usb = UsbClient::new(Channel::from_handle(ch));
    let info = match usb.info() {
        Ok(i) => i,
        Err(e) => {
            println!("usb-hub: INFO selhalo: {e}");
            return 2;
        }
    };
    let root = info.path.is_empty();
    let desc = match usb
        .control(hub::get_hub_descriptor(16), &[])
        .map(|d| HubDescriptor::parse(&d))
    {
        Ok(Some(d)) => d,
        Ok(None) => {
            println!("usb-hub: neplatny deskriptor hubu");
            return 3;
        }
        Err(e) => {
            println!("usb-hub: deskriptor hubu: {e}");
            return 3;
        }
    };
    println!(
        "usb-hub: {} hub {:?}, {} portu",
        if root { "root" } else { "externi" },
        info.path,
        desc.ports
    );
    let mut h = Hub {
        usb,
        ports: desc.ports,
        root,
        attached: alloc::vec![false; desc.ports as usize + 1],
    };
    match h.run(desc) {
        Ok(()) => 0,
        Err(e) => {
            // Zavreny kanal = radic nebo zarizeni zmizelo: normalni konec.
            if e.error != Error::PeerClosed {
                println!("usb-hub: {e}");
            }
            0
        }
    }
}

impl Hub {
    fn status(&mut self, p: u8) -> UsbResult<PortStatus> {
        let d = self.usb.control(hub::get_port_status(p), &[])?;
        PortStatus::from_bytes(&d).ok_or(Error::Corrupted.into())
    }

    fn run(&mut self, desc: HubDescriptor) -> UsbResult<()> {
        for p in 1..=self.ports {
            self.usb.control(hub::set_port_feature(p, feature::PORT_POWER), &[])?;
        }
        libos::time::sleep(Duration::from_millis(2 * desc.power_on_2ms as u64));
        for p in 1..=self.ports {
            self.check_port(p)?;
        }
        self.usb.call(op::SCAN_DONE, 0, 0, &[], Some(Duration::from_secs(5)))?;
        let bitmap_len = (self.ports as u32 + 1).div_ceil(8);
        loop {
            // Bitmapa zmen: bit N = port N.
            let t = self.usb.call(op::INTERRUPT_IN, hub::STATUS_ENDPOINT as u32, bitmap_len, &[], None)?;
            for p in 1..=self.ports {
                if t.data.get(p as usize / 8).is_some_and(|b| b & (1 << (p % 8)) != 0) {
                    self.check_port(p)?;
                }
            }
        }
    }

    /// Projde stav portu: smaze zmeny, nove zarizeni nastavi, odpojene
    /// ohlasi.
    fn check_port(&mut self, p: u8) -> UsbResult<()> {
        let st = self.status(p)?;
        for (c, f) in [
            (port::C_CONNECTION, feature::C_PORT_CONNECTION),
            (port::C_ENABLE, feature::C_PORT_ENABLE),
            (port::C_OVER_CURRENT, feature::C_PORT_OVER_CURRENT),
            (port::C_RESET, feature::C_PORT_RESET),
        ] {
            if st.change & c != 0 {
                self.usb.control(hub::clear_port_feature(p, f), &[])?;
            }
        }
        let connected = st.status & port::CONNECTION != 0;
        let i = p as usize;
        if self.attached[i] && (!connected || st.change & port::C_CONNECTION != 0) {
            self.attached[i] = false;
            let _ = self.usb.call(op::DETACH, p as u32, 0, &[], Some(Duration::from_secs(5)));
        }
        if connected && !self.attached[i] {
            match self.attach(p, st) {
                Ok(()) => self.attached[i] = true,
                Err(e) if e.error == Error::PeerClosed => return Err(e),
                Err(e) => {
                    println!("usb-hub: port {p}: zarizeni nelze nastavit: {e}");
                    // Povoleny port s nenastavenym zarizenim: to by dal
                    // odpovidalo na vychozi adrese 0 a pletlo se do
                    // nastavovani dalsich zarizeni za hubem.
                    let _ = self
                        .usb
                        .control(hub::clear_port_feature(p, feature::PORT_ENABLE), &[]);
                }
            }
        }
        Ok(())
    }

    /// Reset portu (USB 2; port USB 3 se po pripojeni povoli sam) a ohlaseni
    /// zarizeni radici.
    fn attach(&mut self, p: u8, st: PortStatus) -> UsbResult<()> {
        // Port externiho hubu nad 15 radic neumi adresovat (route string):
        // zarizeni na nem se ani neresetuje, zustane na zakazanem portu.
        if !self.root && p > hub::MAX_ROUTE_PORT {
            return Err(Error::NotSupported.into());
        }
        let mut st = st;
        if st.status & port::ENABLE == 0 {
            self.usb.control(hub::set_port_feature(p, feature::PORT_RESET), &[])?;
            let deadline = time::deadline_after(RESET_TIMEOUT);
            loop {
                libos::time::sleep(Duration::from_millis(10));
                st = self.status(p)?;
                if st.change & port::C_RESET != 0 && st.status & port::RESET == 0 {
                    break;
                }
                if time::now() > deadline {
                    return Err(Error::TimedOut.into());
                }
            }
            self.usb.control(hub::clear_port_feature(p, feature::C_PORT_RESET), &[])?;
            if st.status & port::ENABLE == 0 {
                return Err(Error::IoError.into());
            }
        }
        libos::time::sleep(RESET_RECOVERY);
        let speed: Speed = st.speed();
        self.usb
            .call(op::ATTACH, p as u32, speed.to_u8() as u32, &[], Some(Duration::from_secs(10)))?;
        Ok(())
    }
}
