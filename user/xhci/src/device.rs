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
    /// Buffer pro data: interrupt jedna stranka, bulk 64 KiB, z nich se
    /// pouziva okno [`Endpoint::window`].
    pub buf: Dma,
    /// Okno v `buf` pro jeden prenos: (offset, delka). Jeden TRB nesmi
    /// prekrocit hranici 64 KiB (xHCI 4.11.7.1), proto bulk okno 32 KiB
    /// lezi v bufferu 64 KiB tak, aby hranici neprekrocilo.
    pub window: (u64, u64),
}

/// Velikost bufferu bulk endpointu (dvojnasobek okna).
pub const BULK_BUF: u64 = 2 * usbproto::msg::MAX_DATA as u64;
/// Buffer kontrolnich prenosu EP0 (deskriptory, pozadavky tridy).
pub const EP0_BUF: u64 = 16 * 1024;
const _: () = assert!(usbproto::msg::MAX_DATA <= 32 * 1024, "bulk okno musi byt <= 32 KiB (window_offset)");

/// Okno `len` bajtu v souvislem bufferu od fyzicke adresy `phys` o delce
/// `2 * len`, ktere neprekroci hranici 64 KiB (`len` <= 32 KiB). Vraci
/// offset okna v bufferu.
pub fn window_offset(phys: u64, len: u64) -> u64 {
    const B: u64 = 64 * 1024;
    // Vzdalenost k prvni hranici za zacatkem bufferu.
    let gap = (phys / B + 1) * B - phys;
    if gap >= len {
        0
    } else {
        // Hranice lezi v prvni polovine: okno zacne na ni (za ni je
        // aspon `len` bajtu, protoze `gap < len`).
        gap
    }
}

/// Kde zarizeni visi: cesta, route string a transakcni prekladac (TT).
#[derive(Debug, Clone)]
pub struct Location {
    /// Cesta porty od radice: root port, port 1. hubu, ...
    pub path: Vec<u8>,
    /// Route string xHCI: port na kazde urovni hubu po 4 bitech (bez root
    /// portu; port hubu nad 15 nejde adresovat).
    pub route: u32,
    /// Zarizeni low/full speed za hubem high speed: (slot hubu s TT,
    /// port na nem, hub ma TT na kazdy port).
    pub tt: Option<(u8, u8, bool)>,
}

impl Location {
    pub fn root(port: u8) -> Self {
        Self {
            path: alloc::vec![port],
            route: 0,
            tt: None,
        }
    }

    pub fn root_port(&self) -> u8 {
        self.path[0]
    }
}

/// Udaje hubu pro kontext slotu.
#[derive(Debug, Clone, Copy)]
pub struct HubInfo {
    pub ports: u8,
    /// TT na kazdy port (protokol zarizeni 2).
    pub multi_tt: bool,
    /// TT think time (bity 6:5 wHubCharacteristics).
    pub think_time: u8,
}

pub struct Device {
    pub slot: u8,
    pub location: Location,
    pub speed: Speed,
    /// Zarizeni je hub (trida 09).
    pub hub: Option<HubInfo>,
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

    /// Offset okna dat endpointu v jeho bufferu (EP0 a interrupt 0).
    pub fn window_offset(&self, dci: u8) -> u64 {
        self.endpoint(dci).map_or(0, |e| e.window.0)
    }

    /// Rozhrani v pouzivanem nastaveni (u hubu s TT na kazdy port
    /// alternativa 1, jinak zakladni).
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

