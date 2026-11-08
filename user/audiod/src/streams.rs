//! Streamy klientu: sdileny buffer, prevod formatu a frekvence, mix do
//! DMA bufferu zarizeni.

use alloc::vec;
use alloc::vec::Vec;

use audioproto::client::{Opened, StreamInfo};
use audioproto::ring::{self, Ring};
use audioproto::{Direction, Error, Format, Result, volume_gain};
use audiomix::convert::frame_to_stereo;
use audiomix::mix::Mixer;
use audiomix::src::Resampler;
use audiomix::Frame;
use libos::abi::vm::{MAP_READ, MAP_WRITE};
use libos::{Event, Handle, Mapping, Vmo};

use crate::device::Device;

/// Snimky ticha na konec dohravani (historie Catmull-Rom je 4 snimky,
/// vystup lezi za druhym).
const FLUSH_FRAMES: u8 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Stopped,
    Running,
    /// Ceka na dohrani; po nem odpoved na `DRAIN` a zastaveni.
    Draining,
}

pub struct Stream {
    pub id: u32,
    /// Index klienta (vlastnika).
    pub owner: usize,
    pub format: Format,
    _vmo: Vmo,
    _map: Mapping,
    pub ring: Ring,
    pub event: Event,
    pub state: State,
    pub volume: u8,
    src: Resampler,
    /// Snimky klienta prectene z bufferu celkem.
    consumed: u64,
    /// Pozice zarizeni za poslednim skutecnym vzorkem streamu.
    last_data: u64,
    /// Mel stream v minule periode data (pro pocitani podteceni).
    fed: bool,
    pub underruns: u64,
    /// Ceka nekdo na `DRAIN`?
    pub drain_pending: bool,
    /// Kolik snimku ticha jeste dat prevodniku frekvence pri dohravani:
    /// posledni vstupy jsou v jeho historii a bez nich by nezazneli.
    flush: u8,
    scratch: Vec<u8>,
    frames: Vec<Frame>,
}

impl Stream {
    /// Novy stream: buffer na `latency_us` (zaokrouhleno na periody
    /// zarizeni, min. 2). Vraci stream, odpoved a handly pro klienta.
    pub fn open(id: u32, owner: usize, dev: &Device, dir: Direction, format: Format, latency_us: u32) -> Result<(Self, Vec<u8>, Vec<Handle>)> {
        if dir == Direction::Input {
            // Protokol nahravani je hotovy, ovladac ho zatim neumi.
            if !dev.info.input {
                return Err(Error::NotSupported);
            }
        }
        let lat = latency_us.clamp(audioproto::client::MIN_LATENCY_US, audioproto::client::MAX_LATENCY_US);
        // Perioda zarizeni ve snimcich klienta.
        let period = (dev.period as u64 * format.rate as u64).div_ceil(dev.format.rate as u64) as u32;
        let want = format.frames_for_us(lat as u64) as u32;
        let capacity = want.div_ceil(period).max(2) * period;
        let fb = format.frame_bytes();
        if capacity as usize * fb as usize > ring::MAX_DATA {
            return Err(Error::InvalidArgs);
        }
        let size = ring::vmo_size(capacity, fb) as u64;
        let vmo = Vmo::create(size)?;
        let map = Mapping::new(&vmo, 0, size, MAP_READ | MAP_WRITE)?;
        // SAFETY: mapovani o `size` bajtech zije v `Stream`.
        let ring = unsafe { Ring::init(map.addr() as *mut u8, map.len() as usize, capacity, fb, dir)? };
        let latency = format.us_for_frames(capacity as u64) as u32 + dev.latency_us();
        ring.set_latency_us(latency as u64);
        let event = Event::create()?;
        let handles = vec![vmo.handle().duplicate(None)?, event.handle().duplicate(None)?];
        let reply = Opened { stream: id, format, capacity, period, latency_us: latency }.encode();
        let s = Self {
            id,
            owner,
            format,
            _vmo: vmo,
            _map: map,
            ring,
            event,
            state: State::Stopped,
            volume: 100,
            src: Resampler::new(format.rate, dev.format.rate),
            consumed: 0,
            last_data: 0,
            fed: false,
            underruns: 0,
            drain_pending: false,
            flush: FLUSH_FRAMES,
            scratch: Vec::new(),
            frames: vec![[0; 2]; dev.period as usize],
        };
        Ok((s, reply, handles))
    }

    pub fn start(&mut self) {
        if self.state == State::Stopped {
            self.src.reset();
            self.fed = false;
        }
        self.state = State::Running;
        self.flush = FLUSH_FRAMES;
    }

    pub fn stop(&mut self) {
        self.state = State::Stopped;
        self.ring.discard();
        self.drain_pending = false;
    }

    pub fn info(&self) -> StreamInfo {
        StreamInfo { running: self.state != State::Stopped, played: self.ring.played(), underruns: self.underruns, volume: self.volume }
    }

    /// Smicha `n` snimku na pozici zarizeni `pos` do `mixer`.
    pub fn mix(&mut self, mixer: &mut Mixer, n: usize, pos: u64) {
        if self.state == State::Stopped || n == 0 {
            return;
        }
        if !self.ring.consistent() {
            // Klient zapsal nesmyslnou pozici: zahodit, co tam je.
            self.ring.resync();
        }
        let fb = self.format.frame_bytes() as usize;
        let need = self.src.input_needed(n as u32).min(self.ring.readable()) as usize;
        self.scratch.resize(need * fb, 0);
        let got = self.ring.peek(&mut self.scratch) as usize;
        let mut k = 0usize;
        let (fmt, scratch) = (self.format, &self.scratch);
        let draining = self.state == State::Draining && self.ring.readable() as usize == got;
        let flush = &mut self.flush;
        let out = &mut self.frames[..n];
        let made = self.src.process(out, || {
            if k >= got {
                // Dohravani: historii prevodniku vytlacit tichem.
                if draining && *flush > 0 {
                    *flush -= 1;
                    return Some([0, 0]);
                }
                return None;
            }
            let f = frame_to_stereo(&fmt, &scratch[k * fb..(k + 1) * fb]);
            k += 1;
            Some(f)
        });
        self.ring.consume(k as u32);
        self.consumed += k as u64;
        if made > 0 {
            mixer.add(&self.frames[..made], volume_gain(self.volume));
            self.last_data = pos + made as u64;
        }
        if made < n {
            if self.state == State::Running && self.fed {
                self.underruns += 1;
                self.ring.add_underrun();
            }
            self.fed = false;
        } else {
            self.fed = true;
        }
        if k > 0 {
            let _ = self.event.signal();
        }
    }

    /// Po mixu: odhad prehranych snimku a konec `DRAIN` (`true` = stream
    /// prave dohral a ma se odpovedet).
    pub fn after_mix(&mut self, hw: u64, written: u64, dev_rate: u32) -> bool {
        if self.state == State::Stopped {
            return false; // pozice zastaveneho streamu uz se nemeni
        }
        // Smichane, ale jeste nezaznele snimky zarizeni -> snimky klienta.
        let pending_dev = written.saturating_sub(hw);
        let pending = pending_dev * self.format.rate as u64 / dev_rate as u64;
        self.ring.set_played(self.consumed.saturating_sub(pending));
        if self.state == State::Draining && self.ring.readable() == 0 && self.flush == 0 && hw >= self.last_data {
            self.state = State::Stopped;
            self.ring.set_played(self.consumed);
            return true;
        }
        false
    }

    pub fn active(&self) -> bool {
        self.state != State::Stopped
    }
}
