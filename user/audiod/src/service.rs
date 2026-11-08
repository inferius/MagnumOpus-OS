//! Stav sluzby: zarizeni, klienti, streamy; smycka, pozadavky, mix
//! a rozvod nahravani.

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use core::time::Duration;

use audioproto::client::{DeviceEntry, Request, Status};
use audioproto::device::Info;
use audioproto::{Direction, Error, Format, Result, encode_reply, volume_gain};
use audiomix::Frame;
use audiomix::convert::frame_to_stereo;
use audiomix::mix::Mixer;
use libos::{Channel, Handle, Signals, println, time};

use crate::device::{self, Device, LEAD_PERIODS};
use crate::streams::{State, Stream};

/// Za jak dlouho bez hrajiciho streamu zastavit DMA.
pub const IDLE_STOP: Duration = Duration::from_secs(2);
const MAX_CLIENTS: usize = 64;
const MAX_STREAMS_PER_CLIENT: usize = 16;
/// Nejvic zprav jednoho klienta za jedno probuzeni (ferovost).
const BURST: usize = 16;
const MAX_DEVICES: usize = 16;

pub struct Client {
    pub chan: Channel,
    pub open: bool,
    /// Bootstrap kanal od initu: smi pridavat ovladace.
    pub privileged: bool,
}

/// Zarizeni v seznamu. `chan` je `None`, kdyz je zarizeni prave otevrene
/// (kanal drzi [`Device`]) nebo jeho ovladac skoncil.
pub struct Slot {
    pub chan: Option<Channel>,
    pub info: Info,
}

pub struct Service {
    pub slots: Vec<Slot>,
    pub active: Option<usize>,
    pub device: Option<Device>,
    /// Samostatne zarizeni pro nahravani (slot, zarizeni), kdyz aktivni
    /// vstup nema (viz `input.rs`).
    pub rec: Option<(usize, Device)>,
    /// Aktivni zarizeni, jehoz ovladac skoncil: po jeho restartu se na
    /// nej sluzba vrati.
    pub lost: Option<usize>,
    pub clients: Vec<Client>,
    pub streams: Vec<Stream>,
    pub next_id: u32,
    pub master: u8,
    pub muted: bool,
    pub mixer: Mixer,
    pub out: Vec<u8>,
    pub idle_since: Option<u64>,
}

impl Service {
    /// Vybere prvni zarizeni s analogovym vystupem (nebo prvni, ktere
    /// vubec neco umi).
    pub fn choose_default(&mut self) {
        let pick = self.slots.iter().position(|s| s.chan.is_some() && s.info.has_analog()).or_else(|| {
            self.slots.iter().position(|s| s.chan.is_some() && s.info.outputs.iter().any(|o| o.active))
        });
        if let Some(i) = pick
            && let Err(e) = self.select(i)
        {
            println!("audiod: zarizeni {} nejde otevrit: {e}", self.slots[i].info.name);
        }
    }

    /// Prepne na zarizeni `i`: stare zavre, nove otevre, streamy prepocita
    /// na jeho takt. Nahravaci streamy pokracuji, kdyz nove zarizeni umi
    /// vstup; jinak se zastavi.
    pub fn select(&mut self, i: usize) -> Result<()> {
        if self.active == Some(i) {
            return Ok(());
        }
        // Nahravani se po prepnuti vybere znovu (nove zarizeni muze mit vstup).
        self.release_rec();
        let chan = self.slots.get_mut(i).ok_or(Error::NotFound)?.chan.take().ok_or(Error::NotFound)?;
        let info = device::query(&chan).unwrap_or_else(|_| self.slots[i].info.clone());
        let dev = match Device::open(chan.handle().duplicate(None).map(Channel::from_handle)?, info.clone()) {
            Ok(d) => d,
            Err(e) => {
                self.slots[i].chan = Some(chan);
                return Err(e);
            }
        };
        drop(chan);
        if let Some(mut old) = self.device.take() {
            old.close();
            if let Some(a) = self.active {
                self.slots[a].chan = Some(Channel::from_handle(old.channel().handle().duplicate(None)?));
            }
        }
        self.slots[i].info = info;
        self.active = Some(i);
        let clock = dev.clock();
        let period = dev.out.period as usize;
        self.mixer = Mixer::new(period);
        self.out = vec![0u8; period * dev.out.frame_bytes() as usize];
        self.device = Some(dev);
        let mut input_clock = None;
        if self.streams.iter().any(|s| s.dir == Direction::Input) {
            input_clock = self.input_dev().and_then(|d| d.input_clock()).ok();
        }
        let mut need_out = false;
        let mut need_in = false;
        for s in &mut self.streams {
            match (s.dir, input_clock) {
                (Direction::Output, _) => {
                    s.retarget(clock);
                    need_out |= s.active();
                }
                (Direction::Input, Some(c)) => {
                    s.retarget(c);
                    need_in |= s.active();
                }
                (Direction::Input, None) => s.stop(),
            }
        }
        if need_out && let Some(d) = self.device.as_mut() {
            d.start()?;
        }
        if need_in {
            self.input_dev()?.start_input()?;
        }
        Ok(())
    }

