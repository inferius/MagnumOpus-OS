//! hda: ovladac radice Intel High Definition Audio (PCI trida 04:03).
//!
//! Spousti ho spravce zarizeni. Startovni zprava (kind USER, v tomto
//! poradi): omezeny Resource (BARy a konfigurace teto funkce, kvoty
//! preruseni a DMA), stranka konfiguracniho prostoru PCI, kanal ke
//! zvukove sluzbe (protokol `audioproto::device`). Prvni zprava na
//! bootstrap kanalu: `pci <sbernice> <zarizeni> <funkce>`.
//!
//! Postup: reset radice, CORB/RIRB, vycet kodeku a widgetu, cesty
//! k vystupum (`hdaspec::graph`), pak obsluha pozadavku `audiod`
//! a preruseni streamu. Preruseni je MSI; kdyz ho zarizeni nema, cte se
//! pozice casovacem (dvakrat za periodu). Detekce zasunuti konektoru se
//! cte kazdych [`JACK_POLL`] (nevyzadane odpovedi zatim ne).
//!
//! Vystup jde jen pres analogove piny; HDMI/DP se najde a nahlasi, ale
//! nenastavuje (chybi ELD a infoframe).

#![no_std]
#![no_main]

extern crate alloc;

mod codec;
mod ctrl;
mod stream;

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use core::time::Duration;

use audioproto::device::{self, Info, OpenStream, op};
use audioproto::{Error, Format, Result, Sample, encode_reply};
use hdaspec::regs;
use libos::abi::vm::{MAP_READ, MAP_WRITE};
use libos::hw::{Interrupt, Resource};
use libos::{Channel, Mapping, Signals, Startup, Vmo, println, time};
use pci::{Address, Ecam};

use codec::Codec;
use ctrl::Ctrl;
use stream::OutStream;

libos::entry!(main);

/// Velikost registru radice (BAR0) podle specifikace.
const MMIO_SIZE: u64 = 0x4000;
/// Jak casto cist detekci zasunuti.
const JACK_POLL: Duration = Duration::from_millis(500);
/// Cislo streamu (tag) vystupu; 0 je rezervovana.
const OUT_TAG: u8 = 1;

/// PCI Express capability: Device Control, bit 11 = smi zarizeni pouzit
/// "no snoop" (DMA mimo koherenci cache).
const CAP_PCIE: u8 = 0x10;
const DEVCTL_NOSNOOP: u32 = 1 << 11;

struct Driver {
    ecam: Ecam,
    address: Address,
    res: Resource,
    ctrl: Ctrl,
    codec: Option<Codec>,
    /// Ostatni kodeky (HDMI apod.) jen pro hlaseni.
    others: Vec<Codec>,
    irq: Option<Interrupt>,
    out: Option<OutStream>,
    name: String,
}

fn main(startup: Startup) -> i64 {
    let mut handles = startup.handles.into_iter();
    let (Some(res), Some(cfg), Some(client)) = (handles.next(), handles.next(), handles.next()) else {
        println!("hda: chybi handly ve startovni zprave");
        return 1;
    };
    let res = Resource::from_handle(res);
    let cfg = Vmo::from_handle(cfg);
    let client = Channel::from_handle(client);
    let Some(address) = pci::read_driver_config(&startup.bootstrap) else {
        println!("hda: neplatna konfigurace od spravce zarizeni");
        return 2;
    };
    let mut drv = match Driver::open(&cfg, address, res) {
        Ok(d) => d,
        Err(e) => {
            println!("hda {address}: inicializace selhala: {e}");
            return 3;
        }
    };
    drv.serve(&client);
    drv.shutdown();
    0
}

