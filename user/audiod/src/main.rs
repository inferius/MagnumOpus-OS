//! audiod: zvukova sluzba. Mixuje streamy klientu do jednoho vystupu
//! zarizeni, prevadi formaty a vzorkovaci frekvence, ridi hlavni
//! hlasitost a ztlumeni.
//!
//! Start: handly `kind::USER` = kanaly k ovladacum zvukovych karet
//! (`audioproto::device`). Bootstrap kanal od initu je prvni klient
//! (protokol `audioproto::client`); dalsi klienty vytvari `CONNECT`.
//!
//! Smycka: ceka na kanaly klientu a na udalost periody od ovladace. Po
//! kazde periode precte pozici DMA a smicha streamy tak, aby byla data
//! [`device::LEAD_PERIODS`] period pred pozici (rozpocet latence viz
//! `device.rs`). Kdyz zadny stream nehraje dele nez [`IDLE_STOP`],
//! zastavi DMA (uspora energie; zarizeni neprehrava ticho do nekonecna).

#![no_std]
#![no_main]

extern crate alloc;

mod device;
mod streams;

use alloc::vec;
use alloc::vec::Vec;
use core::time::Duration;

use audioproto::client::{Request, Status};
use audioproto::{Error, Format, Result, encode_reply, volume_gain};
use audiomix::mix::Mixer;
use libos::{Channel, Handle, Signals, Startup, println, time};

use device::{Device, LEAD_PERIODS};
use streams::{State, Stream};

libos::entry!(main);

/// Za jak dlouho bez hrajiciho streamu zastavit DMA.
const IDLE_STOP: Duration = Duration::from_secs(2);
const MAX_CLIENTS: usize = 64;
const MAX_STREAMS_PER_CLIENT: usize = 16;
/// Nejvic zprav jednoho klienta za jedno probuzeni (ferovost).
const BURST: usize = 16;

struct Client {
    chan: Channel,
    open: bool,
}

struct Service {
    device: Option<Device>,
    clients: Vec<Client>,
    streams: Vec<Stream>,
    next_id: u32,
    master: u8,
    muted: bool,
    mixer: Mixer,
    out: Vec<u8>,
    idle_since: Option<u64>,
}

fn main(startup: Startup) -> i64 {
    let mut device = None;
    for h in startup.handles {
        let chan = Channel::from_handle(h);
        if device.is_some() {
            continue; // zatim jen jedno zarizeni (prvni s analogovym vystupem)
        }
        match device::query(&chan) {
            Ok(info) if info.has_analog() => match Device::open(chan, info) {
                Ok(d) => device = Some(d),
                Err(e) => println!("audiod: zarizeni nejde otevrit: {e}"),
            },
            Ok(info) => println!("audiod: {} nema analogovy vystup ({}), preskakuji", info.name, info.outputs.len()),
            Err(e) => println!("audiod: ovladac neodpovida: {e}"),
        }
    }
    if device.is_none() {
        println!("audiod: zadne zvukove zarizeni, streamy budou odmitnuty");
    }
    let period = device.as_ref().map_or(256, |d| d.period) as usize;
    let mut svc = Service {
        device,
        clients: vec![Client { chan: startup.bootstrap, open: true }],
        streams: Vec::new(),
        next_id: 1,
        master: 100,
        muted: false,
        mixer: Mixer::new(period),
        out: vec![0u8; period * 4],
        idle_since: None,
    };
    let _ = svc.clients[0].chan.write(b"ok", Vec::new());
    svc.run();
    0
}