    /// Umisteni zarizeni na portu `port` hubu ve slotu `hub` (`None` =
    /// root port): cesta, route string a TT.
    pub fn location(&self, hub: Option<u8>, port: u8, speed: Speed) -> Result<Location> {
        let Some(h) = hub else {
            return Ok(Location::root(port));
        };
        let parent = self.devices[h as usize].as_ref().ok_or(Error::NotFound)?;
        if parent.hub.is_none() {
            return Err(Error::NotSupported);
        }
        // Nejvys 5 urovni hubu a port nejvys 15 (4 bity na uroven route
        // stringu; zkraceni by oslovilo jine zarizeni).
        let tier = parent.location.path.len() - 1;
        let route =
            usbproto::hub::route(parent.location.route, tier, port).ok_or(Error::NotSupported)?;
        let mut path = parent.location.path.clone();
        path.push(port);
        // TT: low/full speed za hubem high speed; za hubem, ktery sam visi
        // za TT, plati TT rodice.
        let tt = match (speed, parent.speed) {
            (Speed::Low | Speed::Full, Speed::High) => {
                let multi = parent.hub.is_some_and(|i| i.multi_tt);
                Some((h, port, multi))
            }
            _ => parent.location.tt,
        };
        Ok(Location { path, route, tt })
    }

    /// Nastavi zarizeni na portu (po resetu portu): slot, adresa,
    /// deskriptory, konfigurace a endpointy jeho rozhrani.
    pub fn enumerate(&mut self, loc: Location, speed: Speed) -> Result<u8> {
        let ev = self.command(Trb {
            param: 0,
            status: 0,
            control: trb::ENABLE_SLOT << 10,
        })?;
        let slot = ev.slot();
        if slot == 0 || slot as usize >= self.devices.len() {
            return Err(Error::NoResources);
        }
        match self.setup_device(slot, loc, speed) {
            Ok(()) => Ok(slot),
            Err(e) => {
                self.disable_slot(slot);
                Err(e)
            }
        }
    }