    /// Ovladac aktivniho zarizeni skoncil: vyradit ho a zkusit jine.
    fn device_lost(&mut self) {
        let name = self.device.as_ref().map(|d| d.info.name.clone()).unwrap_or_default();
        println!("audiod: ovladac zvukove karty {name} skoncil, zarizeni odpojeno");
        self.device = None;
        if let Some(a) = self.active.take() {
            self.slots[a].chan = None;
            self.lost = Some(a);
        }
        for s in &mut self.streams {
            if s.drain_pending
                && let Some(c) = self.clients.get(s.owner)
            {
                let _ = c.chan.write(&encode_reply(Err(Error::PeerClosed)), Vec::new());
            }
            s.stop();
        }
        self.choose_default();
    }

    /// Init pridal ovladac (napr. po jeho restartu).
    fn add_device(&mut self, chan: Channel) {
        let info = match device::query(&chan) {
            Ok(i) => i,
            Err(e) => {
                println!("audiod: novy ovladac neodpovida: {e}");
                return;
            }
        };
        println!("audiod: nove zarizeni {}", info.name);
        // Stejne jmeno a mrtvy kanal = restartovany ovladac: nahradit slot.
        let reuse = self.slots.iter().enumerate().position(|(i, s)| s.chan.is_none() && Some(i) != self.active && s.info.name == info.name);
        match reuse {
            Some(i) => {
                self.slots[i] = Slot { chan: Some(chan), info };
                // Vratil se ovladac drive aktivniho zarizeni: zpet na nej.
                if self.lost == Some(i) {
                    self.lost = None;
                    if let Err(e) = self.select(i) {
                        println!("audiod: obnovene zarizeni nejde otevrit: {e}");
                    }
                    return;
                }
            }
            None if self.slots.len() < MAX_DEVICES => self.slots.push(Slot { chan: Some(chan), info }),
            None => return,
        }
        if self.device.is_none() {
            self.choose_default();
        }
    }

