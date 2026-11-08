//! Stav ovladace: radic, kodeky, streamy (vystup a vstup), obsluha
//! pozadavku `audiod` a preruseni.

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use core::time::Duration;

use audioproto::device::{self, Info, OpenStream, op};
use audioproto::{Error, Format, Result, Sample, encode_reply};
use hdaspec::regs;
use libos::abi::vm::{MAP_READ, MAP_WRITE};
use libos::hw::{Interrupt, Resource};
use libos::{Channel, Handle, Mapping, Signals, Vmo, println, time};
use pci::{Address, Ecam};

use crate::codec::Codec;
use crate::ctrl::Ctrl;
use crate::stream::DmaStream;

/// Velikost registru radice (BAR0) podle specifikace.
const MMIO_SIZE: u64 = 0x4000;
/// Jak casto cist konektory: bez nevyzadanych odpovedi casto, s nimi jen
/// jako zaloha.
const JACK_POLL: Duration = Duration::from_millis(500);
const JACK_POLL_UNSOL: Duration = Duration::from_secs(3);
/// Cisla streamu (tag) vystupu a vstupu; 0 je rezervovana.
const OUT_TAG: u8 = 1;
const IN_TAG: u8 = 2;

pub struct Driver {
    ecam: Ecam,
    address: Address,
    res: Resource,
    ctrl: Ctrl,
    /// Kodeky s audio funkcni skupinou (vsechny s vystupem hraji).
    codecs: Vec<Codec>,
    irq: Option<Interrupt>,
    out: Option<DmaStream>,
    inp: Option<DmaStream>,
    name: String,
    /// Bitova hloubka zarizeni (16, nebo 24 kdyz ji umi vsechny prevodniky).
    bits: u32,
}

impl Driver {
    pub fn open(cfg: &Vmo, address: Address, res: Resource) -> Result<Self> {
        let ecam = Ecam::single(cfg, address)?;
        let vendor = ecam.read16(address, pci::VENDOR_ID);
        let device = ecam.read16(address, pci::VENDOR_ID + 2);
        let bar = ecam.memory_bar(address, 0).ok_or(Error::NotSupported)?;
        crate::quirks(&ecam, address, vendor);
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
        for c in &mut codecs {
            if let Err(e) = c.setup_outputs(&mut ctrl) {
                println!("hda {address}: kodek {} nejde nastavit: {e}", c.addr);
            }
        }
        ctrl.enable_unsol(irq.is_some());
        let ids: Vec<String> = codecs.iter().map(|c| alloc::format!("{:08x}", c.vendor)).collect();
        let name = alloc::format!(
            "HDA {vendor:04x}:{device:04x}{}",
            if ids.is_empty() { String::new() } else { alloc::format!(" kodek {}", ids.join("+")) }
        );
        let mut d = Self { ecam, address, res, ctrl, codecs, irq, out: None, inp: None, name, bits: 16 };
        d.bits = if d.all_support_24() { 24 } else { 16 };
        println!(
            "hda {address}: {}, {} vystupnich a {} vstupnich streamu, {} bitu, {}",
            d.name,
            d.ctrl.gcap.output_streams,
            d.ctrl.gcap.input_streams,
            d.bits,
            if d.irq.is_some() { "MSI" } else { "bez preruseni" }
        );
        Ok(d)
    }

    /// Umi 24 bitu (a 48 kHz) kazdy kodek s vystupem? Jen pak se pouzije.
    fn all_support_24(&self) -> bool {
        let outs: Vec<&Codec> = self.codecs.iter().filter(|c| c.has_output()).collect();
        !outs.is_empty() && outs.iter().all(|c| c.pcm.raw != 0 && c.pcm.bits(24) && c.pcm.rate(48_000))
    }

    fn rate(&self) -> u32 {
        match self.codecs.iter().find(|c| c.has_output()) {
            Some(c) if c.pcm.raw != 0 && !c.pcm.rate(48_000) && c.pcm.rate(44_100) => 44_100,
            _ => 48_000,
        }
    }

