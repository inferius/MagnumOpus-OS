//! Zarizeni na sbernici: kontexty, enumerace (adresa, deskriptory,
//! konfigurace) a endpointy.

use alloc::vec::Vec;

use libos::{Error, Result, println};
use usbproto::Speed;
use usbproto::desc::{Configuration, DeviceDescriptor, Interface, Setup, Transfer, kind};

use crate::hc::Hc;
use crate::ring::{Dma, Ring, Trb, trb};

/// Endpoint mimo EP0.
pub struct Endpoint {
    pub dci: u8,
    pub ring: Ring,
    /// Buffer pro data (jedna stranka).
    pub buf: Dma,
}

pub struct Device {
    pub slot: u8,
    /// Cesta porty od radice (root port, ...).
    pub path: Vec<u8>,
    pub speed: Speed,
    /// Vystupni kontext (zapisuje radic; drzet po dobu zivota slotu).
    pub _out_ctx: Dma,
    pub in_ctx: Dma,
    pub ep0: Ring,
    /// Buffer dat endpointu 0 (az 16 KiB).
    pub ep0_buf: Dma,
    pub endpoints: Vec<Endpoint>,
    pub raw_device: [u8; DeviceDescriptor::SIZE],
    pub descriptor: DeviceDescriptor,
    pub raw_config: Vec<u8>,
    pub config: Option<Configuration>,
}

impl Device {
    pub fn endpoint(&self, dci: u8) -> Option<&Endpoint> {
        self.endpoints.iter().find(|e| e.dci == dci)
    }

    pub fn endpoint_mut(&mut self, dci: u8) -> Option<&mut Endpoint> {
        self.endpoints.iter_mut().find(|e| e.dci == dci)
    }

    pub fn buffer(&self, dci: u8) -> Option<&Dma> {
        if dci == 1 {
            Some(&self.ep0_buf)
        } else {
            self.endpoint(dci).map(|e| &e.buf)
        }
    }

    /// Rozhrani zakladniho nastaveni.
    pub fn interfaces(&self) -> &[Interface] {
        self.config.as_ref().map_or(&[], |c| &c.interfaces)
    }
}

/// DCI endpointu z adresy: EP0 = 1, OUT n = 2n, IN n = 2n + 1.
pub fn dci(address: u8) -> u8 {
    let n = address & 0x0F;
    if n == 0 {
        1
    } else {
        n * 2 + (address >> 7)
    }
}

/// Id rychlosti xHCI (vychozi mapovani Protocol Speed ID).
fn speed_id(s: Speed) -> u32 {
    match s {
        Speed::Full => 1,
        Speed::Low => 2,
        Speed::High => 3,
        Speed::Super => 4,
    }
}

/// Rychlost z pole Port Speed v PORTSC.
pub fn speed_from_id(id: u32) -> Speed {
    match id {
        2 => Speed::Low,
        3 => Speed::High,
        4.. => Speed::Super,
        _ => Speed::Full,
    }
}

/// Interval endpointu xHCI (perioda 2^n x 125 us) z `bInterval`.
fn interval(speed: Speed, transfer: Transfer, b_interval: u8) -> u32 {
    match (speed, transfer) {
        (Speed::High | Speed::Super, _) => (b_interval.clamp(1, 16) - 1) as u32,
        // Low/full speed: bInterval v ms -> ms * 8 mikrorámců, log2.
        (_, Transfer::Interrupt) => {
            let frames = (b_interval.max(1) as u32) * 8;
            (31 - frames.leading_zeros()).clamp(3, 10)
        }
        _ => 0,
    }
}

impl Hc {
    /// Offset kontextu v kontextu vstupu: 0 = kontrolni, 1 = slot, 1 + dci
    /// = endpoint.
    fn in_off(&self, index: u64) -> u64 {
        index * self.ctx_size
    }

