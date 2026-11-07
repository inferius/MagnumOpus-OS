//! xhci: ovladac radice USB (xHCI) a jadro USB (enumerace zarizeni).
//!
//! Spousti ho spravce zarizeni. Startovni zprava (kind USER, v tomto poradi):
//! 1. omezeny Resource (BARy a konfigurace radice, preruseni, DMA),
//! 2. stranka konfiguracniho prostoru PCI radice,
//! 3. kanal ke spravci zarizeni.
//!
//! Zpravy od spravce na bootstrap kanalu: `pci <sbernice> <zarizeni>
//! <funkce>` a `bar0 <fyzicka adresa> <velikost>` (desitkove).
//!
//! Spravci posila `usb-interface <trida> <podtrida> <protokol> <vid> <pid>`
//! (sestnactkove) s kanalem rozhrani (protokol `usbproto::msg`); spravce
//! podle tridy spusti tridni ovladac. Prvni je virtualni root hub (trida
//! 09) - jeho ovladac hubu najde zarizeni na portech a radic je pak
//! nastavi (`ATTACH`). Po prvnim pruchodu porty posle `usb-ready`.

#![no_std]
#![no_main]

extern crate alloc;

mod device;
mod hc;
mod regs;
mod ring;
mod roothub;

use alloc::format;
use alloc::vec::Vec;
use core::time::Duration;

use libos::hw::Resource;
use libos::{Channel, Error, Handle, Signals, Startup, Vmo, println, time};
use pci::Ecam;
use usbproto::desc::{Setup, request, rt};
use usbproto::msg::{self, Info, Reply, Request, op};

use hc::{Done, Hc, Owner};

libos::entry!(main);

/// Komu kanal patri.
enum Kind {
    RootHub,
    Interface { slot: u8, iface: u8, dcis: Vec<u8> },
}

struct Client {
    channel: Channel,
    kind: Kind,
}

struct Server {
    hc: Hc,
    devmgr: Channel,
    clients: Vec<Option<Client>>,
    /// Ovladac hubu ceka na zmenu portu: (klient, id, nejvic bajtu).
    root_wait: Option<(usize, u32, u32)>,
    ready_sent: bool,
}

fn main(startup: Startup) -> i64 {
    let mut handles = startup.handles.into_iter();
    let (Some(res), Some(ecam_vmo), Some(devmgr)) = (handles.next(), handles.next(), handles.next())
    else {
        println!("xhci: chybi handly ve startovni zprave");
        return 1;
    };
    let res = Resource::from_handle(res);
    let ecam_vmo = Vmo::from_handle(ecam_vmo);
    let devmgr = Channel::from_handle(devmgr);
    let Some(address) = virtio::read_driver_config(&startup.bootstrap) else {
        println!("xhci: neplatna konfigurace od spravce zarizeni");
        return 2;
    };
    let Some((bar, size)) = read_bar(&startup.bootstrap) else {
        println!("xhci: chybi adresa registru (bar0)");
        return 2;
    };
    let hc = Ecam::single(&ecam_vmo, address).and_then(|e| Hc::open(&e, address, res, bar, size));
    let hc = match hc {
        Ok(h) => h,
        Err(e) => {
            println!("xhci {address}: inicializace selhala: {e}");
            return 3;
        }
    };
    println!(
        "xhci {address}: {} portu ({} USB 3), {} slotu, kontext {} B",
        hc.ports,
        hc.usb3_ports.count_ones(),
        hc.max_slots,
        hc.ctx_size
    );
    let mut srv = Server {
        hc,
        devmgr,
        clients: Vec::new(),
        root_wait: None,
        ready_sent: false,
    };
    if srv.announce(Kind::RootHub, 0x09, 0, 0, 0, 0).is_err() {
        return 4;
    }
    srv.run();
    0
}

fn read_bar(bootstrap: &Channel) -> Option<(u64, u64)> {
    let msg = bootstrap.read(None).ok()?;
    let text = core::str::from_utf8(&msg.data).ok()?;
    let mut w = text.split_whitespace();
    if w.next()? != "bar0" {
        return None;
    }
    Some((w.next()?.parse().ok()?, w.next()?.parse().ok()?))
}