impl Service {
    fn run(&mut self) {
        loop {
            // Nejdrive zavrene klienty (s jejich streamy) odstranit.
            self.reap();
            if self.clients.is_empty() {
                println!("audiod: zadni klienti, koncim");
                return;
            }
            let mut items: Vec<(&Handle, Signals)> = Vec::with_capacity(self.clients.len() + 1);
            for c in &self.clients {
                items.push((c.chan.handle(), Signals::READABLE | Signals::PEER_CLOSED));
            }
            let dev_running = self.device.as_ref().is_some_and(|d| d.running);
            let nc = self.clients.len();
            if let Some(d) = self.device.as_ref() {
                items.push((d.channel().handle(), Signals::PEER_CLOSED));
                if dev_running {
                    items.push((d.event.handle(), Signals::SIGNALED));
                }
            }
            // Zaloha, kdyby udalost od ovladace neprisla: aspon jednou za
            // dve periody, a cas zastaveni necinneho zarizeni.
            let deadline = if dev_running {
                let d = self.device.as_ref().unwrap();
                Some(time::deadline_after(Duration::from_micros(d.format.us_for_frames(d.period as u64 * 2))))
            } else {
                None
            };
            let woke = libos::wait_many(&items, deadline);
            drop(items);
            let observed = match woke {
                Ok((_, o)) => o,
                Err(Error::TimedOut) => Vec::new(),
                Err(e) => {
                    println!("audiod: chyba cekani: {e}");
                    return;
                }
            };
            if observed.get(nc).is_some_and(|s| s.intersects(Signals::PEER_CLOSED)) {
                // Ovladac skoncil: streamy zastavit (cekani na DRAIN odpovi chybou).
                println!("audiod: ovladac zvukove karty skoncil, zarizeni odpojeno");
                self.device = None;
                for s in &mut self.streams {
                    if s.drain_pending
                        && let Some(c) = self.clients.get(s.owner)
                    {
                        let _ = c.chan.write(&encode_reply(Err(Error::PeerClosed)), Vec::new());
                    }
                    s.stop();
                }
            }
            for (i, sig) in observed.iter().enumerate().take(nc) {
                if sig.intersects(Signals::READABLE) {
                    self.client_messages(i);
                } else if sig.intersects(Signals::PEER_CLOSED) {
                    self.clients[i].open = false;
                }
            }
            if let Some(d) = self.device.as_ref() {
                let _ = d.event.reset();
            }
            self.pump();
        }
    }

    /// Odstrani zavrene klienty a jejich streamy (indexy vlastniku se
    /// precisluji).
    fn reap(&mut self) {
        let mut i = 0;
        while i < self.clients.len() {
            if self.clients[i].open {
                i += 1;
                continue;
            }
            self.clients.remove(i);
            self.streams.retain(|s| s.owner != i);
            for s in &mut self.streams {
                if s.owner > i {
                    s.owner -= 1;
                }
            }
        }
    }

    fn client_messages(&mut self, i: usize) {
        for _ in 0..BURST {
            let m = match self.clients[i].chan.try_read() {
                Ok(m) => m,
                Err(Error::ShouldWait) => return,
                Err(_) => {
                    self.clients[i].open = false;
                    return;
                }
            };
            let (reply, hs) = match Request::decode(&m.data) {
                Ok(req) => match self.request(i, req) {
                    Some(r) => r,
                    None => continue, // odpoved pozdeji (DRAIN)
                },
                Err(e) => (Err(e), Vec::new()),
            };
            if self.clients[i].chan.write(&encode_reply(reply), hs).is_err() {
                self.clients[i].open = false;
                return;
            }
        }
    }

    fn stream_mut(&mut self, client: usize, id: u32) -> Result<&mut Stream> {
        self.streams.iter_mut().find(|s| s.id == id && s.owner == client).ok_or(Error::NotFound)
    }

    /// Obslouzi pozadavek; `None` = odpoved prijde pozdeji.
    fn request(&mut self, client: usize, req: Request) -> Option<(Result<Vec<u8>>, Vec<Handle>)> {
        let none = Vec::new;
        Some(match req {
            Request::Open { dir, format, latency_us } => match self.open(client, dir, format, latency_us) {
                Ok((r, hs)) => (Ok(r), hs),
                Err(e) => (Err(e), none()),
            },
            Request::Start { stream } => {
                let r = self.stream_mut(client, stream).map(|s| s.start());
                if r.is_ok()
                    && let Some(d) = self.device.as_mut()
                    && let Err(e) = d.start()
                {
                    println!("audiod: zarizeni nejde spustit: {e}");
                    if let Ok(s) = self.stream_mut(client, stream) {
                        s.stop();
                    }
                    return Some((Err(e), none()));
                }
                self.idle_since = None;
                (r.map(|_| Vec::new()), none())
            }
            Request::Stop { stream } => (self.stream_mut(client, stream).map(|s| s.stop()).map(|_| Vec::new()), none()),
            Request::Close { stream } => {
                let before = self.streams.len();
                self.streams.retain(|s| !(s.id == stream && s.owner == client));
                let r = if self.streams.len() < before { Ok(Vec::new()) } else { Err(Error::NotFound) };
                (r, none())
            }
            Request::Drain { stream } => match self.stream_mut(client, stream) {
                Ok(s) if s.state == State::Stopped => (Ok(Vec::new()), none()),
                Ok(s) => {
                    s.state = State::Draining;
                    s.drain_pending = true;
                    return None;
                }
                Err(e) => (Err(e), none()),
            },
            Request::StreamVolume { stream, percent } => {
                (self.stream_mut(client, stream).map(|s| s.volume = percent).map(|_| Vec::new()), none())
            }
            Request::MasterVolume { percent } => {
                self.master = percent;
                println!("audiod: hlavni hlasitost {percent} %");
                (Ok(Vec::new()), none())
            }
            Request::Mute { on } => {
                self.muted = on;
                println!("audiod: {}", if on { "ztlumeno" } else { "zvuk zapnut" });
                (Ok(Vec::new()), none())
            }
            Request::Status => (Ok(self.status().encode()), none()),
            Request::StreamInfo { stream } => (self.stream_mut(client, stream).map(|s| s.info().encode()), none()),
            Request::Connect => {
                if self.clients.len() >= MAX_CLIENTS {
                    return Some((Err(Error::NoResources), none()));
                }
                match Channel::create() {
                    Ok((ours, theirs)) => {
                        self.clients.push(Client { chan: ours, open: true });
                        (Ok(Vec::new()), vec![theirs.into_handle()])
                    }
                    Err(e) => (Err(e), none()),
                }
            }
        })
    }