impl Driver {
    fn open(cfg: &Vmo, address: Address, res: Resource) -> Result<Self> {
        let ecam = Ecam::single(cfg, address)?;
        let vendor = ecam.read16(address, pci::VENDOR_ID);
        let device = ecam.read16(address, pci::VENDOR_ID + 2);
        let bar = ecam.memory_bar(address, 0).ok_or(Error::NotSupported)?;
        quirks(&ecam, address, vendor);
        ecam.enable(address);
        let vmo = res.device_memory(bar.phys, MMIO_SIZE)?;
        let mmio = Mapping::new(&vmo, 0, MMIO_SIZE, MAP_READ | MAP_WRITE)?;
        let mut ctrl = Ctrl::open(mmio, &res)?;
        // Preruseni MSI; bez nej se pozice cte casovacem.
        let irq = match res.interrupt() {
            Ok(i) => match pci::msi::enable(&ecam, address, i.msi()) {
                Ok(()) => Some(i),
                Err(e) => {
                    println!("hda {address}: MSI nejde ({e}), pozici budu cist casovacem");
                    None
                }
            },
            Err(e) => {
                println!("hda {address}: preruseni nejde vytvorit ({e})");
                None
            }
        };
        let mut codecs = Vec::new();
        for addr in 0..15u8 {
            if ctrl.codecs & 1 << addr == 0 {
                continue;
            }
            match Codec::probe(&mut ctrl, addr) {
                Ok(Some(c)) => {
                    println!("hda {address}: {}", c.describe());
                    codecs.push(c);
                }
                Ok(None) => println!("hda {address}: kodek {addr} nema zvukovou funkci (modem)"),
                Err(e) => println!("hda {address}: kodek {addr}: {e}"),
            }
        }
        // Prvni kodek s analogovym vystupem hraje; ostatni jen hlasime.
        let main = codecs.iter().position(|c| c.has_output()).map(|i| codecs.remove(i));
        let mut codec = main;
        if let Some(c) = codec.as_mut() {
            c.setup_outputs(&mut ctrl)?;
        }
        let name = alloc::format!(
            "HDA {vendor:04x}:{device:04x}{}",
            codec.as_ref().map_or(String::new(), |c| alloc::format!(" kodek {:08x}", c.vendor))
        );
        println!(
            "hda {address}: {name}, {} vystupnich streamu, {}",
            ctrl.gcap.output_streams,
            if irq.is_some() { "MSI" } else { "bez preruseni" }
        );
        Ok(Self { ecam, address, res, ctrl, codec, others: codecs, irq, out: None, name })
    }

    /// Format zarizeni: 48 kHz (nebo 44,1 kHz), 16 bitu, stereo.
    fn device_format(&self) -> Format {
        let rate = match &self.codec {
            Some(c) if c.pcm.raw != 0 && !c.pcm.rate(48_000) && c.pcm.rate(44_100) => 44_100,
            _ => 48_000,
        };
        Format::new(rate, 2, Sample::S16)
    }

    fn info(&self) -> Info {
        let mut outputs = self.codec.as_ref().map_or(Vec::new(), |c| c.outputs());
        for c in &self.others {
            outputs.extend(c.outputs());
        }
        Info { name: self.name.clone(), outputs, format: self.device_format(), input: false }
    }

    fn open_output(&mut self, want: OpenStream) -> Result<(Vec<u8>, Vec<libos::Handle>)> {
        if self.codec.is_none() || self.ctrl.gcap.output_streams == 0 {
            return Err(Error::NotFound);
        }
        if want.format != self.device_format() {
            return Err(Error::NotSupported);
        }
        if let Some(mut old) = self.out.take() {
            old.close(&self.ctrl);
        }
        let fmt = hdaspec::format::encode(want.format.rate, 16, 2).ok_or(Error::NotSupported)?;
        let index = self.ctrl.gcap.first_output();
        let s = OutStream::open(&self.ctrl, &self.res, index, OUT_TAG, fmt, want)?;
        let handles = vec![s.buf.handle().duplicate(None)?, s.status_vmo.handle().duplicate(None)?, s.event.handle().duplicate(None)?];
        let reply = device::encode_opened(&s.req);
        self.out = Some(s);
        Ok((reply, handles))
    }

    fn start(&mut self) -> Result<()> {
        let polled = self.irq.is_none();
        let (Some(s), Some(c)) = (self.out.as_mut(), self.codec.as_ref()) else { return Err(Error::BadState) };
        c.set_stream(&mut self.ctrl, s.tag, s.fmt)?;
        s.start(&self.ctrl, polled);
        Ok(())
    }

    fn stop(&mut self) -> Result<()> {
        let Some(s) = self.out.as_mut() else { return Err(Error::BadState) };
        s.stop(&self.ctrl);
        if let Some(c) = self.codec.as_ref() {
            let _ = c.set_stream(&mut self.ctrl, 0, 0);
        }
        Ok(())
    }

    fn handle(&mut self, m: &[u8]) -> (Result<Vec<u8>>, Vec<libos::Handle>) {
        let mut r = wire::Reader::new(m);
        let Ok(code) = r.u32() else { return (Err(Error::InvalidArgs), Vec::new()) };
        match code {
            op::INFO => (Ok(self.info().encode()), Vec::new()),
            op::OPEN_OUTPUT => match OpenStream::decode(&mut r).and_then(|w| self.open_output(w)) {
                Ok((reply, hs)) => (Ok(reply), hs),
                Err(e) => (Err(e), Vec::new()),
            },
            op::START => (self.start().map(|_| Vec::new()), Vec::new()),
            op::STOP => (self.stop().map(|_| Vec::new()), Vec::new()),
            op::CLOSE => {
                if let Some(mut s) = self.out.take() {
                    s.close(&self.ctrl);
                }
                (Ok(Vec::new()), Vec::new())
            }
            // Nahravani je v protokolu; ovladac ho zatim neumi.
            op::OPEN_INPUT => (Err(Error::NotSupported), Vec::new()),
            _ => (Err(Error::NotSupported), Vec::new()),
        }
    }