fn error_reply(id: u32, e: Error) -> Vec<u8> {
    Reply {
        id,
        status: e.code(),
        completion: 0,
        actual: 0,
        data: &[],
    }
    .encode()
}

fn ok_reply(id: u32, actual: u32, data: &[u8]) -> Vec<u8> {
    Reply {
        id,
        status: 0,
        completion: msg::completion::SUCCESS,
        actual,
        data,
    }
    .encode()
}

impl Server {
    /// Novy kanal rozhrani a zprava spravci zarizeni.
    fn announce(&mut self, kind: Kind, class: u8, sub: u8, proto: u8, vid: u16, pid: u16) -> libos::Result<()> {
        let (mine, theirs) = Channel::create()?;
        let text = format!("usb-interface {class:02x} {sub:02x} {proto:02x} {vid:04x} {pid:04x}");
        self.devmgr.write(text.as_bytes(), alloc::vec![theirs.into_handle()])?;
        self.clients.push(Some(Client { channel: mine, kind }));
        Ok(())
    }

    fn reply(&self, idx: usize, data: &[u8]) {
        if let Some(Some(c)) = self.clients.get(idx) {
            let _ = c.channel.write(data, Vec::new());
        }
    }

    fn run(&mut self) {
        loop {
            let mut items: Vec<(&Handle, Signals)> = Vec::new();
            items.push(self.hc.irq_signals());
            items.push((self.devmgr.handle(), Signals::PEER_CLOSED));
            let mut idx = Vec::new();
            for (i, c) in self.clients.iter().enumerate() {
                if let Some(c) = c {
                    items.push((c.channel.handle(), Signals::READABLE | Signals::PEER_CLOSED));
                    idx.push(i);
                }
            }
            let observed = match libos::wait_many(&items, Some(time::deadline_after(Duration::from_millis(100)))) {
                Ok((_, o)) => o,
                Err(Error::TimedOut) => Vec::new(),
                Err(e) => {
                    println!("xhci: cekani selhalo: {e}");
                    return;
                }
            };
            drop(items);
            if observed.first().is_some_and(|s| s.intersects(Signals::SIGNALED)) {
                let _ = self.hc.irq.ack();
            }
            if observed.get(1).is_some_and(|s| s.intersects(Signals::PEER_CLOSED)) {
                return;
            }
            self.hc.poll_events();
            for (k, &i) in idx.iter().enumerate() {
                let sig = observed.get(k + 2).copied().unwrap_or_default();
                if sig.intersects(Signals::READABLE) {
                    for _ in 0..32 {
                        let Some(Some(c)) = self.clients.get(i) else { break };
                        match c.channel.try_read() {
                            Ok(m) => self.request(i, &m.data),
                            Err(_) => break,
                        }
                    }
                } else if sig.intersects(Signals::PEER_CLOSED) {
                    self.clients[i] = None;
                    if self.root_wait.is_some_and(|(c, _, _)| c == i) {
                        self.root_wait = None;
                    }
                }
            }
            self.deliver();
        }
    }

    /// Doruci hotove prenosy a zmeny portu.
    fn deliver(&mut self) {
        // Endpointy zastavene chybou obnovit driv, nez klient posle dalsi
        // pozadavek.
        self.hc.recover_halted();
        let done: Vec<Done> = core::mem::take(&mut self.hc.done);
        for d in done {
            let Owner::Client(idx, id) = d.owner else {
                continue;
            };
            let data = if d.code == msg::completion::SUCCESS {
                ok_reply(id, d.actual, &d.data)
            } else {
                Reply {
                    id,
                    status: Error::IoError.code(),
                    completion: d.code,
                    actual: 0,
                    data: &[],
                }
                .encode()
            };
            self.reply(idx, &data);
        }
        if let Some((idx, id, max)) = self.root_wait {
            let map = self.hc.port_changes();
            if map.iter().any(|&b| b != 0) {
                self.root_wait = None;
                let n = map.len().min(max as usize);
                self.reply(idx, &ok_reply(id, n as u32, &map[..n]));
            }
        }
    }