    pub fn run(&mut self) {
        loop {
            // Nejdrive zavrene klienty (s jejich streamy) odstranit.
            self.reap();
            if self.clients.is_empty() {
                println!("audiod: zadni klienti, koncim");
                return;
            }
            let nc = self.clients.len();
            let mut items: Vec<(&Handle, Signals)> = Vec::with_capacity(nc + 3);
            for c in &self.clients {
                items.push((c.chan.handle(), Signals::READABLE | Signals::PEER_CLOSED));
            }
            let mut running_period = None;
            let mut rec_idx = None;
            if let Some(d) = self.device.as_ref() {
                items.push((d.channel().handle(), Signals::PEER_CLOSED));
                for p in [Some(&d.out), d.inp.as_ref()].into_iter().flatten().filter(|p| p.running) {
                    items.push((p.event.handle(), Signals::SIGNALED));
                    running_period = Some(p.format.us_for_frames(p.period as u64 * 2));
                }
            }
            if let Some((_, d)) = self.rec.as_ref() {
                rec_idx = Some(items.len());
                items.push((d.channel().handle(), Signals::PEER_CLOSED));
                if let Some(p) = d.inp.as_ref().filter(|p| p.running) {
                    items.push((p.event.handle(), Signals::SIGNALED));
                    running_period = Some(p.format.us_for_frames(p.period as u64 * 2));
                }
            }
            // Zaloha, kdyby udalost od ovladace neprisla: aspon jednou za
            // dve periody.
            let deadline = running_period.map(|us| time::deadline_after(Duration::from_micros(us)));
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
            if let Some(r) = rec_idx
                && observed.get(r).is_some_and(|s| s.intersects(Signals::PEER_CLOSED))
            {
                self.rec_lost();
            }
            if self.device.is_some() && observed.get(nc).is_some_and(|s| s.intersects(Signals::PEER_CLOSED)) {
                self.device_lost();
            }
            for (i, sig) in observed.iter().enumerate().take(nc) {
                if sig.intersects(Signals::READABLE) {
                    self.client_messages(i);
                } else if sig.intersects(Signals::PEER_CLOSED) {
                    self.clients[i].open = false;
                }
            }
            for d in [self.device.as_ref(), self.rec.as_ref().map(|r| &r.1)].into_iter().flatten() {
                let _ = d.out.event.reset();
                if let Some(p) = &d.inp {
                    let _ = p.event.reset();
                }
            }
            self.pump();
            self.capture();
            self.idle();
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
            if self.clients[i].privileged && m.data == b"device" {
                for h in m.handles {
                    self.add_device(Channel::from_handle(h));
                }
                continue;
            }
            // Test obnovy (jen init): ovladac aktivniho zarizeni ukoncit.
            if self.clients[i].privileged && m.data == b"crash-driver" {
                if let Some(d) = self.device.as_ref() {
                    let _ = d.channel().write(&audioproto::device::simple(audioproto::device::op::TEST_EXIT), Vec::new());
                }
                continue;
            }
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

    /// Spusti stream a jeho smer na zarizeni.
    fn start_stream(&mut self, client: usize, id: u32) -> Result<()> {
        let dir = self.stream_mut(client, id)?.dir;
        let r = match dir {
            Direction::Output => self.device.as_mut().ok_or(Error::NotFound).and_then(|d| d.start()),
            Direction::Input => self.input_dev().and_then(|d| d.start_input()),
        };
        if let Err(e) = r {
            println!("audiod: zarizeni nejde spustit: {e}");
            return Err(e);
        }
        self.stream_mut(client, id)?.start();
        self.idle_since = None;
        Ok(())
    }

    /// Obslouzi pozadavek; `None` = odpoved prijde pozdeji.
    fn request(&mut self, client: usize, req: Request) -> Option<(Result<Vec<u8>>, Vec<Handle>)> {
        let none = Vec::new;
        let ok = |r: Result<()>| (r.map(|_| Vec::new()), Vec::new());
        Some(match req {
            Request::Open { dir, format, latency_us } => match self.open(client, dir, format, latency_us) {
                Ok((r, hs)) => (Ok(r), hs),
                Err(e) => (Err(e), none()),
            },
            Request::Start { stream } => ok(self.start_stream(client, stream)),
            Request::Stop { stream } => ok(self.stream_mut(client, stream).map(|s| s.stop())),
            Request::Close { stream } => {
                let before = self.streams.len();
                self.streams.retain(|s| !(s.id == stream && s.owner == client));
                ok(if self.streams.len() < before { Ok(()) } else { Err(Error::NotFound) })
            }
            Request::Drain { stream } => match self.stream_mut(client, stream) {
                Ok(s) if s.state == State::Stopped || s.dir == Direction::Input => (Ok(Vec::new()), none()),
                Ok(s) => {
                    s.state = State::Draining;
                    s.drain_pending = true;
                    return None;
                }
                Err(e) => (Err(e), none()),
            },
            Request::StreamVolume { stream, percent } => ok(self.stream_mut(client, stream).map(|s| s.volume = percent)),
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
            Request::Devices => (Ok(DeviceEntry::encode_list(&self.device_list())), none()),
            Request::Select { index } => {
                let r = self.select(index as usize);
                if r.is_ok() {
                    println!("audiod: vybrano zarizeni {index}");
                }
                ok(r)
            }
            Request::Connect => {
                if self.clients.len() >= MAX_CLIENTS {
                    return Some((Err(Error::NoResources), none()));
                }
                match Channel::create() {
                    Ok((ours, theirs)) => {
                        self.clients.push(Client { chan: ours, open: true, privileged: false });
                        (Ok(Vec::new()), vec![theirs.into_handle()])
                    }
                    Err(e) => (Err(e), none()),
                }
            }
        })
    }

    fn open(&mut self, client: usize, dir: Direction, format: Format, latency_us: u32) -> Result<(Vec<u8>, Vec<Handle>)> {
        if self.streams.iter().filter(|s| s.owner == client).count() >= MAX_STREAMS_PER_CLIENT {
            return Err(Error::NoResources);
        }
        let clock = match dir {
            Direction::Output => self.device.as_ref().ok_or(Error::NotFound)?.clock(),
            Direction::Input => self.input_dev()?.input_clock()?,
        };
        let id = self.next_id;
        let (s, reply, hs) = Stream::open(id, client, clock, dir, format, latency_us)?;
        self.next_id = self.next_id.wrapping_add(1).max(1);
        self.streams.push(s);
        Ok((reply, hs))
    }

    fn device_list(&self) -> Vec<DeviceEntry> {
        self.slots
            .iter()
            .enumerate()
            .filter(|(i, s)| s.chan.is_some() || self.active == Some(*i) || self.rec.as_ref().is_some_and(|r| r.0 == *i))
            .map(|(i, s)| DeviceEntry {
                index: i as u8,
                name: s.info.name.clone(),
                outputs: s.info.active_names(),
                input: s.info.input,
                active: self.active == Some(i),
            })
            .collect()
    }

    fn status(&self) -> Status {
        let d = self.device.as_ref();
        Status {
            device: d.map_or(String::new(), |d| d.info.name.clone()),
            outputs: d.map_or(String::new(), |d| d.info.active_names()),
            format: d.map_or(Format::DEVICE_DEFAULT, |d| d.out.format),
            master: self.master,
            muted: self.muted,
            streams: self.streams.len() as u32,
            running: self.streams.iter().filter(|s| s.active()).count() as u32,
            hw_pos: d.map_or(0, |d| d.out.status.position()),
            irqs: d.map_or(0, |d| d.out.status.irqs()),
            device_underruns: d.map_or(0, |d| d.underruns),
            latency_us: d.map_or(0, |d| d.latency_us()),
            period: d.map_or(0, |d| d.out.period),
            input: self.any_input(),
            device_index: self.active.unwrap_or(0) as u8,
        }
    }

    /// Smicha, co je potreba, a vyridi dohrane streamy (DRAIN).
    fn pump(&mut self) {
        let Some(dev) = self.device.as_mut() else { return };
        if !dev.out.running {
            return;
        }
        let port = &mut dev.out;
        let hw = port.status.position();
        if hw > port.pos {
            // Nestihli jsme: zarizeni hraje ticho (nulovana perioda).
            dev.underruns += 1;
            port.pos = hw;
        }
        let target = hw + (port.period * LEAD_PERIODS) as u64;
        let gain = if self.muted { 0 } else { volume_gain(self.master) };
        let fb = port.frame_bytes() as usize;
        while port.pos < target {
            let n = ((target - port.pos) as usize).min(port.period as usize);
            self.mixer.clear();
            for s in &mut self.streams {
                s.mix(&mut self.mixer, n, port.pos);
            }
            let bytes = n * fb;
            self.mixer.finish(port.format.sample, gain, &mut self.out[..bytes]);
            port.write_at(port.pos, &self.out[..bytes]);
            port.pos += n as u64;
        }
        // Za smichanymi daty jedna perioda ticha (viz device.rs).
        let guard = port.period as usize * fb;
        self.out[..guard].iter_mut().for_each(|b| *b = 0);
        port.write_at(port.pos, &self.out[..guard]);
        let (written, rate) = (port.pos, port.format.rate);
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
    }

    /// Nahravani: data, ktera zarizeni od minule zapsalo, rozdat streamum.
    fn capture(&mut self) {
        let Some(dev) = self.input_dev_mut() else { return };
        let Some(port) = dev.inp.as_mut().filter(|p| p.running) else { return };
        let hw = port.status.position();
        // Zarizeni nas predehnalo o vic nez buffer: nejstarsi data jsou pryc.
        let keep = (port.buffer - port.period) as u64;
        if hw.saturating_sub(port.pos) > keep {
            dev.overruns += 1;
            port.pos = hw - keep;
        }
        let fb = port.frame_bytes() as usize;
        let mut bytes = vec![0u8; port.period as usize * fb];
        let mut chunks: Vec<Vec<Frame>> = Vec::new();
        while port.pos < hw {
            let n = ((hw - port.pos) as usize).min(port.period as usize);
            port.read_at(port.pos, &mut bytes[..n * fb]);
            chunks.push(bytes[..n * fb].chunks_exact(fb).map(|b| frame_to_stereo(&port.format, b)).collect());
            port.pos += n as u64;
        }
        for frames in &chunks {
            for s in &mut self.streams {
                s.capture(frames);
            }
        }
    }

    /// Necinnost: zastavit DMA smeru, ve kterem nic nebezi dele nez
    /// [`IDLE_STOP`].
    fn idle(&mut self) {
        let out_busy = self.streams.iter().any(|s| s.dir == Direction::Output && s.active());
        let in_busy = self.streams.iter().any(|s| s.dir == Direction::Input && s.active());
        if !in_busy
            && let Some(dev) = self.input_dev_mut()
            && dev.inp.as_ref().is_some_and(|p| p.running)
        {
            let _ = dev.stop_input();
        }
        let Some(dev) = self.device.as_mut() else { return };
        if out_busy || !dev.out.running {
            self.idle_since = None;
            return;
        }
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