    fn serve(&mut self, client: &Channel) {
        let mut next_jack = time::deadline_after(JACK_POLL);
        loop {
            let running = self.out.as_ref().is_some_and(|s| s.running);
            // Bez preruseni: cist pozici dvakrat za periodu. S MSI jednou za
            // dve periody jako zaloha (MSI zapnute, ale nedorucene).
            let poll = match (&self.irq, &self.out) {
                (None, Some(s)) if running => Some(time::deadline_after(Duration::from_nanos(s.period_ns() / 2))),
                (Some(_), Some(s)) if running => Some(time::deadline_after(Duration::from_nanos(s.period_ns() * 2))),
                _ => None,
            };
            let jack = self.codec.as_ref().is_some_and(|c| c.has_jack_detect()).then_some(next_jack);
            let deadline = match (poll, jack) {
                (Some(a), Some(b)) => Some(a.min(b)),
                (a, b) => a.or(b),
            };
            let mut items = vec![(client.handle(), Signals::READABLE | Signals::PEER_CLOSED)];
            if let Some(i) = &self.irq {
                items.push((i.handle(), Signals::SIGNALED));
            }
            let woke = libos::wait_many(&items, deadline);
            drop(items);
            let observed = match woke {
                Ok((_, o)) => o,
                Err(libos::Error::TimedOut) => Vec::new(),
                Err(e) => {
                    println!("hda: chyba cekani: {e}");
                    return;
                }
            };
            if observed.get(1).is_some_and(|s| s.intersects(Signals::SIGNALED)) {
                if let Some(i) = &self.irq {
                    let _ = i.ack();
                }
                // Stav streamu se cte vzdy (INTSTS muze byt mezitim smazany).
                let _ = self.ctrl.r32(regs::INTSTS);
                if let Some(s) = self.out.as_mut() {
                    s.interrupt(&self.ctrl);
                }
            }
            if let Some(sig) = observed.first() {
                if sig.intersects(Signals::READABLE) {
                    while let Ok(m) = client.try_read() {
                        let (r, hs) = self.handle(&m.data);
                        if client.write(&encode_reply(r), hs).is_err() {
                            return;
                        }
                    }
                } else if sig.intersects(Signals::PEER_CLOSED) {
                    println!("hda {}: zvukova sluzba zavrela kanal", self.address);
                    return;
                }
            }
            if let Some(s) = self.out.as_mut() {
                s.poll(&self.ctrl);
            }
            if time::now() >= next_jack {
                next_jack = time::deadline_after(JACK_POLL);
                if let Some(c) = self.codec.as_mut() {
                    c.poll_jacks(&mut self.ctrl, false);
                }
            }
        }
    }

    fn shutdown(&mut self) {
        if let Some(mut s) = self.out.take() {
            s.close(&self.ctrl);
        }
        self.ctrl.shutdown();
        pci::msi::disable(&self.ecam, self.address);
    }
}

/// Upravy konfigurace PCI pred startem (podle Linuxu `snd_hda_intel`):
/// DMA vzdy koherentni s cache (nase pamet DMA je cachovana), u Intelu
/// trida provozu TC0 (jinak na nekterych cipsetech praskani).
fn quirks(ecam: &Ecam, a: Address, vendor: u16) {
    // PCIe Device Control: zakazat "no snoop".
    if let Some((cap, _)) = ecam.capabilities(a).into_iter().find(|&(_, id)| id == CAP_PCIE) {
        let reg = cap + 8;
        let v = ecam.read32(a, reg);
        if v & DEVCTL_NOSNOOP != 0 {
            ecam.write32(a, reg, v & !DEVCTL_NOSNOOP);
        }
    }
    match vendor {
        // Intel: TCSEL (0x44, bity 2:0) = TC0.
        0x8086 => {
            let v = ecam.read32(a, 0x44);
            ecam.write32(a, 0x44, v & !7);
        }
        // AMD/ATI: snoop v registru 0x42 (bity 2:0 = 0b010).
        0x1002 | 0x1022 => {
            let v = ecam.read32(a, 0x40);
            ecam.write32(a, 0x40, (v & !(7 << 16)) | 2 << 16);
        }
        _ => {}
    }
}