    fn request(&mut self, idx: usize, raw: &[u8]) {
        let Some(r) = Request::decode(raw) else {
            return;
        };
        let id = r.id;
        let is_root = matches!(self.clients[idx].as_ref().map(|c| &c.kind), Some(Kind::RootHub));
        let result = if is_root {
            self.root_request(idx, &r)
        } else {
            self.device_request(idx, &r)
        };
        match result {
            Ok(Some(reply)) => self.reply(idx, &reply),
            Ok(None) => {} // odpoved az po dokonceni prenosu
            Err(e) => self.reply(idx, &error_reply(id, e)),
        }
    }

    fn root_request(&mut self, idx: usize, r: &Request) -> libos::Result<Option<Vec<u8>>> {
        match r.op {
            op::INFO => {
                let info = Info {
                    speed: usbproto::Speed::High,
                    interface: 0,
                    path: Vec::new(),
                    device: usbproto::desc::DeviceDescriptor::parse(&roothub::DEVICE).unwrap(),
                    config: Vec::new(),
                };
                Ok(Some(ok_reply(r.id, 0, &info.encode(&roothub::DEVICE))))
            }
            op::CONTROL => {
                let setup = Setup::from_bytes(r.data).ok_or(Error::InvalidArgs)?;
                let data = self.hc.root_control(setup)?;
                Ok(Some(ok_reply(r.id, data.len() as u32, &data)))
            }
            op::INTERRUPT_IN if r.a == usbproto::hub::STATUS_ENDPOINT as u32 => {
                if self.root_wait.is_some() {
                    return Err(Error::AlreadyExists);
                }
                self.root_wait = Some((idx, r.id, r.b));
                Ok(None)
            }
            op::ATTACH => {
                let port = u8::try_from(r.a).map_err(|_| Error::InvalidArgs)?;
                if port == 0 || port > self.hc.ports {
                    return Err(Error::InvalidArgs);
                }
                let speed = self.hc.port_speed(port);
                let slot = self.hc.enumerate(port, speed)?;
                self.start_interfaces(slot)?;
                Ok(Some(ok_reply(r.id, slot as u32, &[])))
            }
            op::DETACH => {
                let port = u8::try_from(r.a).map_err(|_| Error::InvalidArgs)?;
                self.detach(port);
                Ok(Some(ok_reply(r.id, 0, &[])))
            }
            op::SCAN_DONE => {
                if !self.ready_sent {
                    self.ready_sent = true;
                    let _ = self.devmgr.write(b"usb-ready", Vec::new());
                }
                Ok(Some(ok_reply(r.id, 0, &[])))
            }
            _ => Err(Error::NotSupported),
        }
    }

    /// Ohlasi rozhrani noveho zarizeni spravci (ten spusti tridni ovladace).
    fn start_interfaces(&mut self, slot: u8) -> libos::Result<()> {
        let dev = self.hc.devices[slot as usize].as_ref().ok_or(Error::NotFound)?;
        let (vid, pid) = (dev.descriptor.vendor, dev.descriptor.product);
        let list: Vec<(u8, u8, u8, u8, Vec<u8>)> = dev
            .interfaces()
            .iter()
            .map(|i| {
                let dcis = i.endpoints.iter().map(|e| device::dci(e.address)).collect();
                (i.number, i.class, i.subclass, i.protocol, dcis)
            })
            .collect();
        for (iface, class, sub, proto, dcis) in list {
            self.announce(Kind::Interface { slot, iface, dcis }, class, sub, proto, vid, pid)?;
        }
        Ok(())
    }

    /// Zarizeni z root portu zmizelo: zavrit kanaly rozhrani a zrusit slot.
    fn detach(&mut self, port: u8) {
        let slots: Vec<u8> = self
            .hc
            .devices
            .iter()
            .flatten()
            .filter(|d| d.path.first() == Some(&port))
            .map(|d| d.slot)
            .collect();
        for slot in slots {
            for c in self.clients.iter_mut() {
                if matches!(c, Some(Client { kind: Kind::Interface { slot: s, .. }, .. }) if *s == slot) {
                    *c = None;
                }
            }
            self.hc.disable_slot(slot);
            println!("xhci: port {port}: zarizeni odpojeno (slot {slot})");
        }
    }