    /// Format vystupu: 48 kHz (nebo 44,1 kHz), stereo, 16 bitu, nebo 24 bitu
    /// v 32bitovem slove (`S32`), kdyz to umi vsechny kodeky.
    fn device_format(&self) -> Format {
        Format::new(self.rate(), 2, if self.bits == 24 { Sample::S32 } else { Sample::S16 })
    }

    /// Format vstupu: 48 kHz, stereo, 16 bitu (umi kazdy ADC).
    fn input_format(&self) -> Format {
        Format::new(48_000, 2, Sample::S16)
    }

    fn input_codec(&self) -> Option<&Codec> {
        self.codecs.iter().find(|c| c.has_input())
    }

    fn info(&self) -> Info {
        let outputs = self.codecs.iter().flat_map(|c| c.outputs()).collect();
        let input = self.input_codec().is_some() && self.ctrl.gcap.input_streams > 0;
        Info { name: self.name.clone(), outputs, format: self.device_format(), input }
    }

    fn hda_format(f: &Format) -> Result<u16> {
        let bits = if f.sample == Sample::S32 { 24 } else { 16 };
        hdaspec::format::encode(f.rate, bits, f.channels as u32).ok_or(Error::NotSupported)
    }

    fn open_stream(&mut self, input: bool, want: OpenStream) -> Result<(Vec<u8>, Vec<Handle>)> {
        let (expected, index, tag) = if input {
            if self.input_codec().is_none() || self.ctrl.gcap.input_streams == 0 {
                return Err(Error::NotSupported);
            }
            (self.input_format(), 0, IN_TAG)
        } else {
            if !self.codecs.iter().any(|c| c.has_active_output()) || self.ctrl.gcap.output_streams == 0 {
                return Err(Error::NotFound);
            }
            (self.device_format(), self.ctrl.gcap.first_output(), OUT_TAG)
        };
        if want.format != expected {
            return Err(Error::NotSupported);
        }
        let slot = if input { &mut self.inp } else { &mut self.out };
        if let Some(mut old) = slot.take() {
            old.close(&self.ctrl);
        }
        let fmt = Self::hda_format(&want.format)?;
        let s = DmaStream::open(&self.ctrl, &self.res, index, tag, fmt, want)?;
        let handles = vec![s.buf.handle().duplicate(None)?, s.status_vmo.handle().duplicate(None)?, s.event.handle().duplicate(None)?];
        let reply = device::encode_opened(&s.req);
        *slot = Some(s);
        Ok((reply, handles))
    }

    /// Nastavi stream vsem kodekum s vystupem (`tag` 0 = odpojit).
    fn route_output(&mut self, tag: u8) -> Result<()> {
        let fmt = self.out.as_ref().map_or(0, |s| s.fmt);
        for c in &self.codecs {
            c.set_stream(&mut self.ctrl, tag, fmt)?;
        }
        Ok(())
    }

    fn start(&mut self, input: bool) -> Result<()> {
        let polled = self.irq.is_none();
        if input {
            let fmt = self.inp.as_ref().ok_or(Error::BadState)?.fmt;
            let i = self.codecs.iter().position(|c| c.has_input()).ok_or(Error::NotSupported)?;
            self.codecs[i].set_input_stream(&mut self.ctrl, IN_TAG, fmt)?;
            self.inp.as_mut().ok_or(Error::BadState)?.start(&self.ctrl, polled);
        } else {
            self.out.as_ref().ok_or(Error::BadState)?;
            self.route_output(OUT_TAG)?;
            self.out.as_mut().ok_or(Error::BadState)?.start(&self.ctrl, polled);
        }
        Ok(())
    }

    fn stop(&mut self, input: bool) -> Result<()> {
        if input {
            self.inp.as_mut().ok_or(Error::BadState)?.stop(&self.ctrl);
            if let Some(i) = self.codecs.iter().position(|c| c.has_input()) {
                let _ = self.codecs[i].set_input_stream(&mut self.ctrl, 0, 0);
            }
        } else {
            self.out.as_mut().ok_or(Error::BadState)?.stop(&self.ctrl);
            let _ = self.route_output(0);
        }
        Ok(())
    }