    /// Nastavi zarizeni na root portu `port` (po resetu portu): slot,
    /// adresa, deskriptory, konfigurace a endpointy jeho rozhrani.
    pub fn enumerate(&mut self, port: u8, speed: Speed) -> Result<u8> {
        let ev = self.command(Trb {
            param: 0,
            status: 0,
            control: trb::ENABLE_SLOT << 10,
        })?;
        let slot = ev.slot();
        if slot == 0 || slot as usize >= self.devices.len() {
            return Err(Error::NoResources);
        }
        match self.setup_device(slot, port, speed) {
            Ok(()) => Ok(slot),
            Err(e) => {
                self.disable_slot(slot);
                Err(e)
            }
        }
    }

    fn setup_device(&mut self, slot: u8, port: u8, speed: Speed) -> Result<()> {
        let ctx = self.ctx_size;
        let out_ctx = Dma::new(&self.res, 32 * ctx)?;
        let in_ctx = Dma::new(&self.res, 33 * ctx)?;
        let ep0 = Ring::new(&self.res)?;
        let ep0_buf = Dma::new(&self.res, 16 * 1024)?;
        self.set_dcbaa(slot, out_ctx.phys);

        // Kontext vstupu: pridat slot (A0) a EP0 (A1).
        in_ctx.map.write::<u32>(4, 0b11);
        let s = self.in_off(1);
        // Slot: route string 0 (root port), rychlost, 1 polozka kontextu.
        in_ctx.map.write::<u32>(s, speed_id(speed) << 20 | 1 << 27);
        in_ctx.map.write::<u32>(s + 4, (port as u32) << 16);
        let e = self.in_off(2);
        let mps = speed.default_ep0_packet() as u32;
        // EP0: CErr 3, typ Control (4), max. paket.
        in_ctx.map.write::<u32>(e + 4, 3 << 1 | 4 << 3 | mps << 16);
        in_ctx.map.write::<u64>(e + 8, ep0.start());
        in_ctx.map.write::<u32>(e + 16, 8);

        self.devices[slot as usize] = Some(Device {
            slot,
            path: alloc::vec![port],
            speed,
            _out_ctx: out_ctx,
            in_ctx,
            ep0,
            ep0_buf,
            endpoints: Vec::new(),
            raw_device: [0; DeviceDescriptor::SIZE],
            descriptor: DeviceDescriptor::default(),
            raw_config: Vec::new(),
            config: None,
        });
        let in_phys = self.devices[slot as usize].as_ref().unwrap().in_ctx.phys;
        self.command(Trb {
            param: in_phys,
            status: 0,
            control: trb::ADDRESS_DEVICE << 10 | (slot as u32) << 24,
        })?;

        // Prvnich 8 B deskriptoru: skutecna velikost paketu EP0.
        let head = self.control_sync(slot, Setup::get_descriptor(kind::DEVICE, 0, 8), &[])?;
        let real_mps = *head.get(7).ok_or(Error::Corrupted)? as u32;
        let real_mps = if speed == Speed::Super { 1 << real_mps.min(9) } else { real_mps };
        if real_mps != mps && real_mps >= 8 {
            self.update_ep0_packet(slot, real_mps)?;
        }
        let raw = self.control_sync(
            slot,
            Setup::get_descriptor(kind::DEVICE, 0, DeviceDescriptor::SIZE as u16),
            &[],
        )?;
        let descriptor = DeviceDescriptor::parse(&raw).ok_or(Error::Corrupted)?;
        let head = self.control_sync(slot, Setup::get_descriptor(kind::CONFIGURATION, 0, 9), &[])?;
        let total = Configuration::total_length(&head)
            .ok_or(Error::Corrupted)?
            .min(16 * 1024);
        let raw_config =
            self.control_sync(slot, Setup::get_descriptor(kind::CONFIGURATION, 0, total), &[])?;
        let config = Configuration::parse(&raw_config).ok_or(Error::Corrupted)?;
        self.control_sync(slot, Setup::set_configuration(config.value), &[])?;
        {
            let dev = self.devices[slot as usize].as_mut().unwrap();
            dev.raw_device.copy_from_slice(&raw[..DeviceDescriptor::SIZE]);
            dev.descriptor = descriptor;
            dev.raw_config = raw_config;
        }
        self.configure_endpoints(slot, &config)?;
        self.devices[slot as usize].as_mut().unwrap().config = Some(config);
        println!(
            "xhci: port {port}: zarizeni {:04x}:{:04x} ({}), slot {slot}",
            descriptor.vendor,
            descriptor.product,
            speed.name()
        );
        Ok(())
    }