    fn device_request(&mut self, idx: usize, r: &Request) -> libos::Result<Option<Vec<u8>>> {
        let Some(Client {
            kind: Kind::Interface { slot, iface, dcis },
            ..
        }) = self.clients[idx].as_ref()
        else {
            return Err(Error::BadHandle);
        };
        let (slot, iface) = (*slot, *iface);
        match r.op {
            op::INFO => {
                let dev = self.hc.devices[slot as usize].as_ref().ok_or(Error::PeerClosed)?;
                let info = Info {
                    speed: dev.speed,
                    interface: iface,
                    path: dev.path.clone(),
                    device: dev.descriptor,
                    config: dev.raw_config.clone(),
                };
                Ok(Some(ok_reply(r.id, 0, &info.encode(&dev.raw_device))))
            }
            op::CONTROL => {
                let setup = Setup::from_bytes(r.data).ok_or(Error::InvalidArgs)?;
                let out = &r.data[Setup::SIZE..];
                if !control_allowed(&setup, iface, dcis) {
                    return Err(Error::AccessDenied);
                }
                if setup.length as usize > msg::MAX_DATA
                    || (!setup.is_in() && out.len() != setup.length as usize)
                {
                    return Err(Error::InvalidArgs);
                }
                self.hc
                    .submit_control(slot, setup, out, Owner::Client(idx, r.id))?;
                Ok(None)
            }
            op::INTERRUPT_IN | op::INTERRUPT_OUT => {
                let addr = u8::try_from(r.a).map_err(|_| Error::InvalidArgs)?;
                let in_dir = r.op == op::INTERRUPT_IN;
                if (addr & 0x80 != 0) != in_dir {
                    return Err(Error::InvalidArgs);
                }
                let d = device::dci(addr);
                if !dcis.contains(&d) {
                    return Err(Error::AccessDenied);
                }
                let len = if in_dir { r.b } else { r.data.len() as u32 };
                if !in_dir {
                    let dev = self.hc.devices[slot as usize].as_ref().ok_or(Error::PeerClosed)?;
                    let ep = dev.endpoint(d).ok_or(Error::NotFound)?;
                    if r.data.len() as u64 > ep.buf.len {
                        return Err(Error::InvalidArgs);
                    }
                    ep.buf.map.write_bytes(0, r.data);
                }
                self.hc.submit_normal(slot, d, len, Owner::Client(idx, r.id))?;
                Ok(None)
            }
            _ => Err(Error::NotSupported),
        }
    }
}

/// Smi tridni ovladac rozhrani `iface` poslat kontrolni pozadavek? Ovladac
/// nesmi sahat na ostatni rozhrani zarizeni ani menit zarizeni jako celek:
/// - prijemce rozhrani: jen vlastni (`wIndex` dolni bajt),
/// - prijemce endpoint: jen endpointy vlastniho rozhrani,
/// - prijemce zarizeni, standardni pozadavek: jen cteni (IN: deskriptory,
///   stav); zapisy (adresa, konfigurace, vlastnosti) ridi radic,
/// - pozadavky tridy a vyrobce na zarizeni a "other" (porty hubu) projdou.
fn control_allowed(s: &Setup, iface: u8, dcis: &[u8]) -> bool {
    let kind = s.request_type & 0x60;
    match s.request_type & 0x1F {
        rt::TO_INTERFACE => s.index as u8 == iface,
        rt::TO_ENDPOINT => {
            let addr = s.index as u8;
            addr & 0x0F != 0 && dcis.contains(&device::dci(addr))
        }
        rt::TO_DEVICE if kind == rt::STANDARD => {
            s.is_in() && !matches!(s.request, request::SET_ADDRESS | request::SET_CONFIGURATION)
        }
        rt::TO_DEVICE | rt::TO_OTHER => true,
        _ => false,
    }
}