    fn handle(&mut self, m: &[u8]) -> (Result<Vec<u8>>, Vec<Handle>) {
        let mut r = wire::Reader::new(m);
        let Ok(code) = r.u32() else { return (Err(Error::InvalidArgs), Vec::new()) };
        let none = Vec::new;
        match code {
            op::INFO => (Ok(self.info().encode()), none()),
            op::OPEN_OUTPUT | op::OPEN_INPUT => match OpenStream::decode(&mut r).and_then(|w| self.open_stream(code == op::OPEN_INPUT, w)) {
                Ok((reply, hs)) => (Ok(reply), hs),
                Err(e) => (Err(e), none()),
            },
            op::START => (self.start(false).map(|_| Vec::new()), none()),
            op::STOP => (self.stop(false).map(|_| Vec::new()), none()),
            op::START_INPUT => (self.start(true).map(|_| Vec::new()), none()),
            op::STOP_INPUT => (self.stop(true).map(|_| Vec::new()), none()),
            op::CLOSE | op::CLOSE_INPUT => {
                let slot = if code == op::CLOSE { &mut self.out } else { &mut self.inp };
                if let Some(mut s) = slot.take() {
                    s.close(&self.ctrl);
                }
                (Ok(Vec::new()), none())
            }
            op::TEST_EXIT => {
                println!("hda {}: test obnovy, koncim jako po padu", self.address);
                self.shutdown();
                libos::exit(99);
            }
            _ => (Err(Error::NotSupported), none()),
        }
    }

    /// Konektory (nevyzadana odpoved nebo pravidelne cteni); kdyz se
    /// zmenilo, ktere HDMI hraje, a vystup bezi, prepojit stream.
    fn jacks(&mut self, unsol: &[(u32, u8)], all: bool) {
        let mut changed = false;
        for c in &mut self.codecs {
            if all {
                changed |= c.poll_jacks(&mut self.ctrl, false);
            } else {
                let addr = c.addr;
                for &(resp, _) in unsol.iter().filter(|u| u.1 == addr) {
                    changed |= c.unsolicited(&mut self.ctrl, hdaspec::verb::unsol_tag(resp));
                }
            }
        }
        if changed && self.out.as_ref().is_some_and(|s| s.running) {
            let _ = self.route_output(OUT_TAG);
        }
    }

    fn running_period(&self) -> Option<u64> {
        [self.out.as_ref(), self.inp.as_ref()].into_iter().flatten().filter(|s| s.running).map(|s| s.period_ns()).min()
    }

    pub fn serve(&mut self, client: &Channel) {
        let jack_every = if self.ctrl.unsol_on { JACK_POLL_UNSOL } else { JACK_POLL };
        let mut next_jack = time::deadline_after(jack_every);
        loop {
            // Bez preruseni: cist pozici dvakrat za periodu. S MSI jednou za
            // dve periody jako zaloha (MSI zapnute, ale nedorucene).
            let poll = self.running_period().map(|p| {
                let ns = if self.irq.is_none() { p / 2 } else { p * 2 };
                time::deadline_after(Duration::from_nanos(ns))
            });
            let jack = self.codecs.iter().any(|c| c.has_jack_detect()).then_some(next_jack);
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
                for s in [self.out.as_mut(), self.inp.as_mut()].into_iter().flatten() {
                    s.interrupt(&self.ctrl);
                }
            }
            if self.ctrl.unsol_on {
                let u = self.ctrl.take_unsol();
                if !u.is_empty() {
                    self.jacks(&u, false);
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
            for s in [self.out.as_mut(), self.inp.as_mut()].into_iter().flatten() {
                s.poll(&self.ctrl);
            }
            if time::now() >= next_jack {
                next_jack = time::deadline_after(jack_every);
                self.jacks(&[], true);
            }
        }
    }

    pub fn shutdown(&mut self) {
        for s in [self.out.take(), self.inp.take()].into_iter().flatten() {
            let mut s = s;
            s.close(&self.ctrl);
        }
        self.ctrl.shutdown();
        pci::msi::disable(&self.ecam, self.address);
    }
}