    /// Evaluate Context: nova velikost paketu EP0.
    fn update_ep0_packet(&mut self, slot: u8, mps: u32) -> Result<()> {
        let e = self.in_off(2);
        let phys = {
            let dev = self.devices[slot as usize].as_ref().ok_or(Error::NotFound)?;
            dev.in_ctx.map.write::<u32>(0, 0);
            dev.in_ctx.map.write::<u32>(4, 0b10);
            let w = dev.in_ctx.map.read::<u32>(e + 4);
            dev.in_ctx
                .map
                .write::<u32>(e + 4, (w & 0xFFFF) | mps << 16);
            dev.in_ctx.phys
        };
        self.command(Trb {
            param: phys,
            status: 0,
            control: trb::EVALUATE_CTX << 10 | (slot as u32) << 24,
        })
        .map(|_| ())
    }

    /// Configure Endpoint pro interrupt endpointy vsech rozhrani (bulk
    /// a isochronni zatim ne - tridy, ktere je potrebuji, prijdou pozdeji).
    fn configure_endpoints(&mut self, slot: u8, config: &Configuration) -> Result<()> {
        let speed = self.devices[slot as usize].as_ref().ok_or(Error::NotFound)?.speed;
        let mut eps = Vec::new();
        for i in &config.interfaces {
            for e in &i.endpoints {
                if e.transfer == Transfer::Interrupt {
                    eps.push(*e);
                }
            }
        }
        if eps.is_empty() {
            return Ok(());
        }
        let mut add = 1u32; // slot
        let mut max_dci = 1;
        let mut new_eps = Vec::new();
        for e in &eps {
            let d = dci(e.address);
            let ring = Ring::new(&self.res)?;
            let buf = Dma::new(&self.res, 4096)?;
            let off = self.in_off(1 + d as u64);
            let ep_type = if e.is_in() { 7 } else { 3 };
            let mps = e.max_packet as u32;
            let dev = self.devices[slot as usize].as_ref().unwrap();
            let m = &dev.in_ctx.map;
            m.write::<u32>(off, interval(speed, e.transfer, e.interval) << 16);
            m.write::<u32>(off + 4, 3 << 1 | ep_type << 3 | mps << 16);
            m.write::<u64>(off + 8, ring.start());
            // Prumerna delka TRB a max. payload za interval = velikost paketu.
            m.write::<u32>(off + 16, mps | mps << 16);
            add |= 1 << d;
            max_dci = max_dci.max(d as u32);
            new_eps.push(Endpoint { dci: d, ring, buf });
        }
        let phys = {
            let dev = self.devices[slot as usize].as_mut().unwrap();
            let m = &dev.in_ctx.map;
            m.write::<u32>(0, 0);
            m.write::<u32>(4, add);
            // Slot: pocet polozek kontextu = nejvyssi DCI.
            let s = self.ctx_size;
            let w = m.read::<u32>(s);
            m.write::<u32>(s, (w & !(0x1F << 27)) | max_dci << 27);
            dev.endpoints.extend(new_eps);
            dev.in_ctx.phys
        };
        self.command(Trb {
            param: phys,
            status: 0,
            control: trb::CONFIGURE_EP << 10 | (slot as u32) << 24,
        })
        .map(|_| ())
    }

    /// Zrusi zarizeni (odpojeni nebo chyba enumerace).
    pub fn disable_slot(&mut self, slot: u8) {
        self.cancel_device(slot);
        let _ = self.command(Trb {
            param: 0,
            status: 0,
            control: trb::DISABLE_SLOT << 10 | (slot as u32) << 24,
        });
        self.set_dcbaa(slot, 0);
        if let Some(d) = self.devices.get_mut(slot as usize) {
            *d = None;
        }
    }
}