    fn setup_device(&mut self, slot: u8, loc: Location, speed: Speed) -> Result<()> {
        let port = loc.root_port();
        let ctx = self.ctx_size;
        let out_ctx = Dma::new(&self.res, 32 * ctx)?;
        let in_ctx = Dma::new(&self.res, 33 * ctx)?;
        let ep0 = Ring::new(&self.res)?;
        let ep0_buf = Dma::new(&self.res, EP0_BUF)?;
        self.set_dcbaa(slot, out_ctx.phys);

        // Kontext vstupu: pridat slot (A0) a EP0 (A1).
        in_ctx.map.write::<u32>(4, 0b11);
        let s = self.in_off(1);
        // Slot: route string, rychlost, 1 polozka kontextu; root port; TT
        // (slot hubu, port, MTT).
        let mtt = loc.tt.is_some_and(|(_, _, m)| m);
        in_ctx.map.write::<u32>(s, loc.route | speed_id(speed) << 20 | (mtt as u32) << 25 | 1 << 27);
        in_ctx.map.write::<u32>(s + 4, (port as u32) << 16);
        if let Some((hub_slot, hub_port, _)) = loc.tt {
            in_ctx.map.write::<u32>(s + 8, hub_slot as u32 | (hub_port as u32) << 8);
        }
        let e = self.in_off(2);
        let mps = speed.default_ep0_packet() as u32;
        // EP0: CErr 3, typ Control (4), max. paket.
        in_ctx.map.write::<u32>(e + 4, 3 << 1 | 4 << 3 | mps << 16);
        in_ctx.map.write::<u64>(e + 8, ep0.start());
        in_ctx.map.write::<u32>(e + 16, 8);

        self.devices[slot as usize] = Some(Device {
            slot,
            location: loc.clone(),
            speed,
            hub: None,
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
        {
            let dev = self.devices[slot as usize].as_mut().unwrap();
            dev.raw_device.copy_from_slice(&raw[..DeviceDescriptor::SIZE]);
            dev.descriptor = descriptor;
            dev.raw_config = raw_config;
        }
        // xHCI 4.3.5: endpointy konfigurace (Configure Endpoint) pred
        // SET_CONFIGURATION, aby radic mel pro zarizeni prostredky driv,
        // nez je zacne pouzivat.
        self.configure_endpoints(slot, Some(&config))?;
        self.control_sync(slot, Setup::set_configuration(config.value), &[])?;
        let config = if descriptor.class == usbproto::desc::class::HUB {
            self.configure_hub(slot, &descriptor, config)?
        } else {
            config
        };
        self.devices[slot as usize].as_mut().unwrap().config = Some(config);
        println!(
            "xhci: port {:?}: zarizeni {:04x}:{:04x} ({}), slot {slot}",
            loc.path,
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

    /// Hub po SET_CONFIGURATION (jeho deskriptor jde cist az v nastavenem
    /// zarizeni): pocet portu, TT think time a MTT do kontextu slotu (radic
    /// podle nich smeruje pakety zarizeni za hubem). Vraci konfiguraci,
    /// ktera plati (s pouzitou alternativou rozhrani).
    ///
    /// Hub s TT na kazdy port (zarizeni s protokolem 2) ho zapne az
    /// alternativou 1 rozhrani hubu (USB 2.0 11.23.1); bez ni je to hub
    /// s jednim TT. Poradi podle xHCI 4.6.6: nejdriv Configure Endpoint
    /// s endpointy alternativy a MTT, pak SET_INTERFACE. Kdyz hub
    /// SET_INTERFACE odmitne, endpointy a kontext slotu se vrati na
    /// zakladni nastaveni bez MTT. Hub bez alternativy dostane jen novy
    /// kontext slotu (Configure Endpoint bez zmeny endpointu, xHCI 6.2.2.2).
    fn configure_hub(
        &mut self,
        slot: u8,
        descriptor: &DeviceDescriptor,
        config: Configuration,
    ) -> Result<Configuration> {
        let d = self.control_sync(slot, usbproto::hub::get_hub_descriptor(16), &[])?;
        let h = usbproto::hub::HubDescriptor::parse(&d).ok_or(Error::Corrupted)?;
        self.devices[slot as usize].as_mut().ok_or(Error::NotFound)?.hub = Some(HubInfo {
            ports: h.ports,
            multi_tt: false,
            think_time: ((h.characteristics >> 5) & 3) as u8,
        });
        let multi = config
            .interfaces
            .iter()
            .find(|i| i.class == usbproto::desc::class::HUB)
            .filter(|_| descriptor.protocol == 2)
            .and_then(|i| config.with_alternate(i.number, 1).map(|c| (i.number, c)))
            .filter(|(n, c)| c.interfaces.iter().any(|i| i.number == *n && i.protocol == 2));
        let Some((number, alt)) = multi else {
            self.configure_endpoints(slot, None)?;
            return Ok(config);
        };
        self.set_multi_tt(slot, true);
        self.configure_endpoints(slot, Some(&alt))?;
        if self
            .control_sync(slot, Setup::set_interface(number, 1), &[])
            .is_ok()
        {
            return Ok(alt);
        }
        println!("xhci: hub ve slotu {slot} odmitl TT na kazdy port, pouzije jeden TT");
        self.set_multi_tt(slot, false);
        self.configure_endpoints(slot, Some(&config))?;
        Ok(config)
    }

    fn set_multi_tt(&mut self, slot: u8, on: bool) {
        if let Some(Some(d)) = self.devices.get_mut(slot as usize)
            && let Some(h) = &mut d.hub
        {
            h.multi_tt = on;
        }
    }

    /// Configure Endpoint pro interrupt a bulk endpointy vsech rozhrani
    /// (isochronni zatim ne - zvuk USB prijde pozdeji).
    /// Lze volat znovu (zmena alternativy): drive nastavene endpointy se
    /// zrusi (drop) a nahradi novymi. `None` = endpointy beze zmeny, jen
    /// novy kontext slotu (udaje hubu).
    fn configure_endpoints(&mut self, slot: u8, config: Option<&Configuration>) -> Result<()> {
        let speed = self.devices[slot as usize].as_ref().ok_or(Error::NotFound)?.speed;
        let mut eps = Vec::new();
        for i in config.map_or(&[][..], |c| &c.interfaces) {
            for e in &i.endpoints {
                if matches!(e.transfer, Transfer::Interrupt | Transfer::Bulk) {
                    eps.push(*e);
                }
            }
        }
        let hub = self.devices[slot as usize].as_ref().and_then(|d| d.hub);
        let current = self.devices[slot as usize]
            .as_ref()
            .map_or(0u32, |d| d.endpoints.iter().fold(0, |m, e| m | 1 << e.dci));
        // Drive nastavene endpointy (opakovane volani) se zrusi; bez
        // konfigurace zustanou.
        let drop = if config.is_some() { current } else { 0 };
        if config.is_some() && eps.is_empty() && hub.is_none() && drop == 0 {
            return Ok(());
        }
        let mut add = 1u32; // slot
        // Pocet polozek kontextu = nejvyssi DCI platnych endpointu.
        let mut max_dci = if config.is_some() {
            1
        } else {
            (31 - current.leading_zeros().min(31)).max(1)
        };
        let mut new_eps = Vec::new();
        for e in &eps {
            let d = dci(e.address);
            let ring = Ring::new(&self.res)?;
            let bulk = e.transfer == Transfer::Bulk;
            let (buf, window) = if bulk {
                let buf = Dma::new(&self.res, BULK_BUF)?;
                let len = usbproto::msg::MAX_DATA as u64;
                let off = window_offset(buf.phys, len);
                (buf, (off, len))
            } else {
                (Dma::new(&self.res, 4096)?, (0, 4096))
            };
            let off = self.in_off(1 + d as u64);
            // Typ endpointu (xHCI 6.2.3): 2/6 bulk OUT/IN, 3/7 interrupt.
            let ep_type = match (bulk, e.is_in()) {
                (true, false) => 2,
                (true, true) => 6,
                (false, false) => 3,
                (false, true) => 7,
            };
            let mps = e.max_packet as u32;
            let dev = self.devices[slot as usize].as_ref().unwrap();
            let m = &dev.in_ctx.map;
            // Bulk: interval 0, prumerna delka TRB 3 KiB (doporuceni xHCI
            // 4.14.1.1), bez payloadu za interval. Interrupt: interval
            // a payload = velikost paketu.
            let ival = if bulk { 0 } else { interval(speed, e.transfer, e.interval) };
            m.write::<u32>(off, ival << 16);
            m.write::<u32>(off + 4, 3 << 1 | ep_type << 3 | mps << 16);
            m.write::<u64>(off + 8, ring.start());
            m.write::<u32>(off + 16, if bulk { 3072 } else { mps | mps << 16 });
            add |= 1 << d;
            max_dci = max_dci.max(d as u32);
            new_eps.push(Endpoint { dci: d, ring, buf, window });
        }
        let phys = {
            let dev = self.devices[slot as usize].as_mut().unwrap();
            let m = &dev.in_ctx.map;
            m.write::<u32>(0, drop);
            m.write::<u32>(4, add);
            // Slot: pocet polozek kontextu = nejvyssi DCI; u hubu priznak
            // hubu, MTT, pocet portu a TT think time.
            let s = self.ctx_size;
            let mut w = (m.read::<u32>(s) & !(0x1F << 27)) | max_dci << 27;
            if let Some(h) = hub {
                // Priznak hubu (26) a MTT (25) nastavit i shodit.
                w = (w & !(3 << 25)) | 1 << 26 | (h.multi_tt as u32) << 25;
                let w1 = m.read::<u32>(s + 4);
                m.write::<u32>(s + 4, (w1 & 0x00FF_FFFF) | (h.ports as u32) << 24);
                let w2 = m.read::<u32>(s + 8);
                m.write::<u32>(s + 8, (w2 & !(3 << 16)) | (h.think_time as u32) << 16);
            }
            m.write::<u32>(s, w);
            dev.in_ctx.phys
        };
        self.command(Trb {
            param: phys,
            status: 0,
            control: trb::CONFIGURE_EP << 10 | (slot as u32) << 24,
        })?;
        if config.is_some()
            && let Some(Some(dev)) = self.devices.get_mut(slot as usize)
        {
            dev.endpoints = new_eps;
        }
        Ok(())
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