    fn open(&mut self, client: usize, dir: audioproto::Direction, format: Format, latency_us: u32) -> Result<(Vec<u8>, Vec<Handle>)> {
        let dev = self.device.as_ref().ok_or(Error::NotFound)?;
        if self.streams.iter().filter(|s| s.owner == client).count() >= MAX_STREAMS_PER_CLIENT {
            return Err(Error::NoResources);
        }
        let id = self.next_id;
        let (s, reply, hs) = Stream::open(id, client, dev, dir, format, latency_us)?;
        self.next_id = self.next_id.wrapping_add(1).max(1);
        self.streams.push(s);
        Ok((reply, hs))
    }

    fn status(&self) -> Status {
        let d = self.device.as_ref();
        Status {
            device: d.map_or(Default::default(), |d| d.info.name.clone()),
            outputs: d.map_or(Default::default(), |d| d.info.active_names()),
            format: d.map_or(Format::DEVICE_DEFAULT, |d| d.format),
            master: self.master,
            muted: self.muted,
            streams: self.streams.len() as u32,
            running: self.streams.iter().filter(|s| s.active()).count() as u32,
            hw_pos: d.map_or(0, |d| d.status.position()),
            irqs: d.map_or(0, |d| d.status.irqs()),
            device_underruns: d.map_or(0, |d| d.underruns),
            latency_us: d.map_or(0, |d| d.latency_us()),
            period: d.map_or(0, |d| d.period),
        }
    }

    /// Smicha, co je potreba, a vyridi dohrane streamy (DRAIN).
    fn pump(&mut self) {
        let Some(dev) = self.device.as_mut() else { return };
        if !dev.running {
            return;
        }
        let hw = dev.status.position();
        if hw > dev.written {
            // Nestihli jsme: zarizeni hraje ticho (nulovana perioda).
            dev.underruns += 1;
            dev.written = hw;
        }
        let target = hw + (dev.period * LEAD_PERIODS) as u64;
        let gain = if self.muted { 0 } else { volume_gain(self.master) };
        while dev.written < target {
            let n = ((target - dev.written) as usize).min(dev.period as usize);
            self.mixer.clear();
            for s in &mut self.streams {
                s.mix(&mut self.mixer, n, dev.written);
            }
            let bytes = n * dev.frame_bytes() as usize;
            self.mixer.finish_s16(gain, &mut self.out[..bytes]);
            dev.write_at(dev.written, &self.out[..bytes]);
            dev.written += n as u64;
        }
        // Za smichanymi daty jedna perioda ticha (viz device.rs).
        let guard = dev.period as usize * dev.frame_bytes() as usize;
        self.out[..guard].iter_mut().for_each(|b| *b = 0);
        dev.write_at(dev.written, &self.out[..guard]);
        let (written, rate) = (dev.written, dev.format.rate);
        let mut done = Vec::new();
        for s in &mut self.streams {
            if s.after_mix(hw, written, rate) && s.drain_pending {
                s.drain_pending = false;
                done.push(s.owner);
            }
        }
        for owner in done {
            if let Some(c) = self.clients.get(owner) {
                let _ = c.chan.write(&encode_reply(Ok(Vec::new())), Vec::new());
            }
        }
        // Necinnost: zastavit DMA.
        if self.streams.iter().any(|s| s.active()) {
            self.idle_since = None;
        } else {
            let now = time::now();
            let since = *self.idle_since.get_or_insert(now);
            if now - since > IDLE_STOP.as_nanos() as u64 {
                self.idle_since = None;
                if let Err(e) = dev.stop() {
                    println!("audiod: zarizeni nejde zastavit: {e}");
                }
            }
        }
    }
}
